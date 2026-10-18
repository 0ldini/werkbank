//! Systemschicht (defensiv, Windows): fragt lauschende Ports, Geräte im Netz
//! und den Schutzstatus ab und listet/​stellt Papierkorb-Dateien wieder her.
//! Ausschliesslich Lesen bzw. Status prüfen – keine Angriffs- oder
//! Auslese-Funktionen.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use crate::netparse::{self, Device, Listen};
use crate::recycle::{self, Recycled};
use crate::sprache::{t, tf};

fn cmd(program: &str) -> Command {
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

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = cmd(program).args(args).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
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
