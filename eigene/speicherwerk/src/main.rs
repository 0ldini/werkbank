#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod scan;
mod sprache;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 680.0])
            .with_min_inner_size([860.0, 540.0])
            .with_title("Speicherwerk"),
        ..Default::default()
    };
    eframe::run_native(
        "Speicherwerk",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
