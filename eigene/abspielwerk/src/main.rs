#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bild;
mod medien;
mod sprache;
mod stil;
mod thema;
mod vlc;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // Erste Datei oder erster Ordner auf der Befehlszeile (vom Launcher oder «Öffnen mit»).
    // macOS hängt beim Start aus dem Finder manchmal «-psn_…» an; nur Vorhandenes zählt.
    let start = std::env::args_os().skip(1).map(PathBuf::from).find(|p| p.exists());
    // Vor dem Start der Oberfläche (noch ohne weitere Threads): libVLC suchen und den
    // Pfad zu den Plugins setzen.
    let kandidaten = vlc::vorbereiten();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(thema::fenster_icon())
            .with_inner_size([1180.0, 740.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title("Abspielwerk")
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("Abspielwerk", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, start, kandidaten)))))
}
