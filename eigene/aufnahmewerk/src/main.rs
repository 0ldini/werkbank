#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor;
mod frame;
mod model;
mod record;
mod schnapp;
mod shot;
mod sprache;
mod stil;
mod thema;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(thema::fenster_icon())
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([980.0, 640.0])
            .with_title("Aufnahmewerk"),
        ..Default::default()
    };
    eframe::run_native(
        "Aufnahmewerk",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
