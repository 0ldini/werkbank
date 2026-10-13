// Unter Windows ohne Konsolenfenster starten.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod core;

use crate::core::{fmt_version, App, Kind, Manifest, Version};
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let manifest = match core::load_manifest() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Fehler: {e:#}");
            std::process::exit(1);
        }
    };
    if args.is_empty() {
        gui(manifest);
    } else if let Err(e) = cli(&manifest, &args) {
        eprintln!("Fehler: {e:#}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------------------------
// Kommandozeile

fn cli(manifest: &Manifest, args: &[String]) -> anyhow::Result<()> {
    let find = |id: &str| {
        manifest
            .app
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| anyhow::anyhow!("unbekanntes Programm: {id}"))
    };
    let targets = |id: Option<&String>| -> anyhow::Result<Vec<&App>> {
        match id.map(String::as_str) {
            None | Some("all") | Some("alle") => Ok(manifest.app.iter().collect()),
            Some(id) => Ok(vec![find(id)?]),
        }
    };
    let quiet = |_: &str, _: f32| {};
    let show = |v: Option<Version>| v.map(fmt_version).unwrap_or_else(|| "–".into());
    match args[0].as_str() {
        "list" => {
            println!("{:<13} {:<11} {:<11} {}", "Programm", "Desktop", "Browser", "Neueste");
            for app in &manifest.app {
                let latest = core::latest_version(app).map(fmt_version).unwrap_or_else(|e| format!("? ({e:#})"));
                println!(
                    "{:<13} {:<11} {:<11} {}",
                    app.id,
                    show(core::installed_version(app, Kind::Native)),
                    show(core::installed_version(app, Kind::Web)),
                    latest
                );
            }
        }
        "install" | "update" => {
            let kind = if args.iter().any(|a| a == "--web") { Kind::Web } else { Kind::Native };
            let id = args.iter().skip(1).find(|a| !a.starts_with("--"));
            for app in targets(id)? {
                // "update" ohne Namen fasst nur an, was schon installiert ist.
                if args[0] == "update" && id.is_none() && core::installed_version(app, kind).is_none() {
                    continue;
                }
                let before = core::installed_version(app, kind);
                let v = core::install(app, kind, &quiet)?;
                match before {
                    Some(b) if b == v => println!("{}: {} ist aktuell", app.id, fmt_version(v)),
                    Some(b) => println!("{}: {} → {}", app.id, fmt_version(b), fmt_version(v)),
                    None => println!("{}: {} installiert", app.id, fmt_version(v)),
                }
            }
        }
        "install-file" => {
            let (id, file) = (args.get(1), args.get(2));
            let (Some(id), Some(file)) = (id, file) else { anyhow::bail!("Aufruf: install-file <programm> <archiv>") };
            let (kind, v) = core::install_file(find(id)?, std::path::Path::new(file), &quiet)?;
            println!("{id}: {} aus Datei installiert ({kind:?})", fmt_version(v));
        }
        "remove" => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("Aufruf: remove <programm>"))?;
            core::uninstall(find(id)?)?;
            println!("{id}: entfernt");
        }
        "run" => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("Aufruf: run <programm>"))?;
            core::run(find(id)?)?;
        }
        "web" => {
            let addr = core::serve_web(manifest.clone())?;
            let url = match args.iter().skip(1).find(|a| !a.starts_with("--")) {
                Some(id) => {
                    let app = find(id)?;
                    core::install(app, Kind::Web, &quiet)?;
                    format!("{addr}/{id}/")
                }
                None => addr,
            };
            println!("Browser-Versionen laufen unter {url}  (Beenden mit Strg+C)");
            if !args.iter().any(|a| a == "--no-open") {
                let _ = open::that(&url);
            }
            loop {
                std::thread::park();
            }
        }
        "dir" => println!("{}", core::data_dir().display()),
        _ => {
            println!(
                "Aufruf ohne Argumente öffnet die Oberfläche.\n\n\
                 list                          Programme und Versionen anzeigen\n\
                 install <programm|all> [--web]  installieren\n\
                 update [programm] [--web]     aktualisieren (ohne Namen: alle installierten)\n\
                 install-file <programm> <archiv>  selbst gebautes Archiv installieren\n\
                 run <programm>                starten\n\
                 web [programm]                Browser-Version(en) lokal bereitstellen\n\
                 remove <programm>             entfernen\n\
                 dir                           Datenordner anzeigen"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Oberfläche

#[derive(Default, Clone)]
struct Status {
    native: Option<Version>,
    web: Option<Version>,
    latest: Option<Version>,
    busy: Option<(String, f32)>,
    error: Option<String>,
}

type Shared = Arc<Mutex<HashMap<String, Status>>>;

struct Gui {
    manifest: Manifest,
    state: Shared,
    web_addr: Option<String>,
    notice: Option<String>,
}

fn gui(manifest: Manifest) {
    let title = manifest.title.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1060.0, 760.0])
            .with_min_inner_size([560.0, 400.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let app = Gui { manifest, state: Shared::default(), web_addr: None, notice: None };
            theme(&cc.egui_ctx);
            app.refresh(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        eprintln!("Oberfläche konnte nicht starten: {e}");
        std::process::exit(1);
    }
}

impl Gui {
    fn local(&self, app: &App) {
        let mut s = self.state.lock().unwrap();
        let st = s.entry(app.id.clone()).or_default();
        st.native = core::installed_version(app, Kind::Native);
        st.web = core::installed_version(app, Kind::Web);
    }

    /// Liest installierte Versionen und fragt im Hintergrund nach neuen.
    fn refresh(&self, ctx: &egui::Context) {
        for app in &self.manifest.app {
            self.local(app);
            let (app, state, ctx) = (app.clone(), self.state.clone(), ctx.clone());
            std::thread::spawn(move || {
                let latest = core::latest_version(&app);
                let mut s = state.lock().unwrap();
                let st = s.entry(app.id.clone()).or_default();
                match latest {
                    Ok(v) => st.latest = Some(v),
                    Err(e) => st.error = Some(format!("Versionsabfrage fehlgeschlagen: {e:#}")),
                }
                ctx.request_repaint();
            });
        }
    }

    /// Installiert/aktualisiert eine App im Hintergrund; `then_open` öffnet danach den Browser.
    fn install(&self, ctx: &egui::Context, app: &App, kind: Kind, then_open: Option<String>) {
        let (app, state, ctx) = (app.clone(), self.state.clone(), ctx.clone());
        {
            let mut s = state.lock().unwrap();
            let st = s.entry(app.id.clone()).or_default();
            st.busy = Some(("Starte …".into(), 0.0));
            st.error = None;
        }
        std::thread::spawn(move || {
            let progress = |msg: &str, frac: f32| {
                if let Some(st) = state.lock().unwrap().get_mut(&app.id) {
                    st.busy = Some((msg.to_string(), frac));
                }
                ctx.request_repaint();
            };
            let result = core::install(&app, kind, &progress);
            let mut s = state.lock().unwrap();
            let st = s.entry(app.id.clone()).or_default();
            st.busy = None;
            match result {
                Ok(v) => {
                    match kind {
                        Kind::Native => st.native = Some(v),
                        Kind::Web => st.web = Some(v),
                    }
                    st.latest = Some(v);
                    if let Some(url) = then_open {
                        let _ = open::that(url);
                    }
                }
                Err(e) => st.error = Some(format!("{e:#}")),
            }
            ctx.request_repaint();
        });
    }

    fn web_addr(&mut self) -> Option<String> {
        if self.web_addr.is_none() {
            match core::serve_web(self.manifest.clone()) {
                Ok(a) => self.web_addr = Some(a),
                Err(e) => self.notice = Some(format!("{e:#}")),
            }
        }
        self.web_addr.clone()
    }
}

// Farben der Werkbank (dunkler Grund, Bernstein als Akzent).
const BG: egui::Color32 = egui::Color32::from_rgb(0x17, 0x18, 0x1C);
const HEADER: egui::Color32 = egui::Color32::from_rgb(0x1D, 0x1F, 0x24);
const PANEL: egui::Color32 = egui::Color32::from_rgb(0x21, 0x23, 0x29);
const BORDER: egui::Color32 = egui::Color32::from_rgb(0x32, 0x35, 0x3D);
const OUTLINE: egui::Color32 = egui::Color32::from_rgb(0x45, 0x49, 0x53);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xEC, 0xEE, 0xF2);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0xA9, 0xAE, 0xB9);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0xF2, 0xB5, 0x44);
const WARN: egui::Color32 = egui::Color32::from_rgb(0xF0, 0x8A, 0x6A);

