//! Ende-zu-Ende-Test gegen echte Server: FTP, FTPS (selbst signiertes Zertifikat) und SFTP
//! (OpenSSH). Läuft nur, wenn die Umgebungsvariablen gesetzt sind – in der Pipeline
//! (.github/workflows/ftpwerk-test.yml) startet der Linux-Lauf die Server vorher. Ohne sie wird
//! jeder Test mit einer Meldung übersprungen (z. B. unter Windows oder auf dem eigenen Rechner).
//!
//! Geprüft wird der ganze Weg über den Motor (xfer.rs) wie in der Oberfläche: Rückfrage beim
//! unbekannten Zertifikat/Server-Schlüssel (Fingerabdruck stimmt mit openssl/ssh-keygen überein,
//! geänderter Fingerabdruck wird als «geändert» gemeldet), Ordner mit UTF-8-Namen und .htaccess
//! hochladen, auflisten, herunterladen und vergleichen, umbenennen, löschen sowie abgebrochene
//! Übertragungen fortsetzen (Herunterladen mit REST bzw. Versatz, Hochladen mit APPE bzw. Versatz).
//!
//! Umgebungsvariablen: FTPWERK_E2E_HOST, FTPWERK_E2E_USER, FTPWERK_E2E_PASS,
//! FTPWERK_E2E_FTP_PORT, FTPWERK_E2E_FTP_LIST_PORT (Server ohne MLSD), FTPWERK_E2E_FTPS_PORT, FTPWERK_E2E_FTPS_FP (SHA-256 «AB:CD:…»),
//! FTPWERK_E2E_SFTP_PORT, FTPWERK_E2E_SFTP_USER, FTPWERK_E2E_SFTP_PASS, FTPWERK_E2E_SFTP_KEY
//! (private Schlüsseldatei), FTPWERK_E2E_SFTP_FP («SHA256:…»).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::model::{rjoin, Proto, RErr, Site};
use crate::remote::{self, Tick};
use crate::store::{Overwrite, Trust};
use crate::xfer::{self, Cmd, Ev, Session};

const DIR: &str = "Grüezi Ördner";

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn site(proto: Proto, port_var: &str, user_var: &str, pass_var: &str) -> Option<Site> {
    let host = env("FTPWERK_E2E_HOST")?;
    let port: u16 = env(port_var)?.parse().ok()?;
    Some(Site { name: format!("e2e {}", proto.code()), proto, host, port, user: env(user_var)?, password: env(pass_var).unwrap_or_default(), ..Default::default() })
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ftpwerk-e2e-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("Testordner");
    d
}

/// Inhalt, der sich nicht wiederholt (Fehler beim Fortsetzen fallen so sicher auf).
fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as u8
        })
        .collect()
}

/// Lokaler Ordnerbaum: Umlaute, Japanisch, Gedankenstrich, .htaccess, leerer Ordner, grosse Datei.
fn make_tree(base: &Path) -> PathBuf {
    let root = base.join(DIR);
    std::fs::create_dir_all(root.join("Unterordner 日本語").join(".versteckt")).expect("Ordner");
    std::fs::create_dir_all(root.join("leer")).expect("Ordner");
    std::fs::write(root.join(".htaccess"), "Options -Indexes\nRewriteEngine On\n").expect("Datei");
    std::fs::write(root.join("Bärndütsch – ü.txt"), "Grüessech mitenand ✓\n").expect("Datei");
    std::fs::write(root.join("Unterordner 日本語").join("tief.txt"), "tief drin\n").expect("Datei");
    std::fs::write(root.join("Unterordner 日本語").join(".versteckt").join(".htaccess"), "Require all denied\n").expect("Datei");
    std::fs::write(root.join("gross.bin"), pattern(1_500_000, 7)).expect("Datei");
    std::fs::write(root.join("leere Datei.txt"), b"").expect("Datei");
    root
}

/// Alle Dateien und Ordner relativ zu `root`, sortiert, mit Inhalt (Ordner: None).
fn snapshot(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).expect("lesen").flatten() {
            let p = e.path();
            let rel = p.strip_prefix(root).expect("rel").to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                out.push((rel, None));
                todo.push(p);
            } else {
                out.push((rel, Some(std::fs::read(&p).expect("lesen"))));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn wait_ev(s: &Session, what: &str, secs: u64, mut want: impl FnMut(&Ev) -> bool) -> Ev {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let left = end.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "Zeitüberschreitung beim Warten auf: {what}");
        match s.rx.recv_timeout(left.min(Duration::from_millis(200))) {
            Ok(ev) => {
                match &ev {
                    Ev::Failed { what: w, err } => println!("  (Meldung) {w}: {err}"),
                    Ev::ConnectFailed(e) => panic!("{what}: Verbindung fehlgeschlagen: {e:?}"),
                    Ev::ListFailed { path, err } if !want(&ev) => panic!("{what}: Liste {path} fehlgeschlagen: {err}"),
                    _ => {}
                }
                if want(&ev) {
                    return ev;
                }
            }
            Err(_) => {
                for l in s.log_rx.try_iter() {
                    if l.starts_with('!') || l.contains("Fehler") {
                        println!("  log: {l}");
                    }
                }
            }
        }
    }
}

