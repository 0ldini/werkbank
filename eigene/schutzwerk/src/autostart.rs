//! Autostart-Prüfung (defensiv): listet, was beim Start automatisch mitläuft – dort
//! verstecken sich Spionage- und Schadprogramme gern. Windows: Run/RunOnce-Schlüssel
//! (HKCU/HKLM, auch WOW6432Node), Autostart-Ordner, geplante Aufgaben, automatisch
//! startende Dienste ausserhalb von C:\Windows. macOS: LaunchAgents/LaunchDaemons.
//! Linux: ~/.config/autostart, systemd-Benutzer-Units, crontab.
//!
//! Neue Einträge seit der letzten Prüfung werden markiert (Grundlage als JSON im
//! Einstellungsordner). Eingriffe nur auf Wunsch und mit Bestätigung: Run-Wert vorher
//! sichern, dann entfernen; geplante Aufgabe deaktivieren; beides wiederherstellbar.
//! Es werden nie Dateien gelöscht.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::autoparse;
use crate::prozparse::Sig;
use crate::sprache::t;
use crate::sys::{self, config_dir};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Art {
    Run,
    RunOnce,
    Ordner,
    Aufgabe,
    Dienst,
    LaunchAgent,
    LaunchDaemon,
    Desktop,
    Systemd,
    Cron,
}

#[derive(Clone, Debug)]
pub enum Aktion {
    Keine,
    /// Wert in einem Run/RunOnce-Schlüssel.
    RunWert { hive: String, key: String, name: String },
    /// Geplante Aufgabe (voller Pfad, z. B. «\Ordner\Name»).
    Aufgabe { pfad: String },
}

#[derive(Clone, Debug)]
pub struct Eintrag {
    pub id: String,
    pub art: Art,
    /// Wo der Eintrag steht (Schlüssel, Ordner, Aufgabenpfad …).
    pub ort: String,
    pub name: String,
    pub befehl: String,
    /// Programmdatei (Umgebungsvariablen ersetzt), leer wenn unbekannt.
    pub exe: String,
    pub aus: bool,
    /// Zum Betriebssystem gehörend (standardmässig ausgeblendet).
    pub system: bool,
    pub neu: bool,
    pub aktion: Aktion,
    pub sig: Option<Sig>,
}

#[derive(Clone, Default)]
pub struct Ergebnis {
    pub eintraege: Vec<Eintrag>,
    /// Die Grundlage wurde bei dieser Prüfung erst angelegt.
    pub basis_neu_angelegt: bool,
    pub fehler: Vec<&'static str>,
}

fn env_lookup(n: &str) -> Option<String> {
    std::env::var(n).ok().or_else(|| std::env::vars().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v))
}

pub fn exe_von(befehl: &str) -> String {
    autoparse::exe_aus_befehl(&autoparse::env_ersetzen(befehl, env_lookup))
}