fn theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = BORDER;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 6.0);
    });
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// Wichtigste Aktion der Karte.
    Primary,
    /// Update: in der Akzentfarbe.
    Accent,
    Outline,
}

fn button(ui: &mut egui::Ui, label: &str, look: Look) -> egui::Response {
    let (fill, fg, stroke) = match look {
        Look::Primary => (TEXT, BG, TEXT),
        Look::Accent => (ACCENT, BG, ACCENT),
        Look::Outline => (egui::Color32::TRANSPARENT, TEXT, OUTLINE),
    };
    let text = egui::RichText::new(label).color(fg).size(14.5);
    let text = if look == Look::Outline { text } else { text.strong() };
    ui.add(
        egui::Button::new(text)
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, stroke))
            .corner_radius(9u8)
            .min_size(egui::vec2(0.0, 36.0)),
    )
}

/// Farbige Kachel mit dem Anfangsbuchstaben des Programms.
fn tile(ui: &mut egui::Ui, app: &App, size: f32) {
    let color = app.color.as_deref().and_then(|c| egui::Color32::from_hex(c).ok()).unwrap_or(OUTLINE);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(rect, (size * 0.25) as u8, color);
    let initial: String = app.name.chars().take(1).collect();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(size * 0.5),
        egui::Color32::WHITE,
    );
}

