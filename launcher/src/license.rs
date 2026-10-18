//! Lizenzcode: Die Werkbank lässt sich nur mit gültigem Code einrichten und starten.
//!
//! Im Launcher steht nur die Prüfsumme (SHA-256) des Codes, nicht der Code selbst
//! (`launcher/license.sha256`). Nach der ersten richtigen Eingabe merkt sich der Launcher das in
//! `license.key` im Einstellungsordner; das Offline-Setup legt diese Datei selbst an.
//! Das ist eine Hürde für Unbefugte, kein Kopierschutz: wer die Dateien hat und den Code kennt,
//! kann installieren, und jeder mit dem Code darf es.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

const HASHES: &str = include_str!("../license.sha256");

fn expected() -> &'static str {
    HASHES
        .lines()
        .find_map(|l| l.strip_prefix("utf8="))
        .map(str::trim)
        .unwrap_or("")
}

pub fn hash(code: &str) -> String {
    Sha256::digest(code.trim().as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn is_valid(code: &str) -> bool {
    !code.trim().is_empty() && hash(code) == expected()
}

fn key_files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = crate::core::config_dir() {
        v.push(d.join("license.key"));
    }
    // Tragbare Ordner (portable.txt): Schlüsseldatei neben dem Launcher.
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        v.push(dir.join("license.key"));
    }
    v
}

/// Ist dieser Rechner freigeschaltet?
pub fn licensed() -> bool {
    if let Ok(code) = std::env::var("WERKBANK_LICENSE") {
        if is_valid(&code) {
            return true;
        }
    }
    key_files()
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .any(|t| t.trim() == expected() && !expected().is_empty())
}

/// Prüft den Code und merkt sich bei Erfolg die Freischaltung.
pub fn accept(code: &str) -> Result<bool> {
    if !is_valid(code) {
        return Ok(false);
    }
    let dir = crate::core::config_dir().context("kein Ort für die Einstellungen gefunden")?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("license.key"), expected())?;
    Ok(true)
}

/// Kleines Fenster, das den Code abfragt. Kehrt zurück, wenn es geschlossen wurde;
/// ob der Code stimmte, sagt danach `licensed()`.
pub fn prompt() {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([460.0, 250.0])
        .with_resizable(false);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(crate::icons::WERKBANK) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(
        "Werkbank – Lizenzcode",
        options,
        Box::new(|_cc| Ok(Box::new(Prompt::default()))),
    );
    if let Err(e) = result {
        eprintln!("Fenster konnte nicht starten: {e}");
    }
}

#[derive(Default)]
struct Prompt {
    code: String,
    error: Option<String>,
}

impl Prompt {
    fn check(&mut self, ctx: &eframe::egui::Context) {
        match accept(&self.code) {
            Ok(true) => ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close),
            Ok(false) => self.error = Some("Der Lizenzcode ist ungültig.".into()),
            Err(e) => self.error = Some(format!("Freischaltung nicht gespeichert: {e:#}")),
        }
    }
}

impl eframe::App for Prompt {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        use eframe::egui;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("Lizenzcode");
            ui.add_space(6.0);
            ui.label("Die Werkbank lässt sich nur mit gültigem Lizenzcode einrichten.");
            ui.add_space(10.0);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.code)
                    .hint_text("Lizenzcode eingeben")
                    .desired_width(f32::INFINITY),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(0xE0, 0x6C, 0x5A), e);
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Prüfen und weiter").clicked() || enter {
                    self.check(ctx);
                }
                if ui.button("Beenden").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falscher_code_wird_abgelehnt() {
        assert!(!is_valid(""));
        assert!(!is_valid("irgendwas"));
    }

    #[test]
    fn hashdatei_hat_utf8_zeile() {
        assert_eq!(expected().len(), 64);
    }
}
