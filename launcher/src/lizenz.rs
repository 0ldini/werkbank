//! Freischaltung der Werkbank: ein gemeinsamer Code für alle, geprüft vom Lizenzserver auf
//! jarns.ch. Der Server zählt die Geräte und gibt eine unterschriebene Lizenz zurück, die
//! `LIZENZ_TAGE` Tage ohne Verbindung gilt und bei jedem Start mit Internet erneuert wird.
//!
//! Dateien im Einstellungsordner der Werkbank (neben sprache.txt):
//! - `lizenz.txt`: Zeile 1 `werkbank|1|<Gerät>|<Ablauf>`, Zeile 2 Ed25519-Unterschrift (hex)
//! - `geraet.txt`: zufällige Kennung dieses Geräts
//! - `launcher.txt`: Pfad dieses Launchers (für den Knopf «Werkbank öffnen» in den Programmen)
//!
//! Die Prüfung ist dieselbe wie in eigene/lizenz.rs.

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const PUBLIC_KEY_HEX: &str = "4cb8422c7e8266fe45c99fb20e6336ca5d282a5cc2251fca9f95f9e4a6a90493";
pub const SERVER: &str = "https://jarns.ch/werkbank/api.php";

fn hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// Prüft den Text einer Lizenz; liefert (Gerät, Ablauf in Unix-Sekunden).
pub fn pruefen(text: &str, jetzt: u64) -> Option<(String, u64)> {
    let mut lines = text.lines();
    let payload = lines.next()?.trim();
    let sig: [u8; 64] = hex(lines.next()?)?.try_into().ok()?;
    let key: [u8; 32] = hex(PUBLIC_KEY_HEX)?.try_into().ok()?;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&key).ok()?;
    vk.verify_strict(payload.as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig)).ok()?;
    let parts: Vec<&str> = payload.split('|').collect();
    if parts.len() != 4 || parts[0] != "werkbank" || parts[1] != "1" {
        return None;
    }
    let ablauf: u64 = parts[3].parse().ok()?;
    (ablauf > jetzt).then(|| (parts[2].to_string(), ablauf))
}

fn ordner() -> Option<PathBuf> {
    crate::sprache::werkbank_file()?.parent().map(|p| p.to_path_buf())
}

fn jetzt() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn schreiben(name: &str, text: &str) -> Result<()> {
    let dir = ordner().ok_or_else(|| anyhow!("kein Einstellungsordner"))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(name), text)?;
    Ok(())
}

/// Kennung dieses Geräts; wird beim ersten Aufruf zufällig erzeugt und gespeichert.
pub fn geraet() -> String {
    if let Some(id) = ordner().and_then(|d| std::fs::read_to_string(d.join("geraet.txt")).ok()) {
        let id = id.trim().to_string();
        if id.len() >= 16 && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return id;
        }
    }
    let mut h = Sha256::new();
    h.update(format!("{:?}", std::time::SystemTime::now()));
    h.update(std::process::id().to_le_bytes());
    h.update(geraetename());
    let probe = 0u8;
    h.update(format!("{:p}", &probe));
    let id: String = h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect();
    let _ = schreiben("geraet.txt", &id);
    id
}

pub fn geraetename() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn benutzer() -> String {
    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default()
}

fn system() -> String {
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "linux" => "Linux",
        "macos" => "macOS",
        o => o,
    };
    format!("{os} {}", std::env::consts::ARCH)
}

/// Gültige Lizenz für dieses Gerät vorhanden?
pub fn gueltig() -> bool {
    let Some(text) = ordner().and_then(|d| std::fs::read_to_string(d.join("lizenz.txt")).ok()) else { return false };
    matches!(pruefen(&text, jetzt()), Some((g, _)) if g == geraet())
}

/// Ablauf der gespeicherten Lizenz (Unix-Sekunden).
pub fn ablauf() -> Option<u64> {
    let text = std::fs::read_to_string(ordner()?.join("lizenz.txt")).ok()?;
    pruefen(&text, jetzt()).map(|x| x.1)
}

