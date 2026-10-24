//! Systemschicht (defensiv): fragt lauschende Ports, Geräte im Netz und den
//! Schutzstatus ab und listet/​stellt Papierkorb-Dateien wieder her. Dazu gemeinsame
//! Helfer der übrigen Bereiche (Befehle ohne Konsolenfenster, PowerShell, Einstellungsordner,
//! Signaturen, Dateien zeigen). Ausschliesslich Lesen bzw. Status prüfen – keine Angriffs-
//! oder Auslese-Funktionen.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use crate::netparse::{self, Device, Listen};
use crate::prozparse::{self, Sig, SigStatus};
use crate::recycle::{self, Recycled};
use crate::sprache::{t, tf};

/// Befehl ohne Konsolenfenster (Windows: CREATE_NO_WINDOW).
pub fn cmd(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new(program);
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        c
    }
    #[cfg(not(windows))]
    {
        Command::new(program)
    }
}

pub fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = cmd(program).args(args).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Sperre, die auch nach einem abgestürzten Hintergrund-Thread weiter funktioniert.
pub fn sperre<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Base64 (für `powershell -EncodedCommand`).
fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for ch in bytes.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= ch.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Führt ein PowerShell-Skript aus (UTF-8-Ausgabe, ohne Profil, ohne Fenster).
/// Das Skript wird kodiert übergeben – keine Probleme mit Anführungszeichen.
pub fn ps(script: &str) -> Option<String> {
    let full = format!("$ProgressPreference='SilentlyContinue';[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;{script}");
    let utf16: Vec<u8> = full.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let enc = base64(&utf16);
    run("powershell", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &enc])
}

/// Text als PowerShell-Zeichenkette in einfachen Anführungszeichen.
pub fn ps_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''").replace('\u{2018}', "\u{2018}\u{2018}").replace('\u{2019}', "\u{2019}\u{2019}"))
}

/// Einstellungsordner von Schutzwerk (Grundlage, Sicherungen).
pub fn config_dir() -> Option<PathBuf> {
    let f = crate::sprache::werkbank_file()?;
    Some(f.parent()?.join("schutzwerk"))
}

/// Öffnet eine Adresse (https://…, ms-settings:…, windowsdefender:…) mit dem System.
pub fn oeffnen(ziel: &str) {
    #[cfg(windows)]
    let _ = cmd("explorer").arg(ziel).spawn();
    #[cfg(target_os = "macos")]
    let _ = cmd("open").arg(ziel).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = cmd("xdg-open").arg(ziel).spawn();
}

/// Startet ein Programm ohne zu warten (z. B. Systemeinstellungen).
pub fn starten(programm: &str) {
    let _ = Command::new(programm).spawn();
}

/// Zeigt eine Datei im Explorer/Finder bzw. öffnet ihren Ordner.
pub fn im_ordner_zeigen(pfad: &str) {
    if pfad.is_empty() {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let p = pfad.replace('"', "");
        let _ = Command::new("explorer").raw_arg(format!("/select,\"{p}\"")).spawn();
    }
    #[cfg(target_os = "macos")]
    let _ = cmd("open").arg("-R").arg(pfad).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = Path::new(pfad).parent().map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from(pfad));
        let _ = cmd("xdg-open").arg(dir).spawn();
    }
}

/// Sekunden seit 1970.
pub fn jetzt_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// FILETIME (100 ns seit 1601) -> Sekunden seit 1970 (0, wenn davor).
pub fn filetime_zu_unix(ft: u64) -> u64 {
    (ft / 10_000_000).saturating_sub(11_644_473_600)
}

/// FILETIME als lokales Datum «TT.MM.JJJJ hh:mm».
pub fn filetime_text(ft: u64) -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
        let utc = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        let mut lokal = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        // SAFETY: beide Zeiger zeigen auf gültige, lokale FILETIME-Werte.
        let ok = unsafe { FileTimeToLocalFileTime(&utc, &mut lokal) };
        if ok != 0 {
            return recycle::fmt_date((lokal.dwHighDateTime as u64) << 32 | lokal.dwLowDateTime as u64);
        }
    }
    format!("{} UTC", recycle::fmt_date(ft))
}

