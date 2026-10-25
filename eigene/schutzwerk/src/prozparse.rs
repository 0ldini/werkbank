//! Einstufung laufender Programme (defensiv, nur Lesen): Wo liegt die Programmdatei
//! (Temp, AppData, Downloads …)? Ahmt der Name einen Windows-Prozess nach? Bekannte
//! Miner- und Fernzugriffs-Programme, GPU-Last aus der Leistungsindikator-Ausgabe,
//! Signatur-Ausgaben von PowerShell/codesign und die Dauerlast-Überwachung.
//! Nur Standardbibliothek – mit `rustc --edition 2021 --test src/prozparse.rs` testbar.

#![allow(dead_code)]

use std::collections::HashMap;

/// Wo eine Programmdatei liegt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ort {
    /// Programme, Systemordner oder sonst unauffällig.
    Normal,
    /// C:\Windows\System32 bzw. SysWOW64 (oder C:\Windows für explorer.exe).
    System,
    /// Temporärer Ordner (%TEMP%, /tmp, /var/tmp, /dev/shm).
    Temp,
    /// %APPDATA% bzw. %LOCALAPPDATA% (viele echte Programme, aber auch Schadsoftware).
    AppData,
    /// Downloads-Ordner.
    Downloads,
    /// C:\Users\Public oder C:\ProgramData direkt (ohne Unterordner eines Herstellers).
    Oeffentlich,
    /// Papierkorb.
    Papierkorb,
    /// Linux: Programmdatei wurde nach dem Start gelöscht.
    Geloescht,
}

impl Ort {
    /// Gilt als ungewöhnlich (vom Benutzer beschreibbar, typisch für Schadsoftware).
    pub fn ungewoehnlich(self) -> bool {
        !matches!(self, Ort::Normal | Ort::System)
    }
    /// Deutlich auffällig (nicht nur «viele Programme liegen hier»).
    pub fn stark(self) -> bool {
        matches!(self, Ort::Temp | Ort::Downloads | Ort::Oeffentlich | Ort::Papierkorb | Ort::Geloescht)
    }
}

fn norm(p: &str) -> String {
    p.replace('/', "\\").to_lowercase()
}

/// Ordnet einen Pfad ein. `windows` = Windows-Pfad (sonst Unix).
pub fn ort_von(path: &str, windows: bool) -> Ort {
    if path.is_empty() {
        return Ort::Normal;
    }
    if windows {
        let p = norm(path);
        if p.contains("\\$recycle.bin\\") {
            return Ort::Papierkorb;
        }
        if p.starts_with("c:\\windows\\system32\\") || p.starts_with("c:\\windows\\syswow64\\") {
            return Ort::System;
        }
        if p.contains("\\appdata\\local\\temp\\") || p.starts_with("c:\\windows\\temp\\") || p.starts_with("c:\\temp\\") {
            return Ort::Temp;
        }
        if p.contains("\\downloads\\") {
            return Ort::Downloads;
        }
        if p.starts_with("c:\\users\\public\\") {
            return Ort::Oeffentlich;
        }
        // Direkt in C:\ProgramData (ohne Herstellerordner) ist ungewöhnlich.
        if let Some(rest) = p.strip_prefix("c:\\programdata\\") {
            if !rest.contains('\\') {
                return Ort::Oeffentlich;
            }
        }
        if p.contains("\\appdata\\roaming\\") || p.contains("\\appdata\\local\\") {
            return Ort::AppData;
        }
        Ort::Normal
    } else {
        if path.ends_with(" (deleted)") {
            return Ort::Geloescht;
        }
        if path.starts_with("/tmp/") || path.starts_with("/var/tmp/") || path.starts_with("/dev/shm/") || path.starts_with("/private/tmp/") || path.starts_with("/private/var/tmp/") {
            return Ort::Temp;
        }
        if path.contains("/Downloads/") {
            return Ort::Downloads;
        }
        if path.contains("/.Trash/") || path.contains("/.local/share/Trash/") {
            return Ort::Papierkorb;
        }
        Ort::Normal
    }
}

