//! Helles oder dunkles Design, gewählt im Werkbank-Launcher (Standard: hell).
//!
//! Reihenfolge: Umgebungsvariable `WERKBANK_THEME` («hell»/«dunkel», auch «light»/«dark»;
//! setzt der Launcher beim Start eines Programms), dann die Datei `thema.txt` im
//! Einstellungsordner der Werkbank, sonst hell.
//!
//! Beim Start einmal `thema::apply(&cc.egui_ctx)` aufrufen. Eigene Farben im Programm über
//! `thema::pick(hell, dunkel)` oder die fertigen Töne (`bg()`, `panel()`, `text()` …) wählen.
//!
//! Diese Datei ist in allen eigenen Programmen gleich (Vorlage: eigene/thema.rs).

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

/// «hell», «dunkel», «light», «dark» → true für dunkel.
pub fn parse(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "dunkel" | "dark" => Some(true),
        "hell" | "light" => Some(false),
        _ => None,
    }
}

/// Datei, in der der Launcher das gewählte Design ablegt (neben sprache.txt).
pub fn werkbank_file() -> Option<PathBuf> {
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(base).join("Werkbank").join("config").join("thema.txt"))
    } else if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/Werkbank/thema.txt"))
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("werkbank").join("thema.txt"))
    }
}

fn detect() -> bool {
    if let Some(d) = std::env::var("WERKBANK_THEME").ok().as_deref().and_then(parse) {
        return d;
    }
    if let Some(d) = werkbank_file().and_then(|p| std::fs::read_to_string(p).ok()).as_deref().and_then(parse) {
        return d;
    }
    false
}

/// 0 = noch nicht bestimmt, 1 = hell, 2 = dunkel.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn is_dark() -> bool {
    match CURRENT.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let d = detect();
            set_dark(d);
            d
        }
    }
}

pub fn set_dark(dark: bool) {
    CURRENT.store(if dark { 2 } else { 1 }, Ordering::Relaxed);
}

/// Wert je nach Design.
pub fn pick<T>(light: T, dark: T) -> T {
    if is_dark() {
        dark
    } else {
        light
    }
}

/// Setzt die egui-Grundfarben passend zum Design.
pub fn apply(ctx: &egui::Context) {
    // Fest auf hell/dunkel stellen, damit ein Wechsel des Systemdesigns nichts zurücksetzt.
    ctx.set_theme(if is_dark() { egui::Theme::Dark } else { egui::Theme::Light });
    let mut v = if is_dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = bg();
    v.window_fill = panel();
    v.extreme_bg_color = pick(egui::Color32::from_rgb(0xFF, 0xFF, 0xFF), egui::Color32::from_rgb(0x12, 0x13, 0x16));
    v.override_text_color = Some(text());
    v.window_stroke = egui::Stroke::new(1.0, border());
    ctx.set_visuals(v);
}

// Gemeinsame Töne der Werkbank (wie im Launcher).
pub fn bg() -> egui::Color32 {
    pick(egui::Color32::from_rgb(0xF3, 0xF4, 0xF6), egui::Color32::from_rgb(0x17, 0x18, 0x1C))
}
pub fn panel() -> egui::Color32 {
    pick(egui::Color32::WHITE, egui::Color32::from_rgb(0x21, 0x23, 0x29))
}
pub fn hover() -> egui::Color32 {
    pick(egui::Color32::from_rgb(0xEC, 0xEE, 0xF2), egui::Color32::from_rgb(0x2A, 0x2D, 0x35))
}
pub fn border() -> egui::Color32 {
    pick(egui::Color32::from_rgb(0xE1, 0xE4, 0xEA), egui::Color32::from_rgb(0x32, 0x35, 0x3D))
}
pub fn text() -> egui::Color32 {
    pick(egui::Color32::from_rgb(0x17, 0x18, 0x1C), egui::Color32::from_rgb(0xEC, 0xEE, 0xF2))
}
pub fn muted() -> egui::Color32 {
    pick(egui::Color32::from_rgb(0x5B, 0x61, 0x6E), egui::Color32::from_rgb(0xA9, 0xAE, 0xB9))
}

#[cfg(test)]
mod tests {
    #[test]
    fn lesen() {
        assert_eq!(super::parse("Dunkel"), Some(true));
        assert_eq!(super::parse("light"), Some(false));
        assert_eq!(super::parse("bunt"), None);
    }
}
