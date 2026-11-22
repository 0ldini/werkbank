#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dialogs;
#[cfg(test)]
mod e2e;
mod ftp;
mod listparse;
mod model;
mod remote;
#[cfg(feature = "sftp")]
mod sftp;
mod sprache;
mod stil;
mod store;
mod thema;
#[cfg(feature = "tls")]
mod tls;
mod xfer;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(thema::fenster_icon())
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true)
            .with_title("FTPwerk"),
        ..Default::default()
    };
    stil::starten("FTPwerk", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
