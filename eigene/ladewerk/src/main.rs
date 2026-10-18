#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clip;
mod engine;
mod links;
mod net;
mod plan;
mod sprache;
mod store;
mod thema;
mod unpack;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([860.0, 540.0])
            .with_title("Ladewerk"),
        ..Default::default()
    };
    eframe::run_native(
        "Ladewerk",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
