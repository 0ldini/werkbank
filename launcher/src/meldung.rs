//! Zählen: Beim Start meldet sich die Werkbank beim Zähler (zufällige Gerätekennung,
//! Computername, Benutzer, System, Version, installierte Programme mit Version), danach alle
//! 5 Minuten ein kurzes Lebenszeichen. So zeigt der Zähler Geräte, wer gerade online ist und
//! den Verlauf der Installationen und Updates. Abschaltbar in den Einstellungen. Updates laufen
//! davon unabhängig über GitHub.

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

/// An/aus aus den Einstellungen (wirkt sofort, auch auf das Lebenszeichen).
pub static AN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn senden(body: serde_json::Value) {
    if !AN.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let url = std::env::var("WERKBANK_ZAEHLER").unwrap_or_else(|_| ZAEHLER.to_string());
    if !url.is_empty() {
        let _ = crate::core::post_json(&url, &body.to_string());
    }
}

/// Beim Start melden: Gerät und installierte Programme mit Version (Kennung → Version).
/// Blockiert kurz, darum im Hintergrund aufrufen. Fehler bleiben still.
pub fn melden(programme: &[(String, String)]) {
    let liste: serde_json::Map<String, serde_json::Value> = programme.iter().map(|(id, v)| (id.clone(), serde_json::Value::String(v.clone()))).collect();
    senden(serde_json::json!({
        "aktion": "melden",
        "geraet": geraet(),
        "name": geraetename(),
        "benutzer": std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default(),
        "system": system(),
        "version": env!("CARGO_PKG_VERSION"),
        "programme": liste,
    }));
}

/// Lebenszeichen, solange die Werkbank offen ist (für «gerade online»).
pub fn puls() {
    senden(serde_json::json!({ "aktion": "puls", "geraet": geraet() }));
}

/// Im Hintergrund: einmal melden, danach alle 5 Minuten ein Lebenszeichen.
pub fn starten(programme: Vec<(String, String)>) {
    std::thread::spawn(move || {
        melden(&programme);
        loop {
            std::thread::sleep(std::time::Duration::from_secs(300));
            puls();
        }
    });
}
