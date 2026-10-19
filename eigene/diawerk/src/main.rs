#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod frames;
mod model;
mod render;
mod sicht;
mod sprache;
mod thema;
mod tool;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(thema::fenster_icon())
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([980.0, 620.0])
            .with_title("Diawerk"),
        ..Default::default()
    };
    eframe::run_native(
        "Diawerk",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