/// Prüft die Signatur von Programmdateien (Windows: Authenticode, macOS: codesign).
/// Nur für gewählte bzw. auffällige Dateien aufrufen – nicht für alle auf einmal.
pub fn signaturen(pfade: Vec<String>) -> Vec<(String, Sig)> {
    let mut pfade: Vec<String> = pfade.into_iter().filter(|p| !p.is_empty()).collect();
    pfade.sort();
    pfade.dedup();
    let mut v = Vec::new();
    if cfg!(windows) {
        for teil in pfade.chunks(25) {
            let liste: Vec<String> = teil.iter().map(|p| ps_str(p)).collect();
            let script = format!(
                "$ErrorActionPreference='SilentlyContinue';foreach($p in @({})){{$s=Get-AuthenticodeSignature -LiteralPath $p;$sub='';if($s.SignerCertificate){{$sub=$s.SignerCertificate.Subject}};\"$p`t$($s.Status)`t$sub\"}}",
                liste.join(",")
            );
            let out = ps(&script).unwrap_or_default();
            v.extend(prozparse::parse_authenticode(&out));
        }
        // Was PowerShell nicht meldete, bleibt unbekannt.
        for p in &pfade {
            if !v.iter().any(|(q, _)| q.eq_ignore_ascii_case(p)) {
                v.push((p.clone(), Sig { status: SigStatus::Unbekannt, von: String::new() }));
            }
        }
    } else if cfg!(target_os = "macos") {
        for p in pfade {
            let sig = match cmd("codesign").args(["-dv", "--verbose=2"]).arg(&p).output() {
                Ok(o) => prozparse::parse_codesign(&String::from_utf8_lossy(&o.stderr), o.status.success()),
                Err(_) => Sig { status: SigStatus::Unbekannt, von: String::new() },
            };
            v.push((p, sig));
        }
    } else {
        for p in pfade {
            v.push((p, Sig { status: SigStatus::Unbekannt, von: String::new() }));
        }
    }
    v
}

/// Signaturprüfung gibt es nur unter Windows und macOS.
pub fn signatur_moeglich() -> bool {
    cfg!(windows) || cfg!(target_os = "macos")
}

#[cfg(test)]
mod tests {
    #[test]
    fn b64() {
        assert_eq!(super::base64(b"Man"), "TWFu");
        assert_eq!(super::base64(b"Ma"), "TWE=");
        assert_eq!(super::base64(b"M"), "TQ==");
        assert_eq!(super::ps_str("O'Neil"), "'O''Neil'");
    }
}

// ---------------- Offene Ports ----------------

pub struct PortInfo {
    pub listen: Listen,
    pub process: String,
}

/// Lauschende Ports des eigenen Rechners mit Prozessnamen.
pub fn open_ports() -> Vec<PortInfo> {
    let netstat = run("netstat", &["-ano"]).unwrap_or_default();
    let listens = netparse::parse_netstat(&netstat);
    let procs = process_map();
    let mut v: Vec<PortInfo> = listens
        .into_iter()
        .map(|l| {
            let process = procs.get(&l.pid).cloned().unwrap_or_else(|| "—".into());
            PortInfo { listen: l, process }
        })
        .collect();
    v.sort_by_key(|p| p.listen.port);
    v.dedup_by(|a, b| a.listen.port == b.listen.port && a.listen.proto == b.listen.proto);
    v
}

/// PID -> Prozessname über `tasklist`.
fn process_map() -> HashMap<u32, String> {
    let mut m = HashMap::new();
    if let Some(out) = run("tasklist", &["/FO", "CSV", "/NH"]) {
        for line in out.lines() {
            // "image.exe","1044","Console","1","12.345 K"
            let cols: Vec<&str> = line.split("\",\"").collect();
            if cols.len() >= 2 {
                let name = cols[0].trim_start_matches('"').to_string();
                if let Ok(pid) = cols[1].trim_matches('"').parse::<u32>() {
                    m.insert(pid, name);
                }
            }
        }
    }
    m
}

// ---------------- Geräte im Netz ----------------

pub fn lan_devices() -> Vec<Device> {
    let arp = run("arp", &["-a"]).unwrap_or_default();
    netparse::parse_arp(&arp)
}

// ---------------- Schutzstatus ----------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ok,
    Warn,
    Bad,
    Unknown,
}

#[derive(Clone)]
pub struct Check {
    pub name: String,
    pub state: State,
    pub detail: String,
}

