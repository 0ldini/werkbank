#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod app;
mod b64;
mod backend;
mod demo;
mod imap;
mod jmap;
mod jmap_proto;
mod json;
mod mime;
#[cfg(not(target_arch = "wasm32"))]
mod native;
mod presets;
mod role;
mod save;
mod smtp;
mod sprache;
mod store;
mod textutil;
mod timeutil;
mod types;

use app::App;

// ---------------- Nativ ----------------

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 740.0])
            .with_min_inner_size([360.0, 480.0])
            .with_title("Mailwerk"),
        ..Default::default()
    };
    eframe::run_native("Mailwerk", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

// ---------------- Browser (WebAssembly) ----------------

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;
    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().expect("kein window").document().expect("kein document");
        let canvas = document
            .get_element_by_id("mailwerk_canvas")
            .expect("Canvas mailwerk_canvas fehlt")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("kein Canvas-Element");
        eframe::WebRunner::new()
            .start(canvas, web_options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
            .await
            .expect("Start fehlgeschlagen");
    });
}
