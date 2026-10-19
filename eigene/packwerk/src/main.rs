#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anzeige;
mod app;
mod archiv;
mod regeln;
mod sprache;
mod thema;

fn main() -> eframe::Result<()> {
    // `packwerk <archiv>`, `packwerk --entpacken <archiv> [ziel]`, `packwerk datei …`
    let start = regeln::start_lesen(std::env::args_os().skip(1).collect());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(thema::fenster_icon())
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true)
            .with_app_id("packwerk")
            .with_title("Packwerk"),
        ..Default::default()
    };
    eframe::run_native(
        "Packwerk",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, start)))),
    )
}
