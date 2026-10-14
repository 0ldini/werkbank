// Unter Windows ohne Konsolenfenster starten.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assoc;
mod core;
mod icons;

use crate::core::{fmt_version, App, Kind, Manifest, Version};
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Erhöhte Instanz (siehe core::create_dir_elevated): nur den Ordner anlegen, dann beenden.
    if args.first().map(String::as_str) == Some("--mkdir-elevated") {
        let (Some(dir), Some(user)) = (args.get(1), args.get(2)) else { std::process::exit(2) };
        if let Err(e) = core::mkdir_elevated(std::path::Path::new(dir), user) {
            eprintln!("Fehler: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    // Von Windows aufgerufen («Öffnen mit»): Datei im zugeordneten Programm öffnen.
    if args.first().map(String::as_str) == Some("--open") {
        let (Some(id), Some(file)) = (args.get(1), args.get(2)) else { std::process::exit(2) };
        let manifest = core::load_manifest().unwrap_or_else(|_| Manifest { title: "Werkbank".into(), app: Vec::new() });
        if let Some(app) = manifest.app.iter().find(|a| &a.id == id) {
            if core::run_with(app, Some(std::path::Path::new(file))).is_ok() {
                return;
            }
            let name = app.name.clone();
            gui(move || manifest.clone(), None, true, Some(format!("{name} ist nicht installiert. Zum Öffnen von {file} zuerst installieren.")));
            return;
        }
        std::process::exit(1);
    }
    let load = || match core::load_manifest() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Fehler: {e:#}");
            std::process::exit(1);
        }
    };
    // Offline-Setup: ein Ordner `bundle` neben dem Launcher (oder --bundle DIR) wird beim
    // Start eingerichtet; --offline verzichtet auf die Suche nach neueren Versionen.
    let bundle = args
        .iter()
        .position(|a| a == "--bundle")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let dir = std::env::current_exe().ok()?.parent()?.join("bundle");
            dir.is_dir().then_some(dir)
        });
    let online = !args.iter().any(|a| a == "--offline");
    // Übrige Argumente ohne Optionen: leer → Oberfläche, sonst Kommandozeile.
    let mut plain = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--bundle" {
            it.next();
        } else if !a.starts_with("--") {
            plain.push(a.clone());
        }
    }
    if plain.is_empty() {
        gui(load, bundle, online, None);
    } else if let Err(e) = cli(&load(), &args) {
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
        "install-bundle" => {
            let dir = args.iter().skip(1).find(|a| !a.starts_with("--")).ok_or_else(|| anyhow::anyhow!("Aufruf: install-bundle <ordner> [--offline]"))?;
            let online = !args.iter().any(|a| a == "--offline");
            core::install_bundle(
                manifest,
                std::path::Path::new(dir),
                online,
                &|_, _, _| {},
                &|app, kind, r| match r {
                    Ok(v) => println!("{}: {} eingerichtet ({kind:?})", app.id, fmt_version(v)),
                    Err(e) => println!("{}: FEHLER {e:#}", app.id),
                },
            );
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
                 install-file <programm> <archiv>  selbst gebautes Archiv installieren
                 install-bundle <ordner> [--offline]  Archive eines Offline-Setups einrichten\n\
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

/// Ersteinrichtung: Datenordner wählen, solange keiner feststeht.
struct Setup {
    path: String,
    error: Option<String>,
    load: Box<dyn Fn() -> Manifest>,
    bundle: Option<std::path::PathBuf>,
    online: bool,
}

/// Einstellungen → Dateitypen: die Auswahl, solange die Seite offen ist.
struct Settings {
    choice: assoc::Choice,
    message: Option<(String, bool)>,
    /// Teil der Ersteinrichtung (dann «Weiter» statt «Zurück»).
    first_run: bool,
}

struct Gui {
    setup: Option<Setup>,
    settings: Option<Settings>,
    manifest: Manifest,
    textures: HashMap<String, egui::TextureHandle>,
    state: Shared,
    web_addr: Option<String>,
    notice: Option<String>,
}

fn gui(load: impl Fn() -> Manifest + 'static, bundle: Option<std::path::PathBuf>, online: bool, notice: Option<String>) {
    let title = "Werkbank".to_string();
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1060.0, 760.0]).with_min_inner_size([560.0, 400.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(icons::WERKBANK) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            theme(&cc.egui_ctx);
            let empty = Manifest { title: "Werkbank".into(), app: Vec::new() };
            let mut app = Gui {
                setup: None,
                settings: None,
                manifest: empty,
                textures: HashMap::new(),
                state: Shared::default(),
                web_addr: None,
                notice,
            };
            if core::configured_data_dir().is_some() {
                app.begin(&cc.egui_ctx, load(), bundle, online);
            } else {
                app.setup = Some(Setup {
                    path: core::default_data_dir().display().to_string(),
                    error: None,
                    load: Box::new(load),
                    bundle,
                    online,
                });
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        eprintln!("Oberfläche konnte nicht starten: {e}");
        std::process::exit(1);
    }
}

impl Gui {
    /// Nach der Ersteinrichtung (oder sofort, wenn der Datenordner feststeht): Liste laden, los.
    fn begin(&mut self, ctx: &egui::Context, manifest: Manifest, bundle: Option<std::path::PathBuf>, online: bool) {
        self.manifest = manifest;
        self.refresh(ctx);
        if let Some(dir) = bundle {
            self.start_bundle(ctx, dir, online);
        }
        // Schritt 2 der Ersteinrichtung: Dateitypen (nur, wo das System es kann).
        if assoc::supported() && !core::load_config().types_asked {
            self.settings = Some(Settings { choice: assoc::all(&self.manifest), message: None, first_run: true });
        }
    }

    fn open_settings(&mut self) {
        self.settings = Some(Settings { choice: core::load_config().types, message: None, first_run: false });
    }

    fn settings_screen(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut apply = false;
        let Some(settings) = self.settings.as_mut() else { return };
        let apps: Vec<App> = self.manifest.app.iter().filter(|a| !a.types.is_empty()).cloned().collect();
        egui::TopBottomPanel::top("einst-kopf")
            .frame(egui::Frame::new().fill(HEADER).inner_margin(egui::Margin::symmetric(24, 14)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let title = if settings.first_run { "Einrichtung: Dateitypen" } else { "Einstellungen: Dateitypen" };
                        ui.label(egui::RichText::new(title).size(22.0).strong());
                        ui.label(
                            egui::RichText::new("Angehakte Dateitypen lassen sich per Rechtsklick → «Öffnen mit» in der Werkbank öffnen.")
                                .size(12.5)
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if settings.first_run { "Weiter" } else { "Übernehmen" };
                        if button(ui, label, Look::Accent).clicked() {
                            apply = true;
                        }
                        if !settings.first_run && button(ui, "Zurück", Look::Outline).clicked() {
                            close = true;
                        }
                        if settings.first_run && button(ui, "Überspringen", Look::Outline).clicked() {
                            settings.choice.clear();
                            apply = true;
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("einst-fuss")
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(24, 10)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Welches Programm Standard ist, legt Windows fest: Rechtsklick → «Öffnen mit» → «Immer», oder hier:")
                            .size(12.5)
                            .color(MUTED),
                    );
                    if ui.add(egui::Button::new(egui::RichText::new("Standard-Apps in Windows öffnen").size(12.5)).frame(false)).clicked() {
                        assoc::open_system_settings();
                    }
                });
                if let Some((m, ok)) = &settings.message {
                    ui.label(egui::RichText::new(m).color(if *ok { ACCENT } else { WARN }));
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(24, 18)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for app in &apps {
                        let exts = assoc::offered(app);
                        let chosen = settings.choice.entry(app.id.clone()).or_default();
                        egui::Frame::new()
                            .fill(PANEL)
                            .stroke(egui::Stroke::new(1.0, BORDER))
                            .corner_radius(12u8)
                            .inner_margin(14.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    tile(ui, &mut self.textures, app, 36.0);
                                    ui.label(egui::RichText::new(&app.name).size(17.0).strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.small_button("Keine").clicked() {
                                            chosen.clear();
                                        }
                                        if ui.small_button("Alle").clicked() {
                                            *chosen = exts.clone();
                                        }
                                    });
                                });
                                ui.horizontal_wrapped(|ui| {
                                    for ext in &exts {
                                        let mut on = chosen.contains(ext);
                                        if ui.checkbox(&mut on, ext).changed() {
                                            if on {
                                                chosen.push(ext.clone());
                                            } else {
                                                chosen.retain(|e| e != ext);
                                            }
                                        }
                                    }
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            });
        if apply {
            let mut cfg = core::load_config();
            let wanted: assoc::Choice = settings.choice.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
            match assoc::apply(&self.manifest, &cfg.types, &wanted) {
                Ok(()) => {
                    cfg.types = wanted;
                    cfg.types_asked = true;
                    let _ = core::save_config(&cfg);
                    close = true;
                }
                Err(e) => settings.message = Some((format!("{e:#}"), false)),
            }
        }
        if close {
            self.settings = None;
        }
    }

    fn setup_screen(&mut self, ctx: &egui::Context) {
        let mut confirm = false;
        let mut browse = false;
        let Some(setup) = self.setup.as_mut() else { return };
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(48, 40)))
            .show(ctx, |ui| {
                ui.vertical_centered_justified(|ui| {
                    ui.set_max_width(640.0);
                    ui.horizontal(|ui| {
                        if let Some(t) = texture(ctx, &mut self.textures, "werkbank", icons::WERKBANK) {
                            ui.add(egui::Image::new(&t).fit_to_exact_size(egui::vec2(56.0, 56.0)));
                        }
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("Willkommen in der Werkbank").size(24.0).strong());
                            ui.label(egui::RichText::new("Einmalige Einrichtung").size(13.0).color(MUTED));
                        });
                    });
                    ui.add_space(24.0);
                    ui.label(egui::RichText::new("Wo sollen die Programme abgelegt werden?").size(16.0).strong());
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "Der Ordner wird angelegt, falls er fehlt. Unter C:\\Program Files fragt Windows einmalig nach \
                             Administratorrechten; danach braucht die Werkbank keine mehr.",
                        )
                        .size(13.0)
                        .color(MUTED),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let w = ui.available_width() - 140.0;
                        ui.add_sized([w, 36.0], egui::TextEdit::singleline(&mut setup.path).font(egui::FontId::proportional(15.0)));
                        if button(ui, "Durchsuchen …", Look::Outline).clicked() {
                            browse = true;
                        }
                    });
                    if let Some(e) = &setup.error {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(e).color(WARN));
                    }
                    ui.add_space(18.0);
                    ui.horizontal(|ui| {
                        if button(ui, "Weiter", Look::Accent).clicked() {
                            confirm = true;
                        }
                        if button(ui, "Vorschlag", Look::Outline).clicked() {
                            setup.path = core::default_data_dir().display().to_string();
                        }
                    });
                });
            });
        if browse {
            if let Some(dir) = rfd::FileDialog::new().set_title("Datenordner wählen").pick_folder() {
                setup.path = dir.display().to_string();
            }
        }
        if confirm {
            let path = std::path::PathBuf::from(setup.path.trim());
            match core::set_data_dir(&path) {
                Ok(()) => {
                    let setup = self.setup.take().unwrap();
                    let manifest = (setup.load)();
                    self.begin(ctx, manifest, setup.bundle, setup.online);
                }
                Err(e) => setup.error = Some(format!("{e:#}")),
            }
        }
    }

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
                    Err(e) => {
                        let text = format!("{e:#}");
                        // Kein Release für dieses Programm ist kein Fehler, nur ein Zustand.
                        let missing = ["kein Release", "status: 401", "status: 404"].iter().any(|m| text.contains(m));
                        st.error = Some(if missing {
                            "Noch nicht veröffentlicht".to_string()
                        } else {
                            format!("Versionsabfrage fehlgeschlagen: {text}")
                        });
                    }
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