fn im_windows_ordner(exe: &str) -> bool {
    let w = env_lookup("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()).to_lowercase();
    let e = exe.to_lowercase();
    e.starts_with(&format!("{w}\\")) || e.starts_with("c:\\windows\\") || e.starts_with("\\systemroot\\") || e.starts_with("system32\\")
}

// ---------------- Grundlage (bekannter Stand) ----------------

fn basis_datei() -> Option<PathBuf> {
    Some(config_dir()?.join("autostart-grundlage.json"))
}

fn basis_laden() -> Option<Vec<String>> {
    let text = std::fs::read_to_string(basis_datei()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Speichert die aktuellen Einträge als bekannt.
pub fn basis_speichern(ids: &[String]) -> Result<(), String> {
    let p = basis_datei().ok_or_else(|| t("Einstellungsordner unbekannt.").to_string())?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(ids).map_err(|e| e.to_string())?;
    std::fs::write(p, text).map_err(|e| e.to_string())
}

// ---------------- Sicherungen (deaktivierte Einträge) ----------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sicherung {
    /// «run» oder «aufgabe»
    pub art: String,
    #[serde(default)]
    pub hive: String,
    #[serde(default)]
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub typ: String,
    #[serde(default)]
    pub daten: String,
    #[serde(default)]
    pub zeit: u64,
}

fn sicherung_datei() -> Option<PathBuf> {
    Some(config_dir()?.join("autostart-sicherung.json"))
}

pub fn sicherungen() -> Vec<Sicherung> {
    sicherung_datei().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn sicherungen_schreiben(v: &[Sicherung]) -> Result<(), String> {
    let p = sicherung_datei().ok_or_else(|| t("Einstellungsordner unbekannt.").to_string())?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(p, text).map_err(|e| e.to_string())
}

// ---------------- Prüfen ----------------

pub fn pruefen() -> Ergebnis {
    let mut e = Ergebnis::default();
    #[cfg(windows)]
    windows::sammeln(&mut e);
    #[cfg(target_os = "macos")]
    mac::sammeln(&mut e);
    #[cfg(all(unix, not(target_os = "macos")))]
    linux::sammeln(&mut e);

    // Signaturen der sichtbaren Einträge (einmal je Datei).
    if sys::signatur_moeglich() {
        let pfade: Vec<String> = e.eintraege.iter().filter(|x| !x.system && !x.exe.is_empty() && Path::new(&x.exe).is_file()).map(|x| x.exe.clone()).collect();
        let sigs = sys::signaturen(pfade);
        for x in e.eintraege.iter_mut() {
            x.sig = sigs.iter().find(|(p, _)| p.eq_ignore_ascii_case(&x.exe)).map(|(_, s)| s.clone());
        }
    }

    // Abgleich mit dem bekannten Stand.
    let ids: Vec<String> = e.eintraege.iter().map(|x| x.id.clone()).collect();
    let basis = basis_laden();
    let neu = autoparse::neue(&ids, basis.as_deref());
    for (x, n) in e.eintraege.iter_mut().zip(neu) {
        x.neu = n;
    }
    if basis.is_none() {
        e.basis_neu_angelegt = basis_speichern(&ids).is_ok();
    }
    e
}

fn id(art: Art, ort: &str, name: &str, befehl: &str) -> String {
    format!("{art:?}|{ort}|{name}|{befehl}")
}

fn eintrag(art: Art, ort: &str, name: &str, befehl: &str, aus: bool, system: bool, aktion: Aktion) -> Eintrag {
    let exe = if cfg!(windows) { exe_von(befehl) } else { autoparse::exe_aus_befehl(befehl) };
    Eintrag {
        id: id(art, ort, name, befehl),
        art,
        ort: ort.to_string(),
        name: name.to_string(),
        befehl: befehl.to_string(),
        exe,
        aus,
        system,
        neu: false,
        aktion,
        sig: None,
    }
}

// ---------------- Eingriffe (nur auf Wunsch, mit Bestätigung in der Oberfläche) ----------------

/// Deaktiviert einen Eintrag: Run-Wert sichern und entfernen bzw. Aufgabe deaktivieren.
pub fn deaktivieren(x: &Eintrag) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::deaktivieren(x)
    }
    #[cfg(not(windows))]
    {
        let _ = x;
        Err(t("Unter diesem System nicht verfügbar.").into())
    }
}

/// Stellt einen gesicherten Eintrag wieder her und entfernt ihn aus der Sicherung.
pub fn wiederherstellen(s: &Sicherung) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::wiederherstellen(s)?;
        let mut v = sicherungen();
        v.retain(|y| !(y.art == s.art && y.hive == s.hive && y.key == s.key && y.name == s.name && y.zeit == s.zeit));
        sicherungen_schreiben(&v)
    }
    #[cfg(not(windows))]
    {
        let _ = s;
        Err(t("Unter diesem System nicht verfügbar.").into())
    }
}

// ---------------- Windows ----------------