/// Windows-Systemprozesse und wo sie hingehören.
const SYSTEM_NAMEN: &[&str] = &[
    "svchost.exe",
    "csrss.exe",
    "lsass.exe",
    "smss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "spoolsv.exe",
    "taskhostw.exe",
    "dwm.exe",
    "conhost.exe",
    "rundll32.exe",
    "dllhost.exe",
    "sihost.exe",
    "ctfmon.exe",
    "runtimebroker.exe",
    "searchindexer.exe",
    "wuauclt.exe",
    "lsm.exe",
    "fontdrvhost.exe",
    "explorer.exe",
];

/// Levenshtein-Abstand (klein, für kurze Namen).
pub fn abstand(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let kosten = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + kosten);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Ahmt der Name einen Windows-Systemprozess nach? `path` = bekannte Programmdatei.
/// Liefert den nachgeahmten Namen.
pub fn imitation(name: &str, path: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    let p = norm(path);
    for sys in SYSTEM_NAMEN {
        if n == *sys {
            if p.is_empty() {
                return None; // Pfad unbekannt (fehlende Rechte) – nichts behaupten
            }
            let ok = if *sys == "explorer.exe" {
                p == "c:\\windows\\explorer.exe" || p.starts_with("c:\\windows\\syswow64\\")
            } else {
                p.starts_with("c:\\windows\\system32\\") || p.starts_with("c:\\windows\\syswow64\\") || p.starts_with("c:\\windows\\winsxs\\")
            };
            return if ok { None } else { Some(sys) };
        }
    }
    // Ähnliche Namen (svch0st.exe, scvhost.exe …) – nur bei längeren Namen, sonst zu viele Zufallstreffer.
    let stem = n.strip_suffix(".exe")?;
    if stem.chars().count() < 5 {
        return None;
    }
    for sys in SYSTEM_NAMEN {
        let s = sys.trim_end_matches(".exe");
        if s.chars().count() >= 5 && abstand(stem, s) == 1 {
            return Some(sys);
        }
    }
    None
}

/// Bekannte Krypto-Miner (Namen ohne .exe, klein).
const MINER: &[&str] = &[
    "xmrig", "xmr-stak", "xmrstak", "minerd", "cpuminer", "nbminer", "t-rex", "phoenixminer", "lolminer", "ethminer", "cgminer", "bfgminer", "claymore",
    "gminer", "teamredminer", "srbminer", "srbminer-multi", "nanominer", "excavator", "ccminer", "nheqminer", "bzminer", "rigel", "wildrig", "xmrig-notls",
];

pub fn ist_miner(name: &str) -> bool {
    let n = name.to_lowercase();
    let n = n.strip_suffix(".exe").unwrap_or(&n);
    MINER.contains(&n)
}