pub fn protection_status() -> Vec<Check> {
    let mut v = Vec::new();

    // Firewall (alle Profile)
    let fw = run("netsh", &["advfirewall", "show", "allprofiles", "state"]).unwrap_or_default();
    let on = fw.matches("ON").count();
    let off = fw.to_uppercase().matches("OFF").count();
    v.push(Check {
        name: "Firewall".into(),
        state: if off > 0 {
            State::Warn
        } else if on > 0 {
            State::Ok
        } else {
            State::Unknown
        },
        detail: if off > 0 {
            t("Mindestens ein Profil ist aus.").into()
        } else if on > 0 {
            t("Für alle Profile aktiv.").into()
        } else {
            t("Status nicht lesbar.").into()
        },
    });

    // Virenschutz (Defender Echtzeitschutz)
    let def = run(
        "powershell",
        &["-NoProfile", "-Command", "(Get-MpComputerStatus).RealTimeProtectionEnabled"],
    )
    .unwrap_or_default();
    v.push(bool_check(t("Virenschutz (Echtzeit)"), &def, t("Aktiv."), t("Ausgeschaltet.")));

    // Windows-Update-Dienst
    let wu = run(
        "powershell",
        &["-NoProfile", "-Command", "(Get-Service wuauserv).Status"],
    )
    .unwrap_or_default();
    let running = wu.to_lowercase().contains("running");
    v.push(Check {
        name: t("Windows-Update").into(),
        state: if running { State::Ok } else { State::Warn },
        detail: if running { t("Dienst läuft.").into() } else { t("Dienst nicht aktiv.").into() },
    });

    // SmartScreen
    let ss = run(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer' -Name SmartScreenEnabled -ErrorAction SilentlyContinue).SmartScreenEnabled",
        ],
    )
    .unwrap_or_default();
    let ss = ss.trim();
    v.push(Check {
        name: "SmartScreen".into(),
        state: if ss.eq_ignore_ascii_case("Off") {
            State::Warn
        } else if ss.is_empty() {
            State::Unknown
        } else {
            State::Ok
        },
        detail: if ss.is_empty() { t("Status nicht lesbar.").into() } else { tf("Einstellung: {ss}", &[("ss", &ss)]) },
    });

    v
}

fn bool_check(name: &str, out: &str, yes: &str, no: &str) -> Check {
    let v = out.trim().to_lowercase();
    let (state, detail) = if v.contains("true") {
        (State::Ok, yes)
    } else if v.contains("false") {
        (State::Bad, no)
    } else {
        (State::Unknown, t("Status nicht lesbar."))
    };
    Check { name: name.into(), state, detail: detail.into() }
}

/// Gesamtpunktzahl 0..100 aus den Checks (grobe Orientierung).
pub fn score(checks: &[Check]) -> u32 {
    if checks.is_empty() {
        return 0;
    }
    let per = 100.0 / checks.len() as f64;
    let mut s = 0.0;
    for c in checks {
        s += match c.state {
            State::Ok => per,
            State::Warn => per * 0.5,
            State::Unknown => per * 0.5,
            State::Bad => 0.0,
        };
    }
    s.round() as u32
}

// ---------------- Papierkorb ----------------

#[derive(Clone)]
pub struct RecItem {
    pub rec: Recycled,
    pub i_path: PathBuf,
    pub r_path: PathBuf,
}

/// Listet Dateien im Papierkorb aller Laufwerke (defensiv, nur Lesen).
pub fn list_recycle() -> Vec<RecItem> {
    let mut out = Vec::new();
    for drive in available_drives() {
        let bin = PathBuf::from(format!("{drive}:\\$Recycle.Bin"));
        let sids = match std::fs::read_dir(&bin) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for sid in sids.flatten() {
            let dir = sid.path();
            let entries = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for e in entries.flatten() {
                let p = e.path();
                let name = match p.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if !name.starts_with("$I") {
                    continue;
                }
                let bytes = match std::fs::read(&p) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                if let Some(rec) = recycle::parse_i_file(&bytes) {
                    let r_name = format!("$R{}", &name[2..]);
                    let r_path = dir.join(r_name);
                    out.push(RecItem { rec, i_path: p, r_path });
                }
            }
        }
    }
    out.sort_by(|a, b| b.rec.filetime.cmp(&a.rec.filetime));
    out
}

/// Stellt eine Datei aus dem Papierkorb an ihren Ursprungsort zurück.
pub fn restore(item: &RecItem) -> Result<(), String> {
    let dest = PathBuf::from(&item.rec.orig_path);
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if dest.exists() {
        return Err(t("Am Zielort existiert bereits eine Datei gleichen Namens.").into());
    }
    // $R zurückschieben, dann $I entfernen.
    match std::fs::rename(&item.r_path, &dest) {
        Ok(_) => {
            let _ = std::fs::remove_file(&item.i_path);
            Ok(())
        }
        Err(e) => Err(tf("Wiederherstellen fehlgeschlagen: {e}", &[("e", &e)])),
    }
}

fn available_drives() -> Vec<char> {
    let mut v = Vec::new();
    for c in b'A'..=b'Z' {
        let d = c as char;
        if PathBuf::from(format!("{d}:\\")).exists() {
            v.push(d);
        }
    }
    if v.is_empty() {
        v.push('C');
    }
    v
}