impl Gui {
    fn card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, app: &App, st: &Status) {
        let installed = if app.web_only { st.web } else { st.native };
        let update = installed.is_some() && st.latest.is_some() && installed < st.latest;
        let border = if update { ACCENT } else { BORDER };
        egui::Frame::new()
            .fill(PANEL)
            .stroke(egui::Stroke::new(1.0, border))
            .corner_radius(14u8)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    tile(ui, app, 52.0);
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(&app.name).size(19.0).strong());
                        ui.label(egui::RichText::new(&app.about).size(13.0).color(MUTED));
                    });
                });
                ui.add_space(4.0);
                let (line, color) = match (installed, st.latest) {
                    (Some(n), Some(l)) if update => {
                        (format!("Version {} · neu verfügbar: {}", fmt_version(n), fmt_version(l)), ACCENT)
                    }
                    (Some(n), _) => (format!("Version {} · aktuell", fmt_version(n)), MUTED),
                    (None, Some(l)) => (format!("Nicht installiert · verfügbar: {}", fmt_version(l)), MUTED),
                    (None, None) => ("Nicht installiert".to_string(), MUTED),
                };
                if let Some((msg, frac)) = &st.busy {
                    ui.label(egui::RichText::new(msg).size(13.0).color(MUTED));
                    ui.add(egui::ProgressBar::new(*frac).desired_height(6.0).fill(ACCENT));
                    ui.add_space(22.0);
                    return;
                }
                ui.label(egui::RichText::new(line).size(13.0).color(color));
                if let Some(e) = &st.error {
                    ui.label(egui::RichText::new(e).size(12.5).color(WARN));
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if app.web_only {
                        if button(ui, "Öffnen", Look::Primary).clicked() {
                            self.open_web(ctx, app);
                        }
                    } else {
                        if installed.is_some() {
                            if update && button(ui, "Aktualisieren", Look::Accent).clicked() {
                                self.install(ctx, app, Kind::Native, None);
                            }
                            let look = if update { Look::Outline } else { Look::Primary };
                            if button(ui, "Starten", look).clicked() {
                                if let Err(e) = core::run(app) {
                                    self.notice = Some(format!("{e:#}"));
                                }
                            }
                        } else if button(ui, "Installieren", Look::Primary).clicked() {
                            self.install(ctx, app, Kind::Native, None);
                        }
                        if button(ui, "Im Browser", Look::Outline)
                            .on_hover_text("Web-Version lokal laden und im Browser öffnen")
                            .clicked()
                        {
                            self.open_web(ctx, app);
                        }
                    }
                    if (st.native.is_some() || st.web.is_some())
                        && ui
                            .add(egui::Button::new(egui::RichText::new("Entfernen").size(12.5).color(MUTED)).frame(false))
                            .clicked()
                    {
                        if let Err(e) = core::uninstall(app) {
                            self.notice = Some(format!("{e:#}"));
                        }
                        self.local(app);
                    }
                });
            });
    }

    fn open_web(&mut self, ctx: &egui::Context, app: &App) {
        if let Some(addr) = self.web_addr() {
            // install() lädt nur, wenn die Web-Version fehlt oder veraltet ist.
            self.install(ctx, app, Kind::Web, Some(format!("{addr}/{}/", app.id)));
        }
    }
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = self.state.lock().unwrap().clone();
        let apps = self.manifest.app.clone();
        let status = |app: &App| snapshot.get(&app.id).cloned().unwrap_or_default();
        let installed = apps.iter().filter(|a| status(a).native.is_some() || status(a).web.is_some()).count();
        let outdated: Vec<App> = apps
            .iter()
            .filter(|a| {
                let st = status(a);
                !a.web_only && st.busy.is_none() && st.native.is_some() && st.latest.is_some() && st.native < st.latest
            })
            .cloned()
            .collect();

        egui::TopBottomPanel::top("kopf")
            .frame(egui::Frame::new().fill(HEADER).inner_margin(egui::Margin::symmetric(24, 14)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Marke: Bernstein-Kachel mit vier Feldern.
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 10u8, ACCENT);
                    for (i, (dx, dy)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].into_iter().enumerate() {
                        let min = rect.min + egui::vec2(8.0 + dx * 13.0, 8.0 + dy * 13.0);
                        let color = if i == 3 { BG.gamma_multiply(0.45) } else { BG };
                        ui.painter().rect_filled(egui::Rect::from_min_size(min, egui::vec2(11.0, 11.0)), 3u8, color);
                    }
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&self.manifest.title).size(22.0).strong());
                        let mut info = format!("{} Programme · {} installiert", apps.len(), installed);
                        if !outdated.is_empty() {
                            info += &format!(" · {} Update{}", outdated.len(), if outdated.len() == 1 { "" } else { "s" });
                        }
                        ui.label(egui::RichText::new(info).size(12.5).color(MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !outdated.is_empty()
                            && button(ui, &format!("Alle aktualisieren ({})", outdated.len()), Look::Accent).clicked()
                        {
                            for app in &outdated {
                                self.install(ctx, app, Kind::Native, None);
                            }
                        }
                        if button(ui, "Browser-Übersicht", Look::Outline).clicked() {
                            if let Some(a) = self.web_addr() {
                                let _ = open::that(a);
                            }
                        }
                        if button(ui, "Nach Updates suchen", Look::Outline).clicked() {
                            self.refresh(ctx);
                        }
                        if button(ui, "Datenordner", Look::Outline)
                            .on_hover_text("Hier liegen die installierten Programme")
                            .clicked()
                        {
                            let _ = open::that(core::data_dir());
                        }
                    });
                });
                if let Some(n) = &self.notice {
                    ui.label(egui::RichText::new(n).color(WARN));
                }
            });

        egui::TopBottomPanel::bottom("fuss")
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(24, 10)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Die Programme basieren auf den Open-Source-Apps des ArtCraft-Teams (MIT / Apache 2.0).")
                            .size(12.0)
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(concat!("Werkbank ", env!("CARGO_PKG_VERSION"))).size(12.0).color(MUTED),
                        );
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(24, 18)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    let columns = (((ui.available_width() + 14.0) / 330.0).floor() as usize).clamp(1, 4);
                    ui.columns(columns, |cols| {
                        for (i, app) in apps.iter().enumerate() {
                            let ui = &mut cols[i % columns];
                            self.card(ui, ctx, app, &status(app));
                            ui.add_space(6.0);
                        }
                    });
                });
            });
    }
}
