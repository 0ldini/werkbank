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
                let latest = core::latest_version(app).map(fmt_version).unwrap_or_else(|e| format!("? ({e})"));
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
            let url = match args.get(1) {
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
            .with_inner_size([860.0, 680.0])
            .with_min_inner_size([560.0, 400.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let app = Gui { manifest, state: Shared::default(), web_addr: None, notice: None };
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

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = self.state.lock().unwrap().clone();
        let outdated: Vec<App> = self
            .manifest
            .app
            .iter()
            .filter(|a| {
                let st = snapshot.get(&a.id).cloned().unwrap_or_default();
                st.busy.is_none() && st.native.is_some() && st.latest.is_some() && st.native < st.latest
            })
            .cloned()
            .collect();

        egui::TopBottomPanel::top("kopf").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(&self.manifest.title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Datenordner").on_hover_text("Hier liegen die Programme und apps.toml").clicked() {
                        let _ = open::that(core::data_dir());
                    }
                    if ui.button("Browser-Übersicht").clicked() {
                        if let Some(a) = self.web_addr() {
                            let _ = open::that(a);
                        }
                    }
                    if ui.button("Nach Updates suchen").clicked() {
                        self.refresh(ctx);
                    }
                    if !outdated.is_empty()
                        && ui.button(format!("Alle aktualisieren ({})", outdated.len())).clicked()
                    {
                        for app in &outdated {
                            self.install(ctx, app, Kind::Native, None);
                        }
                    }
                });
            });
            if let Some(n) = &self.notice {
                ui.colored_label(egui::Color32::from_rgb(230, 120, 90), n);
            }
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for app in self.manifest.app.clone() {
                    let st = snapshot.get(&app.id).cloned().unwrap_or_default();
                    egui::Frame::group(ui.style()).inner_margin(12.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&app.name).size(18.0).strong());
                                ui.label(egui::RichText::new(&app.about).weak());
                                let update = st.native.is_some() && st.latest.is_some() && st.native < st.latest;
                                let line = match (st.native, st.latest) {
                                    (Some(n), Some(l)) if update => {
                                        format!("Installiert {} · neu verfügbar {}", fmt_version(n), fmt_version(l))
                                    }
                                    (Some(n), _) => format!("Installiert {} · aktuell", fmt_version(n)),
                                    (None, Some(l)) => format!("Nicht installiert · verfügbar {}", fmt_version(l)),
                                    (None, None) => "Nicht installiert".into(),
                                };
                                let color = if update {
                                    egui::Color32::from_rgb(235, 170, 60)
                                } else {
                                    ui.visuals().weak_text_color()
                                };
                                ui.colored_label(color, line);
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if st.busy.is_some() {
                                    ui.spinner();
                                    return;
                                }
                                let update = st.native.is_some() && st.latest.is_some() && st.native < st.latest;
                                if app.web_only {
                                    // Eigene HTML5-Apps: laufen im Browser, Knopf «Im Browser» folgt unten.
                                } else if st.native.is_some() {
                                    if ui.button(egui::RichText::new("  Starten  ").size(16.0)).clicked() {
                                        if let Err(e) = core::run(&app) {
                                            self.notice = Some(format!("{e:#}"));
                                        }
                                    }
                                    if update && ui.button("Aktualisieren").clicked() {
                                        self.install(ctx, &app, Kind::Native, None);
                                    }
                                } else if ui.button(egui::RichText::new("  Installieren  ").size(16.0)).clicked() {
                                    self.install(ctx, &app, Kind::Native, None);
                                }
                                if ui
                                    .button(if app.web_only { "  Öffnen  " } else { "Im Browser" })
                                    .on_hover_text("Web-Version lokal laden und im Browser öffnen")
                                    .clicked()
                                {
                                    if let Some(addr) = self.web_addr() {
                                        // install() lädt nur, wenn die Web-Version fehlt oder veraltet ist.
                                        self.install(ctx, &app, Kind::Web, Some(format!("{addr}/{}/", app.id)));
                                    }
                                }
                                if st.native.is_some() || st.web.is_some() {
                                    if ui.small_button("Entfernen").clicked() {
                                        if let Err(e) = core::uninstall(&app) {
                                            self.notice = Some(format!("{e:#}"));
                                        }
                                        self.local(&app);
                                    }
                                }
                            });
                        });
                        if let Some((msg, frac)) = &st.busy {
                            ui.add(egui::ProgressBar::new(*frac).text(msg.clone()));
                        }
                        if let Some(e) = &st.error {
                            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        });
    }
}