/// Ein Windows-Dienst aus der Registry.
#[derive(Clone, Debug)]
pub struct Dienst {
    pub name: String,
    pub anzeige: String,
    pub image: String,
    /// 2 = automatisch, 3 = manuell, 4 = deaktiviert
    pub start: u32,
}

/// Alle Windows-Dienste (keine Treiber) aus HKLM\SYSTEM\CurrentControlSet\Services.
pub fn dienste() -> Vec<Dienst> {
    #[cfg(windows)]
    {
        windows::dienste()
    }
    #[cfg(not(windows))]
    Vec::new()
}

#[cfg(windows)]
mod windows {
    use super::*;
    use winreg::enums::*;
    use winreg::RegKey;

    const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const RUNONCE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\RunOnce";
    const WOW_RUN: &str = "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Run";
    const WOW_RUNONCE: &str = "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\RunOnce";
    const APPROVED: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\";

    fn hive(name: &str) -> RegKey {
        if name == "HKLM" {
            RegKey::predef(HKEY_LOCAL_MACHINE)
        } else {
            RegKey::predef(HKEY_CURRENT_USER)
        }
    }

    /// Taskmanager hat den Eintrag ausgeschaltet?
    fn approved_aus(h: &str, liste: &str, name: &str) -> bool {
        let Ok(k) = hive(h).open_subkey_with_flags(format!("{APPROVED}{liste}"), KEY_READ) else { return false };
        k.get_raw_value(name).map(|v| autoparse::startup_approved_aus(&v.bytes)).unwrap_or(false)
    }

