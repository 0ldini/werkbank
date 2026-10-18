//! Automatisches Entpacken nach einem fertigen Paket (läuft in einem eigenen Thread).
//!
//! ZIP entpackt Ladewerk selbst (zip-Crate, auch mit Passwort: ZipCrypto und AES).
//! 7z, RAR und geteilte ZIP-Archive gehen über ein installiertes 7-Zip (`7z`/`7zz`).
//! Jeder Versuch entpackt zuerst in einen versteckten Arbeitsordner; erst bei Erfolg werden
//! die Dateien an ihren Platz verschoben – ohne bestehende Dateien zu überschreiben.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::links;
use crate::net::lock;
use crate::plan::{self, ArcKind};
use crate::sprache::{t, tf};

#[derive(Default)]
pub struct UnpackRun {
    pub cancel: AtomicBool,
    pub status: Mutex<String>,
    /// Ok(Zusammenfassung) oder Err(Fehlermeldung).
    pub result: Mutex<Option<Result<String, String>>>,
    pub finished: AtomicBool,
}

pub struct UnpackJob {
    pub dir: PathBuf,
    /// Dateinamen der fertigen Downloads des Pakets.
    pub names: Vec<String>,
    /// Passwörter: zuerst die des Pakets, dann die allgemeine Liste.
    pub passwords: Vec<String>,
    pub sub_folder: bool,
    pub delete_archives: bool,
}

enum Fail {
    WrongPassword,
    Cancelled,
    Other(String),
}

/// Sucht 7-Zip: neben dem Programm, an den üblichen Orten und auf dem PATH.
pub fn find_7z() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["7z.exe", "7zz.exe"] } else { &["7zz", "7z", "7za"] };
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        dirs.push(d);
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(p) = std::env::var_os(var) {
                dirs.push(PathBuf::from(p).join("7-Zip"));
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

pub fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Entpackt alle Archive des Pakets. Ok = Zusammenfassung.
pub fn run(job: &UnpackJob, r: &UnpackRun) -> Result<String, String> {
    let plan = plan::archives_to_extract(&job.names);
    if plan.is_empty() {
        return Ok(t("Keine Archive im Paket").to_string());
    }
    let mut candidates: Vec<String> = vec![String::new()];
    for p in &job.passwords {
        if !p.is_empty() && !candidates.contains(p) {
            candidates.push(p.clone());
        }
    }
    let seven = find_7z();
    let mut ok = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for (name, kind, stem) in &plan {
        if r.cancel.load(Ordering::Relaxed) {
            return Err(t("Abgebrochen").to_string());
        }
        *lock(&r.status) = tf("Entpacke {name} …", &[("name", name)]);
        let archive = job.dir.join(name);
        match extract_one(job, &archive, *kind, stem, &candidates, seven.as_deref(), r) {
            Ok(()) => {
                ok += 1;
                if job.delete_archives {
                    for part in plan::parts_of(stem, *kind, &job.names) {
                        let _ = fs::remove_file(job.dir.join(part));
                    }
                }
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if errors.is_empty() {
        Ok(tf("{n} Archiv(e) entpackt", &[("n", &ok)]))
    } else {
        Err(errors.join("; "))
    }
}

fn extract_one(job: &UnpackJob, archive: &Path, kind: ArcKind, stem: &str, candidates: &[String], seven: Option<&Path>, r: &UnpackRun) -> Result<(), String> {
    if !archive.is_file() {
        return Err(t("Datei fehlt").to_string());
    }
    if !kind.builtin() && seven.is_none() {
        return Err(t("7-Zip nicht gefunden – für 7z, RAR und geteilte ZIP bitte 7-Zip installieren").to_string());
    }
    let tmp_name = links::unique_name(".ladewerk-entpacken", |n| job.dir.join(n).exists());
    let tmp = job.dir.join(tmp_name);
    let result = (|| {
        for pw in candidates {
            if tmp.exists() {
                fs::remove_dir_all(&tmp).map_err(|e| e.to_string())?;
            }
            fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
            let res = match (kind, seven) {
                (ArcKind::Zip, _) => unzip(archive, &tmp, pw, &r.cancel),
                (_, Some(exe)) => seven_zip(exe, archive, &tmp, pw, &r.cancel),
                (_, None) => Err(Fail::Other(String::new())),
            };
            match res {
                Ok(()) => return place(&tmp, &job.dir, stem, job.sub_folder),
                Err(Fail::WrongPassword) => continue,
                Err(Fail::Cancelled) => return Err(t("Abgebrochen").to_string()),
                Err(Fail::Other(e)) => return Err(e),
            }
        }
        Err(if candidates.len() > 1 {
            t("Kein passendes Passwort in der Liste").to_string()
        } else {
            t("Archiv ist verschlüsselt – Passwort fehlt").to_string()
        })
    })();
    if tmp.exists() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

/// Verschiebt das Entpackte an seinen Platz: bei mehreren Einträgen und `sub_folder` in einen
/// eigenen Ordner «stem», sonst direkt in den Paketordner. Namenskonflikte werden mit « (2)»
/// gelöst – bestehende Dateien werden nie überschrieben.
fn place(tmp: &Path, dir: &Path, stem: &str, sub_folder: bool) -> Result<(), String> {
    let entries: Vec<PathBuf> = fs::read_dir(tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    // Nur ein Eintrag (z. B. ein Ordner): kein zusätzlicher Ordner nötig.
    if sub_folder && entries.len() > 1 {
        let wanted = links::sanitize_file_name(stem).unwrap_or_else(|| "entpackt".to_string());
        let target = dir.join(links::unique_name(&wanted, |n| dir.join(n).exists()));
        fs::rename(tmp, &target).map_err(|e| e.to_string())?;
        return Ok(());
    }
    for e in entries {
        let Some(n) = e.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let target = dir.join(links::unique_name(&n, |c| dir.join(c).exists()));
        fs::rename(&e, &target).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn looks_like_password_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("password") || m.contains("decrypt")
}

fn unzip(archive: &Path, dest: &Path, pw: &str, cancel: &AtomicBool) -> Result<(), Fail> {
    let file = File::open(archive).map_err(|e| Fail::Other(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Fail::Other(e.to_string()))?;
    for i in 0..zip.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Fail::Cancelled);
        }
        let entry = if pw.is_empty() { zip.by_index(i) } else { zip.by_index_decrypt(i, pw.as_bytes()) };
        let mut entry = match entry {
            Ok(e) => e,
            Err(e) => {
                let msg = e.to_string();
                return Err(if looks_like_password_error(&msg) { Fail::WrongPassword } else { Fail::Other(msg) });
            }
        };
        // enclosed_name weist Pfade ab, die aus dem Zielordner herausführen würden.
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| Fail::Other(e.to_string()))?;
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p).map_err(|e| Fail::Other(e.to_string()))?;
        }
        if out.exists() {
            continue; // doppelter Eintrag im Archiv: den ersten behalten
        }
        let mut f = File::create(&out).map_err(|e| Fail::Other(e.to_string()))?;
        let mut buf = vec![0u8; 128 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Fail::Cancelled);
            }
            let n = match entry.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n.min(buf.len()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    // Mit falschem ZipCrypto-Passwort fällt es oft erst hier auf (Prüfsumme).
                    let msg = e.to_string();
                    return Err(if !pw.is_empty() || looks_like_password_error(&msg) { Fail::WrongPassword } else { Fail::Other(msg) });
                }
            };
            f.write_all(&buf[..n]).map_err(|e| Fail::Other(e.to_string()))?;
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            // Nur Lese-/Schreib-/Ausführrechte übernehmen, keine Sonderbits.
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o777));
        }
    }
    Ok(())
}