pub fn entfernen() {
    if let Some(d) = ordner() {
        let _ = std::fs::remove_file(d.join("lizenz.txt"));
    }
}

/// Pfad dieses Launchers für die Programme ablegen.
pub fn launcher_merken() {
    if let Ok(me) = std::env::current_exe() {
        let _ = schreiben("launcher.txt", &me.display().to_string());
    }
}

/// Antwort des Servers.
#[derive(Clone, Debug, PartialEq)]
pub enum Antwort {
    Ok,
    /// Code falsch.
    Code,
    /// Gerät gesperrt.
    Gesperrt,
    /// Neuer Code nötig (geändert) oder Gerät unbekannt.
    Neu,
    /// Kein Netz, Server nicht erreichbar oder Fehler.
    Fehler(String),
}

fn anfrage(aktion: &str, code: Option<&str>, programme: &[String]) -> Antwort {
    let url = std::env::var("WERKBANK_LIZENZ_URL").unwrap_or_else(|_| SERVER.to_string());
    let mut body = serde_json::json!({
        "aktion": aktion,
        "geraet": geraet(),
        "name": geraetename(),
        "benutzer": benutzer(),
        "system": system(),
        "version": env!("CARGO_PKG_VERSION"),
        "programme": programme,
    });
    if let Some(c) = code {
        body["code"] = serde_json::Value::String(c.to_string());
    }
    let text = match crate::core::post_json(&url, &body.to_string()) {
        Ok(t) => t,
        Err(e) => return Antwort::Fehler(format!("{e:#}")),
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Antwort::Fehler(crate::sprache::t("Unerwartete Antwort vom Lizenzserver").to_string()),
    };
    if v["ok"].as_bool() == Some(true) {
        let Some(l) = v["lizenz"].as_str() else { return Antwort::Fehler("lizenz fehlt".into()) };
        if pruefen(l, jetzt()).is_none() {
            return Antwort::Fehler(crate::sprache::t("Die Lizenz vom Server ist ungültig").to_string());
        }
        if let Err(e) = schreiben("lizenz.txt", l) {
            return Antwort::Fehler(format!("{e:#}"));
        }
        return Antwort::Ok;
    }
    match v["grund"].as_str().unwrap_or("") {
        "code" => Antwort::Code,
        "gesperrt" => Antwort::Gesperrt,
        "neu" | "unbekannt" => Antwort::Neu,
        g => Antwort::Fehler(v["meldung"].as_str().map(str::to_string).unwrap_or_else(|| format!("Server: {g}"))),
    }
}

/// Mit dem Code freischalten.
pub fn aktivieren(code: &str, programme: &[String]) -> Antwort {
    anfrage("aktivieren", Some(code), programme)
}

/// Beim Start melden und die Lizenz erneuern. Bei «gesperrt» oder «neuer Code» wird die
/// Lizenz entfernt.
pub fn melden(programme: &[String]) -> Antwort {
    let a = anfrage("melden", None, programme);
    if matches!(a, Antwort::Gesperrt | Antwort::Neu) {
        entfernen();
    }
    a
}

#[cfg(test)]
mod tests {
    #[test]
    fn unterschrift() {
        let ok = "werkbank|1|00112233445566778899aabbccddeeff|4102444800\ndc1a1d96a8b5ce76f1c8c4dafbd9a9713e70701efc5f8c559f9a18adcf4f7e4e951ce97d2754eaa72f027238dd4bbed05601d283d71ec9dc53a609dd609d0d0b";
        assert_eq!(super::pruefen(ok, 1_800_000_000).map(|x| x.0), Some("00112233445566778899aabbccddeeff".to_string()));
        assert!(super::pruefen(ok, 4_102_444_801).is_none());
        assert!(super::pruefen(&ok.replace("4102444800", "4102444900"), 0).is_none());
        assert!(super::pruefen("unsinn", 0).is_none());
    }
}
