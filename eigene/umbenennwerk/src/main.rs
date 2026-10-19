#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod lizenz;
mod app;
mod datefmt;
mod rename;
mod sprache;
mod thema;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 660.0])
            .with_min_inner_size([820.0, 520.0])
            .with_title("Umbenennwerk"),
        ..Default::default()
    };
    if !lizenz::ok() {
        return lizenz::sperre("Umbenennwerk");
    }
    eframe::run_native(
        "Umbenennwerk",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
