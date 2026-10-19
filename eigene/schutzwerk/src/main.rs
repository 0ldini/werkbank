#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod netparse;
mod pwcheck;
mod recycle;
mod sprache;
mod stil;
mod sys;
mod thema;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(thema::fenster_icon())
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([920.0, 600.0])
            .with_title("Schutzwerk"),
        ..Default::default()
    };
    eframe::run_native(
        "Schutzwerk",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
