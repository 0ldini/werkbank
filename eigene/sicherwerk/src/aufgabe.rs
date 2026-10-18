//! Zeitplan im System: unter Windows über die Aufgabenplanung (`schtasks /Create /XML …`,
//! die Aufgabe ruft `sicherwerk --auftrag "<name>"` ohne Fenster auf); unter Linux nur
//! Texte für cron oder systemd-Timer zum Selbst-Einrichten.

#![allow(dead_code)]

use crate::auftrag::{Auftrag, Zeitplan};
use crate::zeitplan::{self, Takt};

/// Pfad der laufenden Programmdatei.
pub fn exe_pfad() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "sicherwerk".to_string())
}

/// Takt und Argumente für einen Auftrag (None bei «nur von Hand»).
pub fn takt(a: &Auftrag) -> Option<(Takt, Vec<String>)> {
    let (h, m) = (a.stunde.min(23), a.minute.min(59));
    match a.zeitplan {
        Zeitplan::Keiner => None,
        Zeitplan::Taeglich => Some((Takt::Taeglich { h, m }, vec!["--auftrag".into(), a.name.trim().to_string()])),
        Zeitplan::Woechentlich => {
            Some((Takt::Woechentlich { tag: a.wochentag.min(6), h, m }, vec!["--auftrag".into(), a.name.trim().to_string()]))
        }
        Zeitplan::BeimAnschliessen => Some((Takt::Minuten(15), vec!["--anschluss".into()])),
    }
}

// ------------------------------------------------------------------ Windows

#[cfg(windows)]
fn schtasks(args: &[String]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(text.trim().to_string())
    }
}

#[cfg(not(windows))]
fn schtasks(_args: &[String]) -> Result<String, String> {
    Err("schtasks gibt es nur unter Windows".into())
}

fn benutzer() -> String {
    match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) if !d.is_empty() && !u.is_empty() => format!("{d}\\{u}"),
        (_, Ok(u)) if !u.is_empty() => u,
        _ => String::new(),
    }
}

fn aufgabe_anlegen(aufgabe: &str, beschreibung: &str, argumente: &str, t: Takt) -> Result<(), String> {
    let dir = crate::store::ordner().ok_or_else(|| "Benutzerordner nicht gefunden".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let xml = zeitplan::aufgabe_xml(beschreibung, &exe_pfad(), argumente, t, &crate::lauf::heute(), &benutzer());
    let pfad = dir.join("aufgabe.xml");
    std::fs::write(&pfad, zeitplan::utf16_bom(&xml)).map_err(|e| e.to_string())?;
    let r = schtasks(&zeitplan::schtasks_anlegen(aufgabe, &pfad.display().to_string()));
    let _ = std::fs::remove_file(&pfad);
    r.map(|_| ())
}

/// Trägt einen Auftrag in die Aufgabenplanung ein (ersetzt eine vorhandene Aufgabe).
pub fn einrichten(a: &Auftrag) -> Result<(), String> {
    let Some((t, args)) = takt(a) else {
        return entfernen(&a.name);
    };
    if a.zeitplan == Zeitplan::BeimAnschliessen {
        // Eine gemeinsame Aufgabe für alle «beim Anschliessen»; eine alte eigene Aufgabe weg.
        let _ = entfernen(&a.name);
        return aufgabe_anlegen(zeitplan::ANSCHLUSS_AUFGABE, "Sicherwerk: Ziellaufwerke prüfen", &args.join(" "), t);
    }
    let argumente = zeitplan::argumente_auftrag(&a.name);
    aufgabe_anlegen(&zeitplan::aufgabenname(&a.name), &format!("Sicherwerk: {}", a.name.trim()), &argumente, t)
}

/// Entfernt die Aufgabe eines Auftrags (fehlt sie, ist das kein Fehler).
pub fn entfernen(name: &str) -> Result<(), String> {
    let aufgabe = zeitplan::aufgabenname(name);
    if !vorhanden(&aufgabe) {
        return Ok(());
    }
    schtasks(&zeitplan::schtasks_loeschen(&aufgabe)).map(|_| ())
}

/// Entfernt die gemeinsame Anschluss-Aufgabe, wenn kein anderer Auftrag sie mehr braucht.
pub fn anschluss_aufraeumen(alle: &[Auftrag], ausser: u64) -> Result<(), String> {
    let gebraucht =
        alle.iter().any(|a| a.id != ausser && a.zeitplan == Zeitplan::BeimAnschliessen && a.aufgabe_eingerichtet);
    if gebraucht || !vorhanden(zeitplan::ANSCHLUSS_AUFGABE) {
        return Ok(());
    }
    schtasks(&zeitplan::schtasks_loeschen(zeitplan::ANSCHLUSS_AUFGABE)).map(|_| ())
}

pub fn vorhanden(aufgabe: &str) -> bool {
    cfg!(windows) && schtasks(&zeitplan::schtasks_abfragen(aufgabe)).is_ok()
}

// ------------------------------------------------------------------ Linux

/// Texte für Linux: crontab-Zeile, Name der systemd-Einheit, .service und .timer.
pub struct LinuxTexte {
    pub cron: String,
    pub einheit: String,
    pub dienst: String,
    pub timer: String,
}

pub fn linux_texte(a: &Auftrag) -> Option<LinuxTexte> {
    let (t, args) = takt(a)?;
    let exe = exe_pfad();
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (einheit, beschreibung) = if a.zeitplan == Zeitplan::BeimAnschliessen {
        ("sicherwerk-anschluss".to_string(), "Sicherwerk: Ziellaufwerke prüfen".to_string())
    } else {
        (zeitplan::einheit_name(&a.name, a.id), format!("Sicherwerk: {}", a.name.trim()))
    };
    Some(LinuxTexte {
        cron: zeitplan::cron_zeile(&exe, &arg_refs, t),
        dienst: zeitplan::systemd_dienst(&beschreibung, &exe, &arg_refs),
        timer: zeitplan::systemd_timer(&beschreibung, t),
        einheit,
    })
}