    pub fn sammeln(e: &mut Ergebnis) {
        // Run / RunOnce
        for (h, key, art, liste) in [
            ("HKCU", RUN, Art::Run, "Run"),
            ("HKCU", RUNONCE, Art::RunOnce, ""),
            ("HKLM", RUN, Art::Run, "Run"),
            ("HKLM", RUNONCE, Art::RunOnce, ""),
            ("HKLM", WOW_RUN, Art::Run, "Run32"),
            ("HKLM", WOW_RUNONCE, Art::RunOnce, ""),
        ] {
            let Ok(k) = hive(h).open_subkey_with_flags(key, KEY_READ) else { continue };
            let ort = format!("{h}\\{key}");
            for (name, _) in k.enum_values().flatten() {
                let Ok(befehl) = k.get_value::<String, _>(&name) else { continue };
                let aus = !liste.is_empty() && approved_aus(h, liste, &name);
                let aktion = Aktion::RunWert { hive: h.into(), key: key.into(), name: name.clone() };
                e.eintraege.push(eintrag(art, &ort, &name, &befehl, aus, false, aktion));
            }
        }

        // Autostart-Ordner (Verknüpfungen werden mit PowerShell aufgelöst).
        let mut ordner: Vec<PathBuf> = Vec::new();
        if let Some(a) = env_lookup("APPDATA") {
            ordner.push(PathBuf::from(a).join("Microsoft\\Windows\\Start Menu\\Programs\\Startup"));
        }
        if let Some(p) = env_lookup("ProgramData") {
            ordner.push(PathBuf::from(p).join("Microsoft\\Windows\\Start Menu\\Programs\\StartUp"));
        }
        let mut dateien: Vec<(PathBuf, String)> = Vec::new();
        for o in &ordner {
            let Ok(rd) = std::fs::read_dir(o) else { continue };
            for f in rd.flatten() {
                let p = f.path();
                let n = f.file_name().to_string_lossy().to_string();
                if n.eq_ignore_ascii_case("desktop.ini") || p.is_dir() {
                    continue;
                }
                dateien.push((p, o.display().to_string()));
            }
        }
        let lnks: Vec<String> = dateien.iter().filter(|(p, _)| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk"))).map(|(p, _)| sys::ps_str(&p.display().to_string())).collect();

        // Geplante Aufgaben + Ziele der Verknüpfungen in einem PowerShell-Aufruf.
        let script = format!(
            "$ErrorActionPreference='SilentlyContinue';\
foreach($t in Get-ScheduledTask){{$a=@($t.Actions)[0];$ex='';$ar='';\
if($a -and $a.Execute){{$ex=$a.Execute;$ar=$a.Arguments}} elseif($a -and $a.ClassId){{$ex='COM '+$a.ClassId}};\
(\"T`t$($t.TaskPath)`t$($t.TaskName)`t$($t.State)`t$($t.Author)`t$ex`t$ar\" -replace \"[`r`n]\",' ')}};\
$sh=New-Object -ComObject WScript.Shell;foreach($p in @({})){{$l=$sh.CreateShortcut($p);\"L`t$p`t$($l.TargetPath)`t$($l.Arguments)\"}}",
            lnks.join(",")
        );
        let out = sys::ps(&script).unwrap_or_default();
        let zeilen = autoparse::parse_tsv(&out);
        let mut ziele: Vec<(String, String, String)> = Vec::new();
        for z in &zeilen {
            let f = |i: usize| z.get(i).map(|s| s.as_str()).unwrap_or("");
            match f(0) {
                "T" => {
                    let pfad = format!("{}{}", f(1), f(2));
                    let exe_roh = f(5);
                    let befehl = if f(6).is_empty() {
                        exe_roh.to_string()
                    } else if exe_roh.contains(' ') && !exe_roh.starts_with('"') {
                        format!("\"{exe_roh}\" {}", f(6))
                    } else {
                        format!("{exe_roh} {}", f(6))
                    };
                    let exe = if exe_roh.starts_with("COM ") { String::new() } else { exe_von(exe_roh) };
                    let system = f(1).starts_with("\\Microsoft\\") && (exe.is_empty() || im_windows_ordner(&exe));
                    let mut x = eintrag(Art::Aufgabe, f(1), f(2), &befehl, f(3).eq_ignore_ascii_case("Disabled"), system, Aktion::Aufgabe { pfad });
                    x.exe = exe;
                    e.eintraege.push(x);
                }
                "L" => ziele.push((f(1).to_string(), f(2).to_string(), f(3).to_string())),
                _ => {}
            }
        }
        if out.trim().is_empty() {
            e.fehler.push(t("Geplante Aufgaben konnten nicht gelesen werden (PowerShell)."));
        }
        for (p, ort) in dateien {
            let ps = p.display().to_string();
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let ziel = ziele.iter().find(|(l, _, _)| l.eq_ignore_ascii_case(&ps));
            let (befehl, exe) = match ziel {
                Some((_, tp, ar)) if !tp.is_empty() => (format!("{tp} {ar}").trim().to_string(), tp.clone()),
                _ => (ps.clone(), ps.clone()),
            };
            let mut x = eintrag(Art::Ordner, &ort, &name, &befehl, false, false, Aktion::Keine);
            x.exe = exe;
            e.eintraege.push(x);
        }

        // Automatisch startende Dienste ausserhalb von C:\Windows.
        for d in dienste() {
            if d.start != 2 {
                continue;
            }
            let exe = exe_von(&d.image);
            if exe.is_empty() || im_windows_ordner(&exe) {
                continue;
            }
            let mut x = eintrag(Art::Dienst, "HKLM\\SYSTEM\\CurrentControlSet\\Services", &d.anzeige, &d.image, false, false, Aktion::Keine);
            x.exe = exe;
            e.eintraege.push(x);
        }
    }

    pub fn dienste() -> Vec<Dienst> {
        let mut v = Vec::new();
        let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags("SYSTEM\\CurrentControlSet\\Services", KEY_READ) else { return v };
        for name in root.enum_keys().flatten() {
            let Ok(k) = root.open_subkey_with_flags(&name, KEY_READ) else { continue };
            let typ: u32 = k.get_value("Type").unwrap_or(0);
            if typ & 0x30 == 0 {
                continue; // Treiber
            }
            let image: String = k.get_value("ImagePath").unwrap_or_default();
            let start: u32 = k.get_value("Start").unwrap_or(0);
            let anzeige: String = k.get_value::<String, _>("DisplayName").ok().filter(|s| !s.starts_with('@') && !s.is_empty()).unwrap_or_else(|| name.clone());
            v.push(Dienst { name, anzeige, image, start });
        }
        v
    }

    pub fn deaktivieren(x: &Eintrag) -> Result<(), String> {
        match &x.aktion {
            Aktion::RunWert { hive: h, key, name } => {
                let k = hive(h).open_subkey_with_flags(key, KEY_READ).map_err(|e| e.to_string())?;
                let raw = k.get_raw_value(name).map_err(|e| e.to_string())?;
                let typ = match raw.vtype {
                    winreg::enums::RegType::REG_SZ => "REG_SZ",
                    winreg::enums::RegType::REG_EXPAND_SZ => "REG_EXPAND_SZ",
                    _ => return Err(t("Unbekannter Werttyp – wird nicht verändert.").into()),
                };
                let daten: String = k.get_value(name).map_err(|e| e.to_string())?;
                // Zuerst sichern, erst dann entfernen.
                let mut v = sicherungen();
                v.push(Sicherung { art: "run".into(), hive: h.clone(), key: key.clone(), name: name.clone(), typ: typ.into(), daten, zeit: sys::jetzt_s() });
                sicherungen_schreiben(&v)?;
                let w = hive(h).open_subkey_with_flags(key, KEY_SET_VALUE).map_err(|e| {
                    // Sicherung zurücknehmen, wenn nicht entfernt werden kann.
                    let mut v2 = sicherungen();
                    v2.pop();
                    let _ = sicherungen_schreiben(&v2);
                    if h == "HKLM" {
                        t("Für Einträge unter HKLM Schutzwerk als Administrator starten.").to_string()
                    } else {
                        e.to_string()
                    }
                })?;
                w.delete_value(name).map_err(|e| e.to_string())
            }
            Aktion::Aufgabe { pfad } => {
                let o = sys::cmd("schtasks").args(["/change", "/tn", pfad, "/disable"]).output().map_err(|e| e.to_string())?;
                if !o.status.success() {
                    let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    return Err(if msg.is_empty() { t("schtasks meldet einen Fehler (Adminrechte nötig?).").into() } else { msg });
                }
                let mut v = sicherungen();
                v.push(Sicherung { art: "aufgabe".into(), hive: String::new(), key: String::new(), name: pfad.clone(), typ: String::new(), daten: String::new(), zeit: sys::jetzt_s() });
                sicherungen_schreiben(&v)
            }
            Aktion::Keine => Err(t("Für diesen Eintrag nicht möglich.").into()),
        }
    }

    pub fn wiederherstellen(s: &Sicherung) -> Result<(), String> {
        if s.art == "aufgabe" {
            let o = sys::cmd("schtasks").args(["/change", "/tn", &s.name, "/enable"]).output().map_err(|e| e.to_string())?;
            if !o.status.success() {
                let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
                return Err(if msg.is_empty() { t("schtasks meldet einen Fehler (Adminrechte nötig?).").into() } else { msg });
            }
            return Ok(());
        }
        let k = hive(&s.hive).open_subkey_with_flags(&s.key, KEY_SET_VALUE).map_err(|e| {
            if s.hive == "HKLM" {
                t("Für Einträge unter HKLM Schutzwerk als Administrator starten.").to_string()
            } else {
                e.to_string()
            }
        })?;
        if s.typ == "REG_EXPAND_SZ" {
            // Typ erhalten: über reg.exe schreiben.
            let ziel = format!("{}\\{}", s.hive, s.key);
            let o = sys::cmd("reg").args(["add", &ziel, "/v", &s.name, "/t", "REG_EXPAND_SZ", "/d", &s.daten, "/f"]).output().map_err(|e| e.to_string())?;
            if !o.status.success() {
                return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
            }
            Ok(())
        } else {
            k.set_value(&s.name, &s.daten).map_err(|e| e.to_string())
        }
    }
}

// ---------------- macOS ----------------

#[cfg(target_os = "macos")]
mod mac {
    use super::*;