fn wait_queue(s: &Session, what: &str) {
    let end = Instant::now() + Duration::from_secs(180);
    loop {
        let (w, r, _d, f) = s.queue.counts();
        if w + r == 0 {
            let errs: Vec<String> = s.queue.snapshot().iter().filter(|j| j.status() == xfer::FAIL).map(|j| format!("{}: {}", j.name(), j.error_text())).collect();
            assert!(f == 0, "{what}: {f} Übertragungen fehlgeschlagen:\n{}", errs.join("\n"));
            return;
        }
        assert!(Instant::now() < end, "{what}: Warteschlange wird nicht fertig");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn list(s: &Session, path: &str) -> Vec<crate::model::Entry> {
    s.send(Cmd::List(path.to_string()));
    match wait_ev(s, &format!("Liste {path}"), 60, |e| matches!(e, Ev::Listing { path: p, .. } if p == path)) {
        Ev::Listing { entries, .. } => entries,
        _ => Vec::new(),
    }
}

/// Unbekannt → Rückfrage mit dem richtigen Fingerabdruck; anderer gespeichert → «geändert»;
/// bestätigt → Verbindung steht. Liefert den bestätigten Fingerabdruck.
fn check_trust(site: &Site, expected: &str) -> String {
    let first = remote::connect(site, &[], None);
    let (fp, changed) = match first {
        Err(RErr::UntrustedCert { fp, changed, .. }) => (fp, changed),
        Err(RErr::HostKey { fp, changed, .. }) => (fp, changed),
        Err(e) => panic!("{}: erwartet Rückfrage, bekam Fehler {e:?}", site.name),
        Ok(_) => panic!("{}: unbekannter Server wurde ohne Rückfrage angenommen!", site.name),
    };
    assert!(!changed, "{}: erstes Mal darf nicht «geändert» heissen", site.name);
    assert!(fp.eq_ignore_ascii_case(expected.trim()), "{}: Fingerabdruck {fp} ≠ erwartet {expected}", site.name);
    println!("{}: Rückfrage mit Fingerabdruck {fp} ✓", site.name);

    let other = if site.proto == Proto::Sftp { "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string() } else { "00:11:22:33".to_string() };
    match remote::connect(site, &[other], None) {
        Err(RErr::UntrustedCert { changed, .. }) | Err(RErr::HostKey { changed, .. }) => assert!(changed, "{}: geänderter Fingerabdruck nicht als «geändert» gemeldet", site.name),
        Err(e) => panic!("{}: erwartet Warnung «geändert», bekam {e:?}", site.name),
        Ok(_) => panic!("{}: geänderter Fingerabdruck wurde angenommen!", site.name),
    }
    println!("{}: geänderter Fingerabdruck → deutliche Warnung ✓", site.name);

    let mut c = remote::connect(site, std::slice::from_ref(&fp), None).unwrap_or_else(|e| panic!("{}: mit bestätigtem Fingerabdruck: {e:?}", site.name));
    let cwd = c.pwd().expect("pwd");
    c.close();
    println!("{}: nach Bestätigung verbunden, Startordner {cwd} ✓", site.name);
    fp
}

fn full_cycle(site: Site, trusted_fp: Option<String>) {
    let name = site.name.clone();
    let kind = if site.proto == Proto::Sftp { "ssh" } else { "tls" };
    let mut trust = Trust::default();
    if let Some(fp) = &trusted_fp {
        trust.trust(kind, &site.hostport(), fp);
    }
    let trust = Arc::new(Mutex::new(trust));
    let wake: xfer::Wake = Arc::new(|| {});
    let base_local = scratch(&format!("{}-{}", site.proto.code(), site.port));
    let src = make_tree(&base_local.join("quelle"));

    let s = Session::start(site.clone(), trust.clone(), 2, Overwrite::Overwrite, wake.clone());
    let cwd = match wait_ev(&s, "Verbinden", 60, |e| matches!(e, Ev::Connected { .. })) {
        Ev::Connected { cwd } => cwd,
        _ => unreachable!(),
    };
    let base = rjoin(&cwd, &format!("ftpwerk-e2e-{}", site.proto.code()));
    // Reste eines früheren Laufs entfernen
    s.send(Cmd::Delete(vec![(base.clone(), true)]));
    wait_ev(&s, "Aufräumen", 60, |e| matches!(e, Ev::Changed(_)));
    s.send(Cmd::Mkdir(base.clone()));
    wait_ev(&s, "Ordner anlegen", 60, |e| matches!(e, Ev::Changed(_)));

    // Hochladen: ganzer Ordner samt Unterordnern, .htaccess und UTF-8-Namen
    s.send(Cmd::PlanUpload { items: vec![src.clone()], remote_dir: base.clone() });
    let n = match wait_ev(&s, "Hochladen planen", 60, |e| matches!(e, Ev::Queued(_))) {
        Ev::Queued(n) => n,
        _ => 0,
    };
    assert_eq!(n, 6, "{name}: 6 Dateien erwartet");
    wait_queue(&s, "Hochladen");
    println!("{name}: {n} Dateien hochgeladen ✓");

    // Auflisten: versteckte Dateien und Namen unverändert
    let rdir = rjoin(&base, DIR);
    let entries = list(&s, &rdir);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    for want in [".htaccess", "Bärndütsch – ü.txt", "Unterordner 日本語", "leer", "gross.bin", "leere Datei.txt"] {
        assert!(names.contains(&want), "{name}: «{want}» fehlt in der Liste {names:?}");
    }
    let big = entries.iter().find(|e| e.name == "gross.bin").expect("gross.bin");
    assert_eq!(big.size, 1_500_000, "{name}: Grösse von gross.bin");
    assert!(entries.iter().any(|e| e.name == "Unterordner 日本語" && e.is_dir));
    let deep = list(&s, &rjoin(&rjoin(&rdir, "Unterordner 日本語"), ".versteckt"));
    assert!(deep.iter().any(|e| e.name == ".htaccess"), "{name}: .htaccess im versteckten Unterordner fehlt");
    println!("{name}: Liste mit .htaccess und UTF-8-Namen ✓ ({names:?})");

    // Herunterladen und Byte für Byte vergleichen
    let dl = base_local.join("ziel");
    std::fs::create_dir_all(&dl).expect("Ziel");
    s.send(Cmd::PlanDownload { items: vec![(rdir.clone(), true, 0)], local_dir: dl.clone() });
    wait_ev(&s, "Herunterladen planen", 60, |e| matches!(e, Ev::Queued(_)));
    wait_queue(&s, "Herunterladen");
    assert_eq!(snapshot(&dl.join(DIR)), snapshot(&src), "{name}: heruntergeladener Ordner weicht ab");
    println!("{name}: heruntergeladen, identisch mit dem Original ✓");

    // Umbenennen
    s.send(Cmd::Rename { from: rjoin(&rdir, "Bärndütsch – ü.txt"), to: rjoin(&rdir, "Umbenannt – ä€.txt") });
    wait_ev(&s, "Umbenennen", 60, |e| matches!(e, Ev::Changed(_)));
    let names: Vec<String> = list(&s, &rdir).into_iter().map(|e| e.name).collect();
    assert!(names.iter().any(|n| n == "Umbenannt – ä€.txt") && !names.iter().any(|n| n == "Bärndütsch – ü.txt"), "{name}: Umbenennen {names:?}");
    println!("{name}: umbenannt ✓");

    // Fortsetzen über die Schnittstelle direkt (wie ein Übertragungs-Thread)
    let fps: Vec<String> = trusted_fp.iter().cloned().collect();
    let mut c = remote::connect(&site, &fps, None).expect("Verbindung für Fortsetzen");
    let part = base_local.join("teil.bin");
    let full = pattern(1_500_000, 7);
    std::fs::write(&part, &full[..100_000]).expect("Teildatei");
    let mut start = (0u64, 0u64);
    let got = c
        .download(&rjoin(&rdir, "gross.bin"), &part, true, &mut |t| {
            if let Tick::Start { offset, total } = t {
                start = (offset, total);
            }
            true
        })
        .expect("Herunterladen fortsetzen");
    assert_eq!(start.0, 100_000, "{name}: Herunterladen hätte bei 100 000 weitermachen sollen");
    assert_eq!(got, 1_400_000, "{name}: nur der Rest sollte übertragen werden");
    assert!(std::fs::read(&part).expect("lesen") == full, "{name}: fortgesetzte Datei weicht ab");
    println!("{name}: Herunterladen fortgesetzt ab 100 000 Bytes ✓");

    // Hochladen abbrechen, dann fortsetzen
    let up_local = base_local.join("hoch.bin");
    let up_data = pattern(3_000_000, 11);
    std::fs::write(&up_local, &up_data).expect("Datei");
    let up_remote = rjoin(&base, "hoch.bin");
    let mut sent = 0u64;
    let r = c.upload(&up_local, &up_remote, false, &mut |t| {
        if let Tick::Bytes(n) = t {
            sent += n;
        }
        sent < 600_000
    });
    assert!(matches!(r, Err(RErr::Cancelled)), "{name}: Abbruch erwartet, bekam {r:?}");
    c.close();
    std::thread::sleep(Duration::from_millis(800));
    let mut c = remote::connect(&site, &fps, None).expect("neu verbinden");
    let have = c.size(&up_remote).ok().flatten().unwrap_or(0);
    let mut resumed_from = 0u64;
    let more = c
        .upload(&up_local, &up_remote, true, &mut |t| {
            if let Tick::Start { offset, .. } = t {
                resumed_from = offset;
            }
            true
        })
        .expect("Hochladen fortsetzen");
    assert_eq!(resumed_from, have, "{name}: Fortsetzen nicht ab der Grösse auf dem Server");
    assert_eq!(have + more, up_data.len() as u64, "{name}: Teil + Rest ≠ Gesamtgrösse");
    let check = base_local.join("hoch-zurueck.bin");
    c.download(&up_remote, &check, false, &mut |_| true).expect("zurückladen");
    assert!(std::fs::read(&check).expect("lesen") == up_data, "{name}: fortgesetzt hochgeladene Datei weicht ab");
    if have > 0 {
        println!("{name}: Hochladen nach Abbruch fortgesetzt ab {have} Bytes ✓");
    } else {
        println!("{name}: Server hat die abgebrochene Datei verworfen – komplett neu hochgeladen ✓");
    }
    c.close();

    // Löschen (rekursiv) und prüfen, dass nichts übrig ist
    s.send(Cmd::Delete(vec![(base.clone(), true)]));
    wait_ev(&s, "Löschen", 120, |e| matches!(e, Ev::Changed(_)));
    let left: Vec<String> = list(&s, &cwd).into_iter().map(|e| e.name).collect();
    assert!(!left.iter().any(|n| n.starts_with("ftpwerk-e2e-")), "{name}: Ordner nicht gelöscht: {left:?}");
    println!("{name}: rekursiv gelöscht ✓");

    s.stop();
    s.queue.stop.store(true, Ordering::Relaxed);
    s.join();
    let _ = std::fs::remove_dir_all(&base_local);
}

#[test]
fn e2e_ftp() {
    let Some(site) = site(Proto::Ftp, "FTPWERK_E2E_FTP_PORT", "FTPWERK_E2E_USER", "FTPWERK_E2E_PASS") else {
        println!("e2e_ftp übersprungen (keine Testserver-Angaben)");
        return;
    };
    full_cycle(site, None);
}

/// Server ohne MLSD (nur LIST, wie ältere Webhoster): versteckte Dateien kommen über «LIST -a».
#[test]
fn e2e_ftp_nur_list() {
    let Some(site) = site(Proto::Ftp, "FTPWERK_E2E_FTP_LIST_PORT", "FTPWERK_E2E_USER", "FTPWERK_E2E_PASS") else {
        println!("e2e_ftp_nur_list übersprungen (keine Testserver-Angaben)");
        return;
    };
    full_cycle(site, None);
}

#[test]
#[cfg(feature = "tls")]
fn e2e_ftps() {
    let (Some(site), Some(fp)) = (site(Proto::Ftps, "FTPWERK_E2E_FTPS_PORT", "FTPWERK_E2E_USER", "FTPWERK_E2E_PASS"), env("FTPWERK_E2E_FTPS_FP")) else {
        println!("e2e_ftps übersprungen (keine Testserver-Angaben)");
        return;
    };
    let fp = check_trust(&site, &fp);
    full_cycle(site, Some(fp));
}

#[test]
#[cfg(feature = "sftp")]
fn e2e_sftp() {
    let (Some(site), Some(fp)) = (site(Proto::Sftp, "FTPWERK_E2E_SFTP_PORT", "FTPWERK_E2E_SFTP_USER", "FTPWERK_E2E_SFTP_PASS"), env("FTPWERK_E2E_SFTP_FP")) else {
        println!("e2e_sftp übersprungen (keine Testserver-Angaben)");
        return;
    };
    let fp = check_trust(&site, &fp);
    // Anmeldung mit Schlüsseldatei statt Passwort
    if let Some(key) = env("FTPWERK_E2E_SFTP_KEY") {
        let ks = Site { key_file: key, password: String::new(), ..site.clone() };
        let mut c = remote::connect(&ks, std::slice::from_ref(&fp), None).unwrap_or_else(|e| panic!("SFTP mit Schlüsseldatei: {e:?}"));
        println!("SFTP: Anmeldung mit Schlüsseldatei ✓ ({})", c.pwd().unwrap_or_default());
        c.close();
        let wrong = Site { password: "falsch".into(), key_file: String::new(), ..site.clone() };
        assert!(remote::connect(&wrong, std::slice::from_ref(&fp), None).is_err(), "falsches Passwort angenommen");
        println!("SFTP: falsches Passwort abgelehnt ✓");
    }
    full_cycle(site, Some(fp));
}