/// Fernzugriffs-Programme: (Prozessname klein ohne .exe, Produkt).
const FERN: &[(&str, &str)] = &[
    ("teamviewer", "TeamViewer"),
    ("teamviewer_service", "TeamViewer"),
    ("tv_w32", "TeamViewer"),
    ("tv_x64", "TeamViewer"),
    ("anydesk", "AnyDesk"),
    ("rustdesk", "RustDesk"),
    ("winvnc", "VNC (UltraVNC)"),
    ("uvnc_service", "VNC (UltraVNC)"),
    ("tvnserver", "VNC (TightVNC)"),
    ("vncserver", "VNC (RealVNC)"),
    ("vncserver-x11", "VNC (RealVNC)"),
    ("vncserverui", "VNC (RealVNC)"),
    ("x11vnc", "VNC (x11vnc)"),
    ("vino-server", "VNC (Vino)"),
    ("krfb", "VNC (KDE Krfb)"),
    ("srservice", "Splashtop"),
    ("srserver", "Splashtop"),
    ("strwinclt", "Splashtop"),
    ("logmein", "LogMeIn"),
    ("lmiguardiansvc", "LogMeIn"),
    ("screenconnect.clientservice", "ScreenConnect"),
    ("screenconnect.windowsclient", "ScreenConnect"),
    ("connectwisecontrol.client", "ScreenConnect"),
    ("ateraagent", "Atera"),
    ("rpcservice", "RemotePC"),
    ("supremo", "Supremo"),
    ("supremoservice", "Supremo"),
    ("aa_v3", "Ammyy Admin"),
    ("ammyy_admin", "Ammyy Admin"),
    ("rserver3", "Radmin"),
    ("parsecd", "Parsec"),
    ("remoting_host", "Chrome Remote Desktop"),
    ("nxserver", "NoMachine"),
    ("nxserver.bin", "NoMachine"),
    ("nxd", "NoMachine"),
    ("dwagent", "DWService"),
    ("dwagsvc", "DWService"),
    ("za_connect", "Zoho Assist"),
    ("zaservice", "Zoho Assist"),
    ("g2ax_host", "GoTo Assist"),
    ("quickassist", "Quick Assist (Windows)"),
    ("meshagent", "MeshCentral"),
    ("tacticalrmm", "Tactical RMM"),
    ("screensharingd", "macOS-Bildschirmfreigabe"),
    ("ardagent", "macOS-Entfernte Verwaltung"),
];

pub fn fernzugriff(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    let n = n.strip_suffix(".exe").unwrap_or(&n);
    FERN.iter().find(|(p, _)| *p == n).map(|(_, prod)| *prod)
}

/// Fernzugriffs-Dienste (Windows-Dienstname klein, Produkt) – erkennt Installiertes, das nicht läuft.
pub fn fern_dienst(dienst: &str) -> Option<&'static str> {
    let d = dienst.to_lowercase();
    const D: &[(&str, &str)] = &[
        ("teamviewer", "TeamViewer"),
        ("anydesk", "AnyDesk"),
        ("rustdesk", "RustDesk"),
        ("uvnc_service", "VNC (UltraVNC)"),
        ("tvnserver", "VNC (TightVNC)"),
        ("vncserver", "VNC (RealVNC)"),
        ("splashtopremoteservice", "Splashtop"),
        ("lmiguardiansvc", "LogMeIn"),
        ("logmein", "LogMeIn"),
        ("screenconnect", "ScreenConnect"),
        ("ateraagent", "Atera"),
        ("rpcservice", "RemotePC"),
        ("supremoservice", "Supremo"),
        ("rserver3", "Radmin"),
        ("parsec", "Parsec"),
        ("chromoting", "Chrome Remote Desktop"),
        ("nomachine", "NoMachine"),
        ("nxservice", "NoMachine"),
        ("dwagent", "DWService"),
        ("zohoassist", "Zoho Assist"),
        ("mesh agent", "MeshCentral"),
    ];
    D.iter().find(|(p, _)| d.starts_with(p)).map(|(_, prod)| *prod)
}

/// GPU-Last je Prozess aus Zeilen «pid_1234_luid_…_engtype_3D<TAB>12» (Name, Prozent).
/// Summiert alle Engines eines Prozesses, höchstens 100.
pub fn parse_gpu(out: &str) -> HashMap<u32, f32> {
    let mut m: HashMap<u32, f32> = HashMap::new();
    for line in out.lines() {
        let Some((name, wert)) = line.trim().split_once('\t') else { continue };
        let Some(rest) = name.trim().strip_prefix("pid_") else { continue };
        let pid: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let (Ok(pid), Ok(w)) = (pid.parse::<u32>(), wert.trim().replace(',', ".").parse::<f32>()) else { continue };
        if !w.is_finite() || w <= 0.0 {
            continue;
        }
        let e = m.entry(pid).or_insert(0.0);
        *e = (*e + w).min(100.0);
    }
    m
}