    pub fn sammeln(e: &mut Ergebnis) {
        let home = std::env::var("HOME").unwrap_or_default();
        let orte = [
            (format!("{home}/Library/LaunchAgents"), Art::LaunchAgent),
            ("/Library/LaunchAgents".to_string(), Art::LaunchAgent),
            ("/Library/LaunchDaemons".to_string(), Art::LaunchDaemon),
        ];
        for (dir, art) in orte {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for f in rd.flatten() {
                let p = f.path();
                if !p.extension().is_some_and(|x| x == "plist") {
                    continue;
                }
                let ps = p.display().to_string();
                let xml = sys::cmd("plutil")
                    .args(["-convert", "xml1", "-o", "-"])
                    .arg(&p)
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .or_else(|| std::fs::read_to_string(&p).ok())
                    .unwrap_or_default();
                let (label, prog, aus) = autoparse::parse_plist(&xml);
                let name = label.unwrap_or_else(|| f.file_name().to_string_lossy().to_string());
                let system = name.starts_with("com.apple.");
                let mut x = eintrag(art, &dir, &name, prog.as_deref().unwrap_or(&ps), aus, system, Aktion::Keine);
                if prog.is_none() {
                    x.exe = ps;
                } else {
                    x.exe = prog.unwrap_or_default();
                }
                e.eintraege.push(x);
            }
        }
    }
}

// ---------------- Linux ----------------

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use super::*;

