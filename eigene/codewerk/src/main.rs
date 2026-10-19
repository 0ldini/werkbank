#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod qr;
mod save;
mod sprache;
mod stil;
mod thema;

use app::App;
#[cfg(not(target_arch = "wasm32"))]
use sprache::{t, tf};

// ---------------- Nativ ----------------

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a.starts_with("--")) {
        cli(&args);
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(thema::fenster_icon())
            .with_inner_size([900.0, 580.0])
            .with_min_inner_size([680.0, 460.0])
            .with_title(t("Codewerk – QR-Codes")),
        ..Default::default()
    };
    eframe::run_native(
        "Codewerk",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

/// Kommandozeile: codewerk --text "…" --out datei.png [--scale N --ec L|M|Q|H --fg #hex --bg #hex]
#[cfg(not(target_arch = "wasm32"))]
fn cli(args: &[String]) {
    let mut text = String::new();
    let mut out = String::new();
    let mut st = qr::Style::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut next = || {
            i += 1;
            args.get(i).cloned().unwrap_or_default()
        };
        match a {
            "--help" | "-h" => {
                println!("{}", t("Codewerk – QR-Codes\n\nVerwendung:\n  codewerk --text \"Inhalt\" --out datei.png\n\nOptionen:\n  --text <s>   Inhalt (Text oder Link)\n  --out <f>    Zieldatei (.png oder .svg)\n  --scale <n>  Pixel je Modul (Standard 10)\n  --quiet <n>  Rand in Modulen (Standard 4)\n  --ec <L|M|Q|H>  Fehlerkorrektur (Standard M)\n  --fg <#hex>  Vordergrundfarbe\n  --bg <#hex>  Hintergrundfarbe\n\nOhne --Optionen startet die grafische Oberfläche."));
                return;
            }
            "--text" => text = next(),
            "--out" => out = next(),
            "--scale" => st.scale = next().parse().unwrap_or(st.scale),
            "--quiet" => st.quiet = next().parse().unwrap_or(st.quiet),
            "--ec" => {
                st.ec = match next().to_uppercase().as_str() {
                    "L" => qr::Ec::L,
                    "Q" => qr::Ec::Q,
                    "H" => qr::Ec::H,
                    _ => qr::Ec::M,
                }
            }
            "--fg" => st.fg = parse_hex(&next()).unwrap_or(st.fg),
            "--bg" => st.bg = parse_hex(&next()).unwrap_or(st.bg),
            _ => {}
        }
        i += 1;
    }
    if text.is_empty() || out.is_empty() {
        eprintln!("{}", t("Fehlt: --text und --out (siehe --help)"));
        std::process::exit(2);
    }
    let res = if out.to_lowercase().ends_with(".svg") {
        qr::to_svg(&text, &st).map(|s| s.into_bytes())
    } else {
        qr::to_png(&text, &st)
    };
    match res {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&out, bytes) {
                eprintln!("{}", tf("Schreiben fehlgeschlagen: {e}", &[("e", &e)]));
                std::process::exit(1);
            }
            println!("{}", tf("Geschrieben: {out}", &[("out", &out)]));
        }
        Err(e) => {
            eprintln!("{}", tf("Fehler: {e}", &[("e", &e)]));
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b])
}

// ---------------- Browser (WebAssembly) ----------------

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;
    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("kein window")
            .document()
            .expect("kein document");
        let canvas = document
            .get_element_by_id("codewerk_canvas")
            .expect("Canvas codewerk_canvas fehlt")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("kein Canvas-Element");
        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(App::new(cc)))),
            )
            .await
            .expect("Start fehlgeschlagen");
    });
}