/// Lädt ein eingebautes PNG als Textur (einmal pro Programm).
fn texture(ctx: &egui::Context, cache: &mut HashMap<String, egui::TextureHandle>, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    if let Some(t) = cache.get(name) {
        return Some(t.clone());
    }
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
    let t = ctx.load_texture(name, color, egui::TextureOptions::LINEAR);
    cache.insert(name.to_string(), t.clone());
    Some(t)
}

/// Programm-Icon; ohne eingebautes Icon eine farbige Kachel mit dem Anfangsbuchstaben.
fn tile(ui: &mut egui::Ui, cache: &mut HashMap<String, egui::TextureHandle>, app: &App, size: f32) {
    let icon = app.icon.as_deref().unwrap_or(&app.id);
    if let Some(bytes) = icons::ICONS.iter().find(|(n, _)| *n == icon).map(|(_, b)| *b) {
        if let Some(t) = texture(ui.ctx(), cache, icon, bytes) {
            ui.add(egui::Image::new(&t).fit_to_exact_size(egui::vec2(size, size)));
            return;
        }
    }
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
                    tile(ui, &mut self.textures, app, 52.0);
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

    /// Offline-Setup: richtet die Archive aus `dir` nacheinander im Hintergrund ein.
    fn start_bundle(&mut self, ctx: &egui::Context, dir: std::path::PathBuf, online: bool) {
        let entries = core::bundle_entries(&self.manifest, &dir);
        if entries.is_empty() {
            return;
        }
        self.notice = Some(format!(
            "Einrichtung: {} Programm{} aus dem Setup{}",
            entries.len(),
            if entries.len() == 1 { "" } else { "e" },
            if online { ", neuere Versionen werden aus dem Netz geholt" } else { "" }
        ));
        {
            let mut s = self.state.lock().unwrap();
            for e in &entries {
                s.entry(e.app.id.clone()).or_default().busy = Some(("Wartet …".into(), 0.0));
            }
        }
        let (manifest, state, ctx) = (self.manifest.clone(), self.state.clone(), ctx.clone());
        std::thread::spawn(move || {
            let report = |id: &str, msg: &str, frac: f32| {
                if let Some(st) = state.lock().unwrap().get_mut(id) {
                    st.busy = Some((msg.to_string(), frac));
                }
                ctx.request_repaint();
            };
            let done = |app: &App, kind: Kind, r: anyhow::Result<Version>| {
                let mut s = state.lock().unwrap();
                let st = s.entry(app.id.clone()).or_default();
                st.busy = None;
                match r {
                    Ok(v) => {
                        match kind {
                            Kind::Native => st.native = Some(v),
                            Kind::Web => st.web = Some(v),
                        }
                        if st.latest.is_none_or(|l| l < v) {
                            st.latest = Some(v);
                        }
                    }
                    Err(e) => st.error = Some(format!("{e:#}")),
                }
                ctx.request_repaint();
            };
            core::install_bundle(&manifest, &dir, online, &report, &done);
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
        if self.setup.is_some() {
            self.setup_screen(ctx);
            return;
        }
        if self.settings.is_some() {
            self.settings_screen(ctx);
            return;
        }
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
                    if let Some(t) = texture(ctx, &mut self.textures, "werkbank", icons::WERKBANK) {
                        ui.add(egui::Image::new(&t).fit_to_exact_size(egui::vec2(40.0, 40.0)));
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
                        if assoc::supported() && button(ui, "Dateitypen", Look::Outline).clicked() {
                            self.open_settings();
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
