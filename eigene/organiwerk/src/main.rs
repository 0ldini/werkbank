#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod app;
mod json;
mod model;
mod save;
mod sprache;
mod thema;

use app::App;

// ---------------- Nativ ----------------

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(thema::fenster_icon())
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([360.0, 480.0])
            .with_title("Organiwerk"),
        ..Default::default()
    };
    eframe::run_native("Organiwerk", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

// ---------------- Browser (WebAssembly) ----------------

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;
    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().expect("kein window").document().expect("kein document");
        let canvas = document
            .get_element_by_id("organiwerk_canvas")
            .expect("Canvas organiwerk_canvas fehlt")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("kein Canvas-Element");
        eframe::WebRunner::new()
            .start(canvas, web_options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
            .await
            .expect("Start fehlgeschlagen");
    });
}
