//! Wer greift zu? (defensiv, nur Lesen): Verbindungen nach aussen je Prozess,
//! Zugriffe auf Kamera und Mikrofon (Windows-Datenschutzprotokoll, Linux: offene Geräte)
//! und Fernzugriff (Fernwartungs-Programme, Remotedesktop, Remoteunterstützung).
//! Es wird nichts verändert; Einstellungen öffnet nur der Benutzer selbst.

use std::collections::HashMap;

use crate::netparse::{self, Conn};
use crate::prozparse::{self, Ort};
use crate::sys;
use crate::waechter;

// ---------------- Verbindungen nach aussen ----------------

#[derive(Clone)]
pub struct VerbProz {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    pub ort: Ort,
    pub imitation: Option<&'static str>,
    pub miner: bool,
    pub fern: Option<&'static str>,
    pub conns: Vec<Conn>,
}

impl VerbProz {
    /// Verbindet sich mit einem typischen Mining-Pool-Port.
    pub fn mining_port(&self) -> bool {
        self.conns.iter().any(|c| netparse::port_name(c.remote_port).is_some_and(|(_, m)| m))
    }
}

/// Hergestellte TCP-Verbindungen (ohne Loopback), nach Prozess gruppiert.
pub fn verbindungen() -> Vec<VerbProz> {
    let conns: Vec<Conn> = if cfg!(windows) {
        netparse::parse_netstat_established(&sys::run("netstat", &["-ano", "-p", "TCP"]).unwrap_or_default())
            .into_iter()
            .chain(netparse::parse_netstat_established(&sys::run("netstat", &["-ano", "-p", "TCPv6"]).unwrap_or_default()))
            .collect()
    } else if cfg!(target_os = "macos") {
        netparse::parse_lsof(&sys::run("lsof", &["-nP", "-iTCP", "-sTCP:ESTABLISHED"]).unwrap_or_default())
    } else {
        let ss = netparse::parse_ss(&sys::run("ss", &["-tnpH", "state", "established"]).unwrap_or_default());
        if ss.is_empty() {
            netparse::parse_lsof(&sys::run("lsof", &["-nP", "-iTCP", "-sTCP:ESTABLISHED"]).unwrap_or_default())
        } else {
            ss
        }
    };
    let procs = waechter::prozesse_kurz();
    let mut m: HashMap<u32, VerbProz> = HashMap::new();
    for c in conns {
        let e = m.entry(c.pid).or_insert_with(|| {
            let (name, exe) = procs.get(&c.pid).cloned().unwrap_or_else(|| (if c.name.is_empty() { "—".into() } else { c.name.clone() }, String::new()));
            VerbProz {
                pid: c.pid,
                ort: prozparse::ort_von(&exe, cfg!(windows)),
                imitation: if cfg!(windows) { prozparse::imitation(&name, &exe) } else { None },
                miner: prozparse::ist_miner(&name),
                fern: prozparse::fernzugriff(&name),
                name,
                exe,
                conns: Vec::new(),
            }
        });
        if !e.conns.iter().any(|x| x.remote_ip == c.remote_ip && x.remote_port == c.remote_port) {
            e.conns.push(c);
        }
    }
    let mut v: Vec<VerbProz> = m.into_values().collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

// ---------------- Kamera & Mikrofon ----------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Geraet {
    Kamera,
    Mikrofon,
}

#[derive(Clone, Debug)]
pub struct KamEintrag {
    pub geraet: Geraet,
    pub app: String,
    pub pfad: String,
    /// FILETIME (0 = unbekannt)
    pub start: u64,
    pub stop: u64,
    pub aktiv: bool,
}

#[derive(Clone, Default)]
pub struct KamBefund {
    pub verfuegbar: bool,
    pub eintraege: Vec<KamEintrag>,
    /// Windows: Zugriff grundsätzlich erlaubt («Allow») oder gesperrt («Deny»).
    pub kamera_erlaubt: Option<bool>,
    pub mikro_erlaubt: Option<bool>,
}

