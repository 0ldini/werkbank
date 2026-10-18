#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dialogs;
mod ftp;
mod listparse;
mod model;
mod remote;
mod sftp;
mod sprache;
mod store;
mod tls;
mod xfer;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true)
            .with_title("FTPwerk"),
        ..Default::default()
    };
    eframe::run_native("FTPwerk", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