    pub fn sammeln(e: &mut Ergebnis) {
        let home = std::env::var("HOME").unwrap_or_default();
        let conf = std::env::var("XDG_CONFIG_HOME").ok().filter(|p| p.starts_with('/')).unwrap_or_else(|| format!("{home}/.config"));
        for (dir, system) in [(format!("{conf}/autostart"), false), ("/etc/xdg/autostart".to_string(), true)] {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for f in rd.flatten() {
                let p = f.path();
                if !p.extension().is_some_and(|x| x == "desktop") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                let (name, exec, aus) = autoparse::parse_desktop(&text);
                let name = name.unwrap_or_else(|| f.file_name().to_string_lossy().to_string());
                e.eintraege.push(eintrag(Art::Desktop, &dir, &name, exec.as_deref().unwrap_or(""), aus, system, Aktion::Keine));
            }
        }
        // systemd-Benutzer-Units
        if let Some(out) = sys::run("systemctl", &["--user", "list-unit-files", "--state=enabled", "--no-legend", "--no-pager"]) {
            let user_dir = format!("{conf}/systemd/user");
            for unit in autoparse::parse_unit_files(&out) {
                let datei = Path::new(&user_dir).join(&unit);
                let eigen = datei.is_file();
                let exec = if eigen { std::fs::read_to_string(&datei).ok().and_then(|t| autoparse::ini_wert(&t, "ExecStart")).unwrap_or_default() } else { String::new() };
                let exec = exec.trim_start_matches(['-', '@', '+', '!', ':']).to_string();
                e.eintraege.push(eintrag(Art::Systemd, if eigen { &user_dir } else { "systemd --user" }, &unit, &exec, false, !eigen, Aktion::Keine));
            }
        }
        // crontab des Benutzers
        if let Some(out) = sys::run("crontab", &["-l"]) {
            for (i, c) in autoparse::parse_crontab(&out).into_iter().enumerate() {
                e.eintraege.push(eintrag(Art::Cron, "crontab", &format!("#{}", i + 1), &c, false, false, Aktion::Keine));
            }
        }
    }
}
