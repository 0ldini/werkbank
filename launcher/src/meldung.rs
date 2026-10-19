//! Zählen: Beim Start meldet sich die Werkbank einmal beim Zähler (zufällige Gerätekennung,
//! Computername, Benutzer, System, Version, installierte Programme), damit sichtbar ist, wie
//! viele sie nutzen. Abschaltbar in den Einstellungen. Updates laufen davon unabhängig über
//! GitHub.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

const ZAEHLER: &str = "https://jarns.ch/werkbank/api.php";

fn ordner() -> Option<PathBuf> {
    crate::sprache::werkbank_file()?.parent().map(|p| p.to_path_buf())
}

/// Zufällige Kennung dieses Geräts, beim ersten Aufruf erzeugt und gespeichert.
fn geraet() -> String {
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
    if let Some(d) = ordner() {
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(d.join("geraet.txt"), &id);
    }
    id
}

fn geraetename() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
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

/// Einmal melden (blockiert kurz, darum im Hintergrund aufrufen). Fehler bleiben still.
pub fn melden(programme: &[String]) {
    let url = std::env::var("WERKBANK_ZAEHLER").unwrap_or_else(|_| ZAEHLER.to_string());
    if url.is_empty() {
        return;
    }
    let body = serde_json::json!({
        "aktion": "melden",
        "geraet": geraet(),
        "name": geraetename(),
        "benutzer": std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default(),
        "system": system(),
        "version": env!("CARGO_PKG_VERSION"),
        "programme": programme,
    });
    let _ = crate::core::post_json(&url, &body.to_string());
}