fn seven_zip(exe: &Path, archive: &Path, dest: &Path, pw: &str, cancel: &AtomicBool) -> Result<(), Fail> {
    let mut out_arg = OsString::from("-o");
    out_arg.push(dest);
    let mut cmd = Command::new(exe);
    // -p immer angeben, damit 7-Zip nie auf eine Eingabe wartet.
    cmd.arg("x").arg("-y").arg("-bd").arg("-aou").arg(format!("-p{pw}")).arg(out_arg).arg("--").arg(archive);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| Fail::Other(tf("7-Zip lässt sich nicht starten: {e}", &[("e", &e)])))?;
    // Fehlerausgabe in einem eigenen Thread lesen, damit 7-Zip nie blockiert.
    let reader = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            let _ = s.by_ref().take(1 << 20).read_to_end(&mut v);
            let _ = std::io::copy(&mut s, &mut std::io::sink());
            String::from_utf8_lossy(&v).into_owned()
        })
    });
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Fail::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(Fail::Other(e.to_string())),
        }
    };
    let err = reader.and_then(|h| h.join().ok()).unwrap_or_default();
    match status.code() {
        Some(0) | Some(1) => Ok(()),
        _ if looks_like_password_error(&err) => Err(Fail::WrongPassword),
        code => {
            let line = err.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("").to_string();
            let code = code.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string());
            Err(Fail::Other(tf("7-Zip meldet Fehler {code}: {line}", &[("code", &code), ("line", &line)])))
        }
    }
}