/// App-Name aus einem Schlüssel des Datenschutzprotokolls.
pub fn app_name(schluessel: &str, nonpackaged: bool) -> (String, String) {
    if nonpackaged {
        let pfad = schluessel.replace('#', "\\");
        let name = pfad.rsplit('\\').next().unwrap_or(&pfad).to_string();
        (name, pfad)
    } else {
        let name = schluessel.split('_').next().unwrap_or(schluessel);
        let kurz = name.rsplit('.').next().filter(|s| !s.is_empty()).unwrap_or(name);
        (if name.starts_with("Microsoft.") { name.trim_start_matches("Microsoft.").to_string() } else { kurz.to_string() }, schluessel.to_string())
    }
}

pub fn kamera_mikro() -> KamBefund {
    #[cfg(windows)]
    {
        win_kamera()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux_kamera()
    }
    #[cfg(target_os = "macos")]
    KamBefund::default()
}

#[cfg(windows)]
fn win_kamera() -> KamBefund {
    use winreg::enums::*;
    use winreg::RegKey;
    const BASIS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\";
    let mut b = KamBefund { verfuegbar: true, ..Default::default() };
    for (geraet, sub) in [(Geraet::Kamera, "webcam"), (Geraet::Mikrofon, "microphone")] {
        for (hive, benutzer) in [(HKEY_CURRENT_USER, true), (HKEY_LOCAL_MACHINE, false)] {
            let Ok(k) = RegKey::predef(hive).open_subkey_with_flags(format!("{BASIS}{sub}"), KEY_READ) else { continue };
            if benutzer {
                let erlaubt = k.get_value::<String, _>("Value").ok().map(|v| !v.eq_ignore_ascii_case("Deny"));
                match geraet {
                    Geraet::Kamera => b.kamera_erlaubt = erlaubt,
                    Geraet::Mikrofon => b.mikro_erlaubt = erlaubt,
                }
            }
            let mut lies = |sk: &RegKey, name: &str, np: bool| {
                let start: u64 = sk.get_value("LastUsedTimeStart").unwrap_or(0);
                let stop: u64 = sk.get_value("LastUsedTimeStop").unwrap_or(0);
                if start == 0 {
                    return;
                }
                let (app, pfad) = app_name(name, np);
                if b.eintraege.iter().any(|e| e.geraet == geraet && e.pfad.eq_ignore_ascii_case(&pfad) && e.start == start) {
                    return;
                }
                b.eintraege.push(KamEintrag { geraet, app, pfad, start, stop, aktiv: stop == 0 || stop < start });
            };
            for name in k.enum_keys().flatten() {
                let Ok(sk) = k.open_subkey_with_flags(&name, KEY_READ) else { continue };
                if name.eq_ignore_ascii_case("NonPackaged") {
                    for n2 in sk.enum_keys().flatten() {
                        if let Ok(s2) = sk.open_subkey_with_flags(&n2, KEY_READ) {
                            lies(&s2, &n2, true);
                        }
                    }
                } else {
                    lies(&sk, &name, false);
                }
            }
        }
    }
    b.eintraege.sort_by(|a, c| c.aktiv.cmp(&a.aktiv).then(c.start.cmp(&a.start)));
    b
}

/// Linux: welche (eigenen) Prozesse halten gerade Kamera (/dev/video*) oder
/// Mikrofon (/dev/snd/pcm…c) offen? Ein Verlauf ist hier nicht verfügbar.
#[cfg(all(unix, not(target_os = "macos")))]
fn linux_kamera() -> KamBefund {
    let mut b = KamBefund { verfuegbar: true, ..Default::default() };
    let Ok(rd) = std::fs::read_dir("/proc") else { return KamBefund::default() };
    for e in rd.flatten() {
        let pid = e.file_name().to_string_lossy().to_string();
        if !pid.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else { continue };
        let mut kam = false;
        let mut mik = false;
        for fd in fds.flatten().take(4096) {
            if let Ok(ziel) = std::fs::read_link(fd.path()) {
                let z = ziel.to_string_lossy();
                if z.starts_with("/dev/video") {
                    kam = true;
                } else if z.starts_with("/dev/snd/pcm") && z.ends_with('c') {
                    mik = true;
                }
            }
        }
        if !(kam || mik) {
            continue;
        }
        let name = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default().trim().to_string();
        let pfad = std::fs::read_link(e.path().join("exe")).map(|p| p.display().to_string()).unwrap_or_default();
        for (an, g) in [(kam, Geraet::Kamera), (mik, Geraet::Mikrofon)] {
            if an {
                b.eintraege.push(KamEintrag { geraet: g, app: name.clone(), pfad: pfad.clone(), start: 0, stop: 0, aktiv: true });
            }
        }
    }
    b
}