/// Signatur-Status einer Programmdatei.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigStatus {
    Gueltig,
    Unsigniert,
    /// Signatur vorhanden, aber ungültig oder nicht vertrauenswürdig.
    Ungueltig,
    Unbekannt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sig {
    pub status: SigStatus,
    /// Herausgeber (O= bzw. CN= aus dem Zertifikat), leer wenn unbekannt.
    pub von: String,
}

/// Feld (O, CN …) aus einem Zertifikats-Betreff wie `CN=X, O="Firma, Inc.", C=US`.
pub fn betreff_feld(subject: &str, feld: &str) -> Option<String> {
    let mut i = 0;
    let b: Vec<char> = subject.chars().collect();
    while i < b.len() {
        while i < b.len() && (b[i] == ' ' || b[i] == ',') {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i] != '=' && b[i] != ',' {
            i += 1;
        }
        let key: String = b.get(start..i).map(|s| s.iter().collect()).unwrap_or_default();
        if i >= b.len() || b[i] != '=' {
            continue;
        }
        i += 1;
        let mut val = String::new();
        if i < b.len() && b[i] == '"' {
            i += 1;
            while i < b.len() && b[i] != '"' {
                val.push(b[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < b.len() && b[i] != ',' {
                val.push(b[i]);
                i += 1;
            }
        }
        if key.trim().eq_ignore_ascii_case(feld) {
            return Some(val.trim().to_string());
        }
    }
    None
}

/// Zeilen «Pfad<TAB>Status<TAB>Betreff» von `Get-AuthenticodeSignature`.
pub fn parse_authenticode(out: &str) -> Vec<(String, Sig)> {
    let mut v = Vec::new();
    for line in out.lines() {
        let c: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
        if c.len() < 2 || c[0].trim().is_empty() {
            continue;
        }
        let status = match c[1].trim() {
            "Valid" => SigStatus::Gueltig,
            "NotSigned" => SigStatus::Unsigniert,
            "HashMismatch" | "NotTrusted" => SigStatus::Ungueltig,
            _ => SigStatus::Unbekannt,
        };
        let subject = c.get(2).map(|s| s.trim()).unwrap_or("");
        let von = betreff_feld(subject, "O").or_else(|| betreff_feld(subject, "CN")).unwrap_or_default();
        v.push((c[0].trim().to_string(), Sig { status, von }));
    }
    v
}

/// Ausgabe von `codesign -dv --verbose=2` (macOS, steht in stderr).
pub fn parse_codesign(stderr: &str, erfolg: bool) -> Sig {
    if stderr.contains("not signed at all") {
        return Sig { status: SigStatus::Unsigniert, von: String::new() };
    }
    if !erfolg {
        return Sig { status: SigStatus::Unbekannt, von: String::new() };
    }
    let von = stderr
        .lines()
        .find_map(|l| l.trim().strip_prefix("Authority="))
        .map(|a| a.strip_prefix("Developer ID Application: ").unwrap_or(a).to_string())
        .unwrap_or_default();
    if von.is_empty() && stderr.contains("Signature=adhoc") {
        // Ad-hoc signiert: kein Herausgeber, gilt hier als unsigniert.
        return Sig { status: SigStatus::Unsigniert, von: String::new() };
    }
    Sig { status: SigStatus::Gueltig, von }
}

/// Leerlaufzeit aus `ioreg -c IOHIDSystem` (macOS, «"HIDIdleTime" = Nanosekunden»).
pub fn parse_ioreg_idle(out: &str) -> Option<u64> {
    for line in out.lines() {
        if let Some(pos) = line.find("\"HIDIdleTime\" = ") {
            let rest = line.get(pos + 16..)?;
            let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            return n.parse::<u64>().ok().map(|ns| ns / 1_000_000_000);
        }
    }
    None
}

/// Merkt sich, seit wann ein Prozess über der Schwelle liegt (Dauerlast-Wache).
#[derive(Default)]
pub struct Dauerlast {
    /// pid -> (Name, seit Sekunde, letzte Last)
    seit: HashMap<u32, (String, u64, f32)>,
}

impl Dauerlast {
    /// Neue Messung zur Zeit `jetzt` (Sekunden). Liefert alle Prozesse, die seit mindestens
    /// `dauer` Sekunden über `schwelle` liegen: (pid, Name, Sekunden darüber, aktuelle Last).
    pub fn messen(&mut self, jetzt: u64, proben: &[(u32, String, f32)], schwelle: f32, dauer: u64) -> Vec<(u32, String, u64, f32)> {
        let mut neu = HashMap::new();
        for (pid, name, last) in proben {
            if *last >= schwelle {
                let start = self.seit.get(pid).filter(|(n, _, _)| n == name).map(|(_, s, _)| *s).unwrap_or(jetzt);
                neu.insert(*pid, (name.clone(), start, *last));
            }
        }
        self.seit = neu;
        let mut v: Vec<(u32, String, u64, f32)> = self
            .seit
            .iter()
            .filter(|(_, (_, s, _))| jetzt.saturating_sub(*s) >= dauer)
            .map(|(pid, (n, s, l))| (*pid, n.clone(), jetzt.saturating_sub(*s), *l))
            .collect();
        v.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orte_windows() {
        assert_eq!(ort_von(r"C:\Windows\System32\svchost.exe", true), Ort::System);
        assert_eq!(ort_von(r"C:\Users\Oli\AppData\Local\Temp\x1\run.exe", true), Ort::Temp);
        assert_eq!(ort_von(r"C:\Users\Oli\AppData\Roaming\upd\upd.exe", true), Ort::AppData);
        assert_eq!(ort_von(r"C:\Users\Oli\Downloads\setup.exe", true), Ort::Downloads);
        assert_eq!(ort_von(r"C:\Users\Public\svc.exe", true), Ort::Oeffentlich);
        assert_eq!(ort_von(r"C:\ProgramData\host.exe", true), Ort::Oeffentlich);
        assert_eq!(ort_von(r"C:\ProgramData\Microsoft\Windows Defender\x.exe", true), Ort::Normal);
        assert_eq!(ort_von(r"C:\Program Files\Mozilla Firefox\firefox.exe", true), Ort::Normal);
        assert_eq!(ort_von(r"C:\$Recycle.Bin\S-1-5\$R1.exe", true), Ort::Papierkorb);
        assert!(Ort::Temp.stark() && !Ort::AppData.stark() && Ort::AppData.ungewoehnlich());
    }

    #[test]
    fn orte_unix() {
        assert_eq!(ort_von("/tmp/.x/kworker", false), Ort::Temp);
        assert_eq!(ort_von("/dev/shm/m", false), Ort::Temp);
        assert_eq!(ort_von("/home/oli/Downloads/tool", false), Ort::Downloads);
        assert_eq!(ort_von("/usr/bin/bash", false), Ort::Normal);
        assert_eq!(ort_von("/home/oli/.cache/x (deleted)", false), Ort::Geloescht);
    }

    #[test]
    fn nachahmer() {
        assert_eq!(imitation("svchost.exe", r"C:\Windows\System32\svchost.exe"), None);
        assert_eq!(imitation("svchost.exe", r"C:\Users\Oli\AppData\Roaming\svchost.exe"), Some("svchost.exe"));
        assert_eq!(imitation("svchost.exe", ""), None);
        assert_eq!(imitation("svch0st.exe", r"C:\x\svch0st.exe"), Some("svchost.exe"));
        assert_eq!(imitation("svhost.exe", r"C:\x\svhost.exe"), Some("svchost.exe"));
        assert_eq!(imitation("explorer.exe", r"C:\Windows\explorer.exe"), None);
        assert_eq!(imitation("explorer.exe", r"C:\Users\Public\explorer.exe"), Some("explorer.exe"));
        assert_eq!(imitation("firefox.exe", r"C:\Program Files\firefox.exe"), None);
        assert_eq!(imitation("dwn.exe", r"C:\x\dwn.exe"), None);
        assert_eq!(abstand("svchost", "svch0st"), 1);
        assert_eq!(abstand("", "abc"), 3);
    }

    #[test]
    fn miner_und_fern() {
        assert!(ist_miner("xmrig.exe"));
        assert!(ist_miner("XMRig"));
        assert!(!ist_miner("chrome.exe"));
        assert_eq!(fernzugriff("AnyDesk.exe"), Some("AnyDesk"));
        assert_eq!(fernzugriff("TeamViewer_Service.exe"), Some("TeamViewer"));
        assert_eq!(fernzugriff("notepad.exe"), None);
        assert_eq!(fern_dienst("TeamViewer"), Some("TeamViewer"));
        assert_eq!(fern_dienst("ScreenConnect Client (abc123)"), Some("ScreenConnect"));
        assert_eq!(fern_dienst("Spooler"), None);
    }

    #[test]
    fn gpu() {
        let out = "pid_1234_luid_0x00000000_0x0000D1A5_phys_0_eng_0_engtype_3D\t12\r\n\
                   pid_1234_luid_0x00000000_0x0000D1A5_phys_0_eng_1_engtype_Copy\t3\n\
                   pid_88_luid_0x0_phys_0_eng_0_engtype_VideoDecode\t0\n\
                   _Total\t40\nkaputt\n";
        let m = parse_gpu(out);
        assert_eq!(m.get(&1234).copied(), Some(15.0));
        assert!(!m.contains_key(&88));
    }

    #[test]
    fn signaturen() {
        let out = "C:\\Program Files\\A\\a.exe\tValid\tCN=\"Firma, Inc.\", O=\"Firma, Inc.\", L=Zürich, C=CH\r\n\
                   C:\\Users\\x\\AppData\\Roaming\\b.exe\tNotSigned\t\n\
                   C:\\c.exe\tHashMismatch\tCN=Evil\n";
        let v = parse_authenticode(out);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].1, Sig { status: SigStatus::Gueltig, von: "Firma, Inc.".into() });
        assert_eq!(v[1].1.status, SigStatus::Unsigniert);
        assert_eq!(v[2].1, Sig { status: SigStatus::Ungueltig, von: "Evil".into() });
        assert_eq!(betreff_feld("CN=Microsoft Windows, O=Microsoft Corporation, C=US", "O").as_deref(), Some("Microsoft Corporation"));
    }

    #[test]
    fn codesign() {
        let ok = "Executable=/Applications/Firefox.app/Contents/MacOS/firefox\nAuthority=Developer ID Application: Mozilla Corporation (43AQ936H96)\nAuthority=Developer ID Certification Authority\n";
        assert_eq!(parse_codesign(ok, true), Sig { status: SigStatus::Gueltig, von: "Mozilla Corporation (43AQ936H96)".into() });
        assert_eq!(parse_codesign("/tmp/x: code object is not signed at all\n", false).status, SigStatus::Unsigniert);
        assert_eq!(parse_codesign("Signature=adhoc\n", true).status, SigStatus::Unsigniert);
    }

    #[test]
    fn ioreg() {
        let out = "    | |   \"HIDIdleTime\" = 125000000000\n";
        assert_eq!(parse_ioreg_idle(out), Some(125));
        assert_eq!(parse_ioreg_idle("nichts"), None);
    }

    #[test]
    fn dauerlast() {
        let mut d = Dauerlast::default();
        let p = |l: f32| vec![(7u32, "miner.exe".to_string(), l), (8, "edit.exe".to_string(), 5.0)];
        assert!(d.messen(0, &p(95.0), 80.0, 120).is_empty());
        assert!(d.messen(60, &p(90.0), 80.0, 120).is_empty());
        let a = d.messen(130, &p(91.0), 80.0, 120);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].0, 7);
        assert_eq!(a[0].2, 130);
        // Fällt kurz darunter: Zählung beginnt neu.
        assert!(d.messen(140, &p(10.0), 80.0, 120).is_empty());
        assert!(d.messen(150, &p(99.0), 80.0, 120).is_empty());
    }
}