// ---------------- Fernzugriff ----------------

#[derive(Clone)]
pub struct FernTool {
    pub produkt: &'static str,
    /// Prozess- oder Dienstname
    pub quelle: String,
    pub laeuft: bool,
    /// Als automatisch startender Dienst eingerichtet.
    pub dienst_auto: bool,
    pub pfad: String,
}

#[derive(Clone, Default)]
pub struct FernBefund {
    pub tools: Vec<FernTool>,
    /// Windows-Remotedesktop eingeschaltet?
    pub rdp: Option<bool>,
    /// Windows-Remoteunterstützung erlaubt?
    pub ra: Option<bool>,
    /// SSH-Server läuft (macOS/Linux)?
    pub ssh: Option<bool>,
}

pub fn fernzugriff() -> FernBefund {
    let mut b = FernBefund::default();
    let procs = waechter::prozesse_kurz();
    for (name, exe) in procs.values() {
        if let Some(prod) = prozparse::fernzugriff(name) {
            if let Some(t) = b.tools.iter_mut().find(|t| t.produkt == prod) {
                t.laeuft = true;
                if t.pfad.is_empty() {
                    t.pfad = exe.clone();
                }
            } else {
                b.tools.push(FernTool { produkt: prod, quelle: name.clone(), laeuft: true, dienst_auto: false, pfad: exe.clone() });
            }
        }
    }
    if !cfg!(windows) {
        let ssh = procs.values().any(|(n, _)| n == "sshd");
        b.ssh = Some(ssh);
    }
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        for d in crate::autostart::dienste() {
            let prod = prozparse::fern_dienst(&d.name).or_else(|| prozparse::fern_dienst(&d.anzeige));
            let Some(prod) = prod else { continue };
            if d.start == 4 {
                continue; // deaktiviert
            }
            let auto = d.start == 2;
            if let Some(t) = b.tools.iter_mut().find(|t| t.produkt == prod) {
                t.dienst_auto |= auto;
            } else {
                b.tools.push(FernTool { produkt: prod, quelle: d.anzeige.clone(), laeuft: false, dienst_auto: auto, pfad: crate::autostart::exe_von(&d.image) });
            }
        }
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        b.rdp = hklm
            .open_subkey_with_flags("SYSTEM\\CurrentControlSet\\Control\\Terminal Server", KEY_READ)
            .ok()
            .and_then(|k| k.get_value::<u32, _>("fDenyTSConnections").ok())
            .map(|v| v == 0);
        b.ra = hklm
            .open_subkey_with_flags("SYSTEM\\CurrentControlSet\\Control\\Remote Assistance", KEY_READ)
            .ok()
            .and_then(|k| k.get_value::<u32, _>("fAllowToGetHelp").ok())
            .map(|v| v == 1);
    }
    b.tools.sort_by(|a, c| c.laeuft.cmp(&a.laeuft).then(a.produkt.cmp(c.produkt)));
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appnamen() {
        assert_eq!(app_name("C:#Program Files#Zoom#bin#Zoom.exe", true), ("Zoom.exe".into(), "C:\\Program Files\\Zoom\\bin\\Zoom.exe".into()));
        assert_eq!(app_name("Microsoft.WindowsCamera_8wekyb3d8bbwe", false).0, "WindowsCamera");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0", false).0, "SpotifyMusic");
    }
}
