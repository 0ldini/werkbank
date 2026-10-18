//! Oberfläche: Startseite (Uhr, Wetter, zuletzt geöffnet, installierte Programme), Menü,
//! «Programme entdecken» zum Installieren und Aktualisieren, Einstellungen.

use crate::core::{self, fmt_version, App, Kind, Manifest, Place, Recent, Version};
use crate::weather::{self, Weather};
use crate::sprache::{self, t, tf, Lang};
use crate::{assoc, icons};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default, Clone)]
struct Status {
    native: Option<Version>,
    web: Option<Version>,
    latest: Option<Version>,
    busy: Option<(String, f32)>,
    error: Option<String>,
}

impl Status {
    fn installed(&self) -> bool {
        self.native.is_some() || self.web.is_some()
    }
    /// Installierte Version, nach der sich ein Update richtet.
    fn current(&self) -> Option<Version> {
        self.native.or(self.web)
    }
    fn update(&self) -> bool {
        self.busy.is_none() && self.current().is_some() && self.latest.is_some() && self.current() < self.latest
    }
    fn update_kind(&self) -> Kind {
        if self.native.is_some() {
            Kind::Native
        } else {
            Kind::Web
        }
    }
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
struct Types {
    choice: assoc::Choice,
    message: Option<(String, bool)>,
    /// Teil der Ersteinrichtung (dann «Weiter» statt «Zurück»).
    first_run: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Home,
    Catalog,
    Prefs,
}

#[derive(Clone, Copy, PartialEq)]
enum Chip {
    All,
    Office,
    Photo,
    Video,
    System,
    Installed,
    Updates,
}

impl Chip {
    fn label(self) -> &'static str {
        t(match self {
            Chip::All => "Alle",
            Chip::Office => "Büro",
            Chip::Photo => "Foto und Grafik",
            Chip::Video => "Video und Ton",
            Chip::System => "System und Werkzeuge",
            Chip::Installed => "Installiert",
            Chip::Updates => "Updates",
        })
    }
}

/// Rubriken in der Reihenfolge der Anzeige.
const CATEGORIES: [Chip; 4] = [Chip::Office, Chip::Photo, Chip::Video, Chip::System];

/// Rubrik eines Programms: aus der Programmliste (`category`), sonst eingebaut.
fn category(app: &App) -> Chip {
    match app.category.as_deref() {
        Some("buero") => return Chip::Office,
        Some("foto") => return Chip::Photo,
        Some("video") => return Chip::Video,
        Some("system") => return Chip::System,
        _ => {}
    }
    match app.id.as_str() {
        "pdfcraft" | "wordcraft" | "gridcraft" | "deckcraft" | "mailwerk" => Chip::Office,
        "photocraft" | "vectorcraft" | "designcraft" | "lightcraft" | "cadcraft" => Chip::Photo,
        "filmcraft" | "effectcraft" | "soundcraft" | "diawerk" | "aufnahmewerk" | "mediathekwerk" => Chip::Video,
        _ => Chip::System,
    }
}

#[derive(Default)]
struct Sky {
    data: Option<Weather>,
    error: Option<String>,
    busy: bool,
}

#[derive(Default)]
struct PlaceSearch {
    query: String,
    busy: bool,
    result: Option<Result<Vec<Place>, String>>,
}

pub struct Gui {
    setup: Option<Setup>,
    types: Option<Types>,
    manifest: Manifest,
    textures: HashMap<String, egui::TextureHandle>,
    state: Shared,
    web_addr: Option<String>,
    notice: Option<String>,
    view: View,
    menu: bool,
    about: bool,
    chip: Chip,
    search: String,
    cfg: core::Config,
    sky: Arc<Mutex<Sky>>,
    sky_at: Option<Instant>,
    cfg_at: Instant,
    places: Arc<Mutex<PlaceSearch>>,
}

pub fn run(load: impl Fn() -> Manifest + 'static, bundle: Option<std::path::PathBuf>, online: bool, notice: Option<String>) {
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1120.0, 780.0]).with_min_inner_size([520.0, 420.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(icons::WERKBANK) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(
        "Werkbank",
        options,
        Box::new(move |cc| {
            theme(&cc.egui_ctx);
            let cfg = core::load_config();
            let mut sky = Sky::default();
            let place = cfg.place.clone().unwrap_or_else(Place::sarnen);
            sky.data = weather::cached().filter(|w| w.place == place.name);
            let mut app = Gui {
                setup: None,
                types: None,
                manifest: Manifest { title: "Werkbank".into(), app: Vec::new(), server: None },
                textures: HashMap::new(),
                state: Shared::default(),
                web_addr: None,
                notice,
                view: View::Home,
                menu: false,
                about: false,
                chip: Chip::All,
                search: String::new(),
                cfg,
                sky: Arc::new(Mutex::new(sky)),
                sky_at: None,
                cfg_at: Instant::now(),
                places: Arc::default(),
            };
            // Nur für Bildschirmfotos der Dokumentation: Ansicht direkt öffnen.
            match std::env::var("WERKBANK_ANSICHT").as_deref() {
                Ok("menue") => app.menu = true,
                Ok("katalog") => app.view = View::Catalog,
                Ok("updates") => (app.view, app.chip) = (View::Catalog, Chip::Updates),
                Ok("einstellungen") => app.view = View::Prefs,
                _ => {}
            }
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

// ---------------------------------------------------------------------------------------------
// Farben (dunkler Grund, Bernstein als Akzent)

const BG: Color32 = Color32::from_rgb(0x17, 0x18, 0x1C);
const HEADER: Color32 = Color32::from_rgb(0x1D, 0x1F, 0x24);
const PANEL: Color32 = Color32::from_rgb(0x21, 0x23, 0x29);
const HOVER: Color32 = Color32::from_rgb(0x2A, 0x2D, 0x35);
const CHIP: Color32 = Color32::from_rgb(0x34, 0x38, 0x42);
const BORDER: Color32 = Color32::from_rgb(0x32, 0x35, 0x3D);
const OUTLINE: Color32 = Color32::from_rgb(0x45, 0x49, 0x53);
const TEXT: Color32 = Color32::from_rgb(0xEC, 0xEE, 0xF2);
const SOFT: Color32 = Color32::from_rgb(0xD6, 0xD9, 0xE0);
const MUTED: Color32 = Color32::from_rgb(0xA9, 0xAE, 0xB9);
const ACCENT: Color32 = Color32::from_rgb(0xF2, 0xB5, 0x44);
const WARN: Color32 = Color32::from_rgb(0xF0, 0x8A, 0x6A);

fn theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x12, 0x13, 0x16);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, BG);
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.inactive.weak_bg_fill = PANEL;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 6.0);
        s.spacing.interact_size.y = 26.0;
    });
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Primary,
    Accent,
    Outline,
}

fn button(ui: &mut egui::Ui, label: &str, look: Look) -> egui::Response {
    let (fill, fg, stroke) = match look {
        Look::Primary => (TEXT, BG, TEXT),
        Look::Accent => (ACCENT, BG, ACCENT),
        Look::Outline => (Color32::TRANSPARENT, TEXT, OUTLINE),
    };
    let text = egui::RichText::new(label).color(fg).size(14.5);
    let text = if look == Look::Outline { text } else { text.strong() };
    ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::new(1.0, stroke)).corner_radius(10u8).min_size(egui::vec2(0.0, 38.0)))
}

fn link(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).size(13.0).color(MUTED)).frame(false))
}

/// Lädt ein eingebautes PNG als Textur (einmal pro Name).
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

/// Programm-Icon in `rect`; ohne eingebautes Icon eine farbige Kachel mit dem Anfangsbuchstaben.
fn paint_icon(ui: &egui::Ui, cache: &mut HashMap<String, egui::TextureHandle>, app: &App, rect: Rect) {
    let icon = app.icon.as_deref().unwrap_or(&app.id);
    if let Some(bytes) = icons::ICONS.iter().find(|(n, _)| *n == icon).map(|(_, b)| *b) {
        if let Some(t) = texture(ui.ctx(), cache, icon, bytes) {
            egui::Image::new(&t).paint_at(ui, rect);
            return;
        }
    }
    let color = app.color.as_deref().and_then(|c| Color32::from_hex(c).ok()).unwrap_or(OUTLINE);
    ui.painter().rect_filled(rect, (rect.width() * 0.25) as u8, color);
    let initial: String = app.name.chars().take(1).collect();
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initial, egui::FontId::proportional(rect.width() * 0.5), Color32::WHITE);
}

fn icon(ui: &mut egui::Ui, cache: &mut HashMap<String, egui::TextureHandle>, app: &App, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui, cache, app, rect);
}

fn logo(ui: &mut egui::Ui, cache: &mut HashMap<String, egui::TextureHandle>, size: f32) {
    if let Some(t) = texture(ui.ctx(), cache, "werkbank", icons::WERKBANK) {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(size)));
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Menu,
    Close,
    Back,
    Home,
    Grid,
    Update,
    File,
    Folder,
    Globe,
    Gear,
}

/// Kleine Strichsymbole, gezeichnet statt aus der Schrift (die nicht alle Zeichen kennt).
fn glyph(p: &egui::Painter, r: Rect, g: Glyph, color: Color32) {
    let s = Stroke::new(1.8, color);
    let c = r.center();
    let u = r.width() / 24.0;
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * u, r.top() + y * u);
    let line = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), s));
    };
    match g {
        Glyph::Menu => {
            for y in [7.0, 12.0, 17.0] {
                line(&[(4.0, y), (20.0, y)]);
            }
        }
        Glyph::Close => {
            line(&[(6.0, 6.0), (18.0, 18.0)]);
            line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Glyph::Back => {
            line(&[(19.0, 12.0), (5.0, 12.0)]);
            line(&[(11.0, 6.0), (5.0, 12.0), (11.0, 18.0)]);
        }
        Glyph::Home => line(&[(4.0, 11.0), (12.0, 4.0), (20.0, 11.0), (20.0, 20.0), (15.0, 20.0), (15.0, 15.0), (9.0, 15.0), (9.0, 20.0), (4.0, 20.0), (4.0, 11.0)]),
        Glyph::Grid => {
            for (x, y) in [(3.0, 3.0), (14.0, 3.0), (3.0, 14.0)] {
                p.rect_stroke(Rect::from_min_max(at(x, y), at(x + 7.0, y + 7.0)), 1.5, s, egui::StrokeKind::Middle);
            }
            line(&[(17.5, 14.0), (17.5, 21.0)]);
            line(&[(14.0, 17.5), (21.0, 17.5)]);
        }
        Glyph::Update => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = -0.6 + i as f32 / 24.0 * 5.2;
                    Pos2::new(c.x + a.cos() * 8.0 * u, c.y + a.sin() * 8.0 * u)
                })
                .collect();
            p.add(egui::Shape::line(pts, s));
            line(&[(20.0, 4.0), (19.6, 8.4), (15.5, 8.0)]);
        }
        Glyph::File => {
            line(&[(14.0, 3.0), (6.0, 3.0), (6.0, 21.0), (18.0, 21.0), (18.0, 7.0), (14.0, 3.0), (14.0, 7.0), (18.0, 7.0)]);
            line(&[(9.0, 13.0), (15.0, 13.0)]);
            line(&[(9.0, 17.0), (13.0, 17.0)]);
        }
        Glyph::Folder => line(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (21.0, 8.0), (21.0, 19.0), (3.0, 19.0), (3.0, 6.0)]),
        Glyph::Globe => {
            p.circle_stroke(c, 9.0 * u, s);
            line(&[(3.0, 12.0), (21.0, 12.0)]);
            p.add(egui::Shape::ellipse_stroke(c, egui::vec2(4.0 * u, 9.0 * u), s));
        }
        Glyph::Gear => {
            p.circle_stroke(c, 3.0 * u, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                p.line_segment([c + egui::vec2(a.cos(), a.sin()) * 6.0 * u, c + egui::vec2(a.cos(), a.sin()) * 9.5 * u], s);
            }
        }
    }
}

fn glyph_button(ui: &mut egui::Ui, g: Glyph, hint: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 10.0, HOVER);
    }
    glyph(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(22.0)), g, TEXT);
    resp.on_hover_text(hint)
}

/// Eine Karte fester Grösse: Rahmen zeichnen, Inhalt in einem Kind-Bereich.
fn card<R>(ui: &mut egui::Ui, size: Vec2, border: Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> (egui::Response, R) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect(rect, 16.0, PANEL, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(18.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    let r = add(&mut child);
    (resp, r)
}

/// Spaltenzahl und Breite für ein Raster mit Mindestbreite.
fn grid(width: f32, min: f32, gap: f32, max_cols: usize) -> (usize, f32) {
    let cols = (((width + gap) / (min + gap)).floor() as usize).clamp(1, max_cols.max(1));
    (cols, (width - gap * (cols as f32 - 1.0)) / cols as f32)
}


/// Zwei Umschaltknöpfe für ein Layout von rechts nach links («Alle» | «Nach Kategorie»);
/// liefert die neue Wahl.
fn segmented(ui: &mut egui::Ui, on: bool, on_label: &str, off_label: &str) -> Option<bool> {
    let mut out = None;
    // Im Layout von rechts nach links: zuerst der rechte Knopf.
    for (value, label) in [(true, on_label), (false, off_label)] {
        let active = on == value;
        let b = egui::Button::new(egui::RichText::new(label).size(13.0).color(if active { BG } else { SOFT }))
            .fill(if active { ACCENT } else { PANEL })
            .stroke(Stroke::new(1.0, if active { ACCENT } else { BORDER }))
            .corner_radius(14u8)
            .min_size(egui::vec2(0.0, 30.0));
        if ui.add(b).clicked() && !active {
            out = Some(value);
        }
    }
    out
}

/// Deutsch/English als zwei Knöpfe; liefert die neu gewählte Sprache.
fn language_pick(ui: &mut egui::Ui) -> Option<Lang> {
    let cur = sprache::lang();
    let mut out = None;
    ui.horizontal(|ui| {
        for (l, label) in [(Lang::De, "Deutsch"), (Lang::En, "English")] {
            let on = cur == l;
            let b = egui::Button::new(egui::RichText::new(label).size(14.0).color(if on { BG } else { SOFT }))
                .fill(if on { ACCENT } else { PANEL })
                .stroke(Stroke::new(1.0, if on { ACCENT } else { BORDER }))
                .corner_radius(16u8)
                .min_size(egui::vec2(100.0, 34.0));
            if ui.add(b).clicked() && !on {
                out = Some(l);
            }
        }
    });
    out
}

// ---------------------------------------------------------------------------------------------
// Zeit auf Deutsch

const WEEKDAYS: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
const MONTHS: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const WEEKDAYS_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn weekday(i: usize) -> &'static str {
    let i = i.min(6);
    if sprache::is_en() {
        WEEKDAYS_EN[i]
    } else {
        WEEKDAYS[i]
    }
}

/// Kurzer Wochentag aus der Wettervorhersage («Mo») in der gewählten Sprache.
fn weekday_short(de: &str) -> String {
    match ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"].iter().position(|d| *d == de) {
        Some(i) if sprache::is_en() => WEEKDAYS_EN[i][..3].to_string(),
        _ => de.to_string(),
    }
}

fn greeting(hour: u32) -> &'static str {
    t(match hour {
        5..=10 => "Guten Morgen",
        11..=17 => "Guten Tag",
        18..=22 => "Guten Abend",
        _ => "Gute Nacht",
    })
}

fn long_date(d: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let wd = weekday(d.weekday().num_days_from_monday() as usize);
    let m = (d.month0() as usize).min(11);
    if sprache::is_en() {
        format!("{wd}, {} {}", MONTHS_EN[m], d.day())
    } else {
        format!("{wd}, {}. {}", d.day(), MONTHS[m])
    }
}

/// «gerade eben», «heute 09:12», «gestern», «Mittwoch», «03.10.2026».
fn when(secs: u64, now: chrono::DateTime<chrono::Local>) -> String {
    use chrono::{Datelike, TimeZone};
    let Some(at) = chrono::Local.timestamp_opt(secs as i64, 0).single() else { return String::new() };
    let days = (now.date_naive() - at.date_naive()).num_days();
    if (now - at).num_seconds() < 90 {
        t("gerade eben").into()
    } else if days == 0 {
        tf("heute {zeit}", &[("zeit", &at.format("%H:%M"))])
    } else if days == 1 {
        t("gestern").into()
    } else if days < 7 {
        weekday(at.weekday().num_days_from_monday() as usize).into()
    } else if sprache::is_en() {
        at.format("%Y-%m-%d").to_string()
    } else {
        at.format("%d.%m.%Y").to_string()
    }
}

// ---------------------------------------------------------------------------------------------

impl Gui {
    /// Nach der Ersteinrichtung (oder sofort, wenn der Datenordner feststeht): Liste laden, los.
    fn begin(&mut self, ctx: &egui::Context, manifest: Manifest, bundle: Option<std::path::PathBuf>, online: bool) {
        self.manifest = manifest;
        self.refresh(ctx);
        self.fetch_weather(ctx);
        if let Some(dir) = bundle {
            self.start_bundle(ctx, dir, online);
        }
        // Schritt 2 der Ersteinrichtung: Dateitypen (nur, wo das System es kann).
        if assoc::supported() && !self.cfg.types_asked {
            self.types = Some(Types { choice: assoc::all(&self.manifest), message: None, first_run: true });
        }
    }

    /// Sprache wechseln: gilt sofort im Launcher und beim nächsten Start jedes Programms.
    fn set_language(&mut self, l: Lang) {
        sprache::set_lang(l);
        let saved = sprache::werkbank_file().ok_or_else(|| anyhow::anyhow!("kein Einstellungsordner")).and_then(|p| {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&p, l.code())?;
            Ok(())
        });
        if let Err(e) = saved {
            self.notice = Some(tf("Einstellungen nicht gespeichert: {fehler}", &[("fehler", &format!("{e:#}"))]));
        }
    }

    fn place(&self) -> Place {
        self.cfg.place.clone().unwrap_or_else(Place::sarnen)
    }

    fn save_cfg(&mut self) {
        // Andere Teile (Dateitypen, «--open») schreiben dieselbe Datei: frisch laden, eigene Felder setzen.
        let mut cfg = core::load_config();
        cfg.place = self.cfg.place.clone();
        cfg.home = self.cfg.home.clone();
        if let Err(e) = core::save_config(&cfg) {
            self.notice = Some(tf("Einstellungen nicht gespeichert: {fehler}", &[("fehler", &format!("{e:#}"))]));
        }
        self.cfg = cfg;
    }

    fn fetch_weather(&mut self, ctx: &egui::Context) {
        if !self.cfg.home.weather {
            return;
        }
        self.sky_at = Some(Instant::now());
        let place = self.place();
        let (sky, ctx) = (self.sky.clone(), ctx.clone());
        {
            let mut s = sky.lock().unwrap();
            if s.busy {
                return;
            }
            s.busy = true;
        }
        std::thread::spawn(move || {
            let r = weather::fetch(&place);
            let mut s = sky.lock().unwrap();
            s.busy = false;
            match r {
                Ok(w) => {
                    s.data = Some(w);
                    s.error = None;
                }
                Err(e) => s.error = Some(format!("{e:#}")),
            }
            ctx.request_repaint();
        });
    }

    /// Regelmässiges: Wetter alle 30 Minuten, «Zuletzt geöffnet» alle paar Sekunden neu lesen.
    fn tick(&mut self, ctx: &egui::Context) {
        if self.sky_at.is_none_or(|t| t.elapsed() > Duration::from_secs(30 * 60)) {
            self.fetch_weather(ctx);
        }
        if self.cfg_at.elapsed() > Duration::from_secs(4) {
            self.cfg_at = Instant::now();
            self.cfg.recent = core::load_config().recent;
        }
        if self.cfg.home.clock || self.cfg.home.recent {
            ctx.request_repaint_after(Duration::from_secs(1));
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
                    Ok(v) => {
                        st.latest = Some(v);
                        st.error = None;
                    }
                    Err(e) => {
                        let text = format!("{e:#}");
                        // Kein Release für dieses Programm ist kein Fehler, nur ein Zustand.
                        let missing = ["kein Release", "status: 401", "status: 404"].iter().any(|m| text.contains(m));
                        st.error = Some(if missing { t("Noch nicht veröffentlicht").to_string() } else { t("Keine Verbindung zum Server").to_string() });
                    }
                }
                ctx.request_repaint();
            });
        }
    }

    /// Installiert/aktualisiert ein Programm im Hintergrund; `then_open` öffnet danach den Browser.
    fn install(&self, ctx: &egui::Context, app: &App, kind: Kind, then_open: Option<String>) {
        let (app, state, ctx) = (app.clone(), self.state.clone(), ctx.clone());
        {
            let mut s = state.lock().unwrap();
            let st = s.entry(app.id.clone()).or_default();
            st.busy = Some((t("Starte …").into(), 0.0));
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
                    if st.latest.is_none_or(|l| l < v) {
                        st.latest = Some(v);
                    }
                    if let Some(url) = then_open {
                        let _ = open::that(url);
                    }
                }
                Err(e) => st.error = Some(format!("{e:#}")),
            }
            ctx.request_repaint();
        });
    }

    fn install_default(&self, ctx: &egui::Context, app: &App) {
        let kind = if app.web_only { Kind::Web } else { Kind::Native };
        self.install(ctx, app, kind, None);
    }

    fn update_all(&self, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        for app in &self.manifest.app {
            if let Some(st) = snapshot.get(&app.id).filter(|s| s.update()) {
                self.install(ctx, app, st.update_kind(), None);
            }
        }
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

    fn open_web(&mut self, ctx: &egui::Context, app: &App) {
        if let Some(addr) = self.web_addr() {
            // install() lädt nur, wenn die Web-Version fehlt oder veraltet ist.
            self.install(ctx, app, Kind::Web, Some(format!("{addr}/{}/", app.id)));
        }
    }

    /// Startet die Desktop-Version, sonst die Browser-Version.
    fn launch(&mut self, ctx: &egui::Context, app: &App, st: &Status) {
        if st.native.is_some() && !app.web_only {
            if let Err(e) = core::run(app) {
                self.notice = Some(format!("{e:#}"));
            }
        } else {
            self.open_web(ctx, app);
        }
    }

    fn remove(&mut self, app: &App) {
        if let Err(e) = core::uninstall(app) {
            self.notice = Some(format!("{e:#}"));
        }
        self.local(app);
    }

    fn open_recent(&mut self, r: &Recent) {
        let Some(app) = self.manifest.app.iter().find(|a| a.id == r.app).cloned() else { return };
        if let Err(e) = core::run_with(&app, Some(&r.path)) {
            self.notice = Some(format!("{e:#}"));
        }
        self.cfg.recent = core::load_config().recent;
    }

    /// «Datei öffnen …»: Programm nach der Dateiendung wählen.
    fn open_file(&mut self, snapshot: &HashMap<String, Status>) {
        let Some(path) = rfd::FileDialog::new().set_title(t("Datei öffnen")).pick_file() else { return };
        let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
        let candidates: Vec<&App> = self.manifest.app.iter().filter(|a| a.types.iter().any(|t| t.eq_ignore_ascii_case(&ext))).collect();
        let installed = candidates.iter().find(|a| snapshot.get(&a.id).is_some_and(|s| s.native.is_some()));
        match (installed, candidates.first()) {
            (Some(app), _) => {
                let app = (*app).clone();
                if let Err(e) = core::run_with(&app, Some(&path)) {
                    self.notice = Some(format!("{e:#}"));
                }
                self.cfg.recent = core::load_config().recent;
            }
            (None, Some(app)) => {
                self.notice = Some(tf("Für {ext}-Dateien zuerst {name} installieren (Menü → Programme entdecken).", &[("ext", &ext), ("name", &app.name)]));
            }
            (None, None) => self.notice = Some(tf("Kein Programm der Werkbank öffnet {ext}-Dateien.", &[("ext", &ext)])),
        }
    }

    fn go(&mut self, view: View) {
        self.view = view;
        self.menu = false;
    }

    fn open_catalog(&mut self, chip: Chip) {
        self.chip = chip;
        self.search.clear();
        self.go(View::Catalog);
    }

    /// Offline-Setup: richtet die Archive aus `dir` nacheinander im Hintergrund ein.
    fn start_bundle(&mut self, ctx: &egui::Context, dir: std::path::PathBuf, online: bool) {
        let entries = core::bundle_entries(&self.manifest, &dir);
        if entries.is_empty() {
            return;
        }
        self.notice = Some(if online {
            tf("Einrichtung: {n} Programm(e) aus dem Setup, neuere Versionen werden aus dem Netz geholt", &[("n", &entries.len())])
        } else {
            tf("Einrichtung: {n} Programm(e) aus dem Setup", &[("n", &entries.len())])
        });
        {
            let mut s = self.state.lock().unwrap();
            for e in &entries {
                s.entry(e.app.id.clone()).or_default().busy = Some((t("Wartet …").into(), 0.0));
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

    // -----------------------------------------------------------------------------------------
    // Ersteinrichtung und Dateitypen

    fn setup_screen(&mut self, ctx: &egui::Context) {
        let mut confirm = false;
        let mut browse = false;
        let mut pick_lang = None;
        let Some(setup) = self.setup.as_mut() else { return };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(48, 40))).show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                ui.set_max_width(640.0);
                ui.horizontal(|ui| {
                    logo(ui, &mut self.textures, 56.0);
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(t("Willkommen in der Werkbank")).size(24.0).strong());
                        ui.label(egui::RichText::new(t("Einmalige Einrichtung")).size(13.0).color(MUTED));
                    });
                });
                ui.add_space(20.0);
                ui.label(egui::RichText::new("Sprache · Language").size(16.0).strong());
                ui.add_space(4.0);
                pick_lang = language_pick(ui);
                ui.add_space(20.0);
                ui.label(egui::RichText::new(t("Wo sollen die Programme abgelegt werden?")).size(16.0).strong());
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(t("Der Ordner wird angelegt, falls er fehlt. Unter C:\\Program Files fragt Windows einmalig nach Administratorrechten; danach braucht die Werkbank keine mehr."))
                    .size(13.0)
                    .color(MUTED),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 140.0;
                    ui.add_sized([w, 36.0], egui::TextEdit::singleline(&mut setup.path).font(egui::FontId::proportional(15.0)));
                    if button(ui, t("Durchsuchen …"), Look::Outline).clicked() {
                        browse = true;
                    }
                });
                if let Some(e) = &setup.error {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(e).color(WARN));
                }
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if button(ui, t("Weiter"), Look::Accent).clicked() {
                        confirm = true;
                    }
                    if button(ui, t("Vorschlag"), Look::Outline).clicked() {
                        setup.path = core::default_data_dir().display().to_string();
                    }
                });
            });
        });
        if let Some(l) = pick_lang {
            self.set_language(l);
            return;
        }
        let Some(setup) = self.setup.as_mut() else { return };
        if browse {
            if let Some(dir) = rfd::FileDialog::new().set_title(t("Datenordner wählen")).pick_folder() {
                setup.path = dir.display().to_string();
            }
        }
        if confirm {
            let path = std::path::PathBuf::from(setup.path.trim());
            match core::set_data_dir(&path) {
                Ok(()) => {
                    let l = sprache::lang();
                    if let Some(setup) = self.setup.take() {
                        self.set_language(l);
                        self.cfg = core::load_config();
                        let manifest = (setup.load)();
                        self.begin(ctx, manifest, setup.bundle, setup.online);
                    }
                }
                Err(e) => setup.error = Some(format!("{e:#}")),
            }
        }
    }

    fn open_types(&mut self) {
        self.menu = false;
        self.types = Some(Types { choice: core::load_config().types, message: None, first_run: false });
    }

    fn types_screen(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut apply = false;
        let Some(types) = self.types.as_mut() else { return };
        let apps: Vec<App> = self.manifest.app.iter().filter(|a| !a.types.is_empty()).cloned().collect();
        egui::TopBottomPanel::top("typen-kopf").frame(egui::Frame::new().fill(HEADER).inner_margin(egui::Margin::symmetric(24, 14))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if !types.first_run && glyph_button(ui, Glyph::Back, t("Zurück")).clicked() {
                    close = true;
                }
                ui.vertical(|ui| {
                    let title = if types.first_run { t("Einrichtung: Dateitypen") } else { t("Dateitypen") };
                    ui.label(egui::RichText::new(title).size(22.0).strong());
                    ui.label(egui::RichText::new(t("Angehakte Dateitypen lassen sich per Rechtsklick → «Öffnen mit» in der Werkbank öffnen.")).size(12.5).color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if types.first_run { t("Weiter") } else { t("Übernehmen") };
                    if button(ui, label, Look::Accent).clicked() {
                        apply = true;
                    }
                    if types.first_run && button(ui, t("Überspringen"), Look::Outline).clicked() {
                        types.choice.clear();
                        apply = true;
                    }
                });
            });
        });
        egui::TopBottomPanel::bottom("typen-fuss").frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(24, 10))).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t("Welches Programm Standard ist, legt Windows fest: Rechtsklick → «Öffnen mit» → «Immer», oder hier:")).size(12.5).color(MUTED));
                if link(ui, t("Standard-Apps in Windows öffnen")).clicked() {
                    assoc::open_system_settings();
                }
            });
            if let Some((m, ok)) = &types.message {
                ui.label(egui::RichText::new(m).color(if *ok { ACCENT } else { WARN }));
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(24, 18))).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for app in &apps {
                    let exts = assoc::offered(app);
                    let chosen = types.choice.entry(app.id.clone()).or_default();
                    egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(12u8).inner_margin(14.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            icon(ui, &mut self.textures, app, 36.0);
                            ui.label(egui::RichText::new(&app.name).size(17.0).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button(t("Keine")).clicked() {
                                    chosen.clear();
                                }
                                if ui.small_button(t("Alle")).clicked() {
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
            let wanted: assoc::Choice = types.choice.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
            match assoc::apply(&self.manifest, &cfg.types, &wanted) {
                Ok(()) => {
                    cfg.types = wanted;
                    cfg.types_asked = true;
                    let _ = core::save_config(&cfg);
                    self.cfg = cfg;
                    close = true;
                }
                Err(e) => types.message = Some((format!("{e:#}"), false)),
            }
        }
        if close {
            self.types = None;
        }
    }

    // -----------------------------------------------------------------------------------------
    // Kopfzeile, Menü

    fn header(&mut self, ctx: &egui::Context, title: Option<&str>) {
        egui::TopBottomPanel::top("kopf").frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 18, right: 24, top: 14, bottom: 6 })).show(ctx, |ui| {
            ui.horizontal(|ui| {
                match title {
                    None => {
                        if glyph_button(ui, Glyph::Menu, t("Menü")).clicked() {
                            self.menu = !self.menu;
                        }
                        logo(ui, &mut self.textures, 32.0);
                        ui.label(egui::RichText::new(&self.manifest.title).size(21.0).strong());
                    }
                    Some(title) => {
                        if glyph_button(ui, Glyph::Back, t("Zur Startseite")).clicked() {
                            self.go(View::Home);
                        }
                        ui.label(egui::RichText::new(title).size(21.0).strong());
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.view == View::Home && button(ui, t("+  Programme hinzufügen"), Look::Accent).clicked() {
                        self.open_catalog(Chip::All);
                    }
                    if self.view != View::Home && glyph_button(ui, Glyph::Menu, t("Menü")).clicked() {
                        self.menu = !self.menu;
                    }
                });
            });
            let mut clear = false;
            if let Some(n) = &self.notice {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(n).color(WARN));
                    if link(ui, t("Ausblenden")).clicked() {
                        clear = true;
                    }
                });
            }
            if clear {
                self.notice = None;
            }
        });
    }

    fn drawer(&mut self, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        let installed = self.manifest.app.iter().filter(|a| snapshot.get(&a.id).is_some_and(Status::installed)).count();
        let total = self.manifest.app.len();
        let updates = snapshot.values().filter(|s| s.update()).count();
        let mut changed = false;
        let panel = egui::SidePanel::left("menue")
            .exact_width(320.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(HEADER).stroke(Stroke::new(1.0, BORDER)).inner_margin(egui::Margin::symmetric(14, 18)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    logo(ui, &mut self.textures, 40.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(t("Werkbank")).size(20.0).strong());
                        ui.label(egui::RichText::new(tf("{installed} von {total} Programmen installiert", &[("installed", &installed), ("total", &total)])).size(12.0).color(MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if glyph_button(ui, Glyph::Close, t("Menü schliessen")).clicked() {
                            self.menu = false;
                        }
                    });
                });
                ui.add_space(12.0);
                let mut item = |ui: &mut egui::Ui, g: Glyph, label: &str, badge: Option<(usize, bool)>, active: bool| -> bool {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 48.0), Sense::click());
                    if active || resp.hovered() {
                        ui.painter().rect_filled(rect, 10.0, HOVER);
                    }
                    let gr = Rect::from_center_size(Pos2::new(rect.left() + 26.0, rect.center().y), Vec2::splat(22.0));
                    glyph(ui.painter(), gr, g, if active { ACCENT } else { MUTED });
                    ui.painter().text(
                        Pos2::new(rect.left() + 52.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(15.0),
                        TEXT,
                    );
                    if let Some((n, hot)) = badge.filter(|(n, _)| *n > 0) {
                        let text = n.to_string();
                        let r = Rect::from_center_size(Pos2::new(rect.right() - 26.0, rect.center().y), egui::vec2(12.0 + 8.0 * text.len() as f32, 22.0));
                        ui.painter().rect_filled(r, 11.0, if hot { ACCENT } else { CHIP });
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(12.0), if hot { BG } else { SOFT });
                    }
                    resp.clicked()
                };
                if item(ui, Glyph::Home, t("Startseite"), None, self.view == View::Home) {
                    self.go(View::Home);
                }
                if item(ui, Glyph::Grid, t("Programme entdecken"), Some((total - installed, false)), self.view == View::Catalog && self.chip != Chip::Updates) {
                    self.open_catalog(Chip::All);
                }
                if item(ui, Glyph::Update, t("Updates"), Some((updates, true)), self.view == View::Catalog && self.chip == Chip::Updates) {
                    self.open_catalog(Chip::Updates);
                }
                if assoc::supported() && item(ui, Glyph::File, t("Dateitypen"), None, false) {
                    self.open_types();
                }
                if item(ui, Glyph::Folder, t("Datenordner öffnen"), None, false) {
                    let _ = open::that(core::data_dir());
                }
                if item(ui, Glyph::Globe, t("Browser-Übersicht"), None, false) {
                    if let Some(a) = self.web_addr() {
                        let _ = open::that(a);
                    }
                }
                if item(ui, Glyph::Gear, t("Einstellungen"), None, self.view == View::Prefs) {
                    self.go(View::Prefs);
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(egui::RichText::new(t("STARTSEITE ZEIGT")).size(11.5).color(MUTED));
                let place = self.place().name;
                ui.add_space(2.0);
                changed |= ui.checkbox(&mut self.cfg.home.clock, t("Uhr und Datum")).changed();
                changed |= ui.checkbox(&mut self.cfg.home.weather, tf("Wetter für {ort}", &[("ort", &place)])).changed();
                changed |= ui.checkbox(&mut self.cfg.home.recent, t("Zuletzt geöffnete Dateien")).changed();
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(concat!("Werkbank ", env!("CARGO_PKG_VERSION"), " ·")).size(12.0).color(MUTED));
                        if link(ui, t("Über Werkbank")).clicked() {
                            self.about = true;
                        }
                    });
                });
            });
        if changed {
            self.save_cfg();
            if self.cfg.home.weather {
                self.fetch_weather(ctx);
            }
        }
        // Rest abdunkeln; ein Klick daneben schliesst das Menü.
        let screen = ctx.screen_rect();
        let rest = Rect::from_min_max(Pos2::new(panel.response.rect.right(), screen.top()), screen.max);
        egui::Area::new(egui::Id::new("menue-schatten")).order(egui::Order::Middle).fixed_pos(rest.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(rest.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(150));
            if resp.clicked() {
                self.menu = false;
            }
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.menu = false;
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.about;
        egui::Window::new(t("Über Werkbank"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    logo(ui, &mut self.textures, 48.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(concat!("Werkbank ", env!("CARGO_PKG_VERSION"))).size(18.0).strong());
                        ui.label(egui::RichText::new(t("Programme installieren, starten und einzeln aktualisieren")).color(MUTED));
                    });
                });
                ui.add_space(8.0);
                ui.label(t("Die Programme basieren auf den Open-Source-Apps des ArtCraft-Teams (MIT / Apache 2.0); dazu kommen eigene Werkzeuge."));
                ui.label(t("Wetterdaten: Open-Meteo.com (CC BY 4.0)."));
                ui.hyperlink_to(t("Quelltext auf GitHub"), "https://github.com/0ldini/werkbank");
            });
        self.about = open;
    }

    // -----------------------------------------------------------------------------------------
    // Startseite

    fn home(&mut self, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        let mine: Vec<App> = self.manifest.app.iter().filter(|a| snapshot.get(&a.id).is_some_and(|s| s.installed() || s.busy.is_some())).cloned().collect();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 24, right: 24, top: 10, bottom: 18 })).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                self.widgets(ui, ctx, snapshot);
                ui.add_space(14.0);
                let updates: usize = mine.iter().filter(|a| snapshot.get(&a.id).is_some_and(Status::update)).count();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t("Meine Programme")).size(18.0).strong());
                    ui.label(egui::RichText::new(mine.len().to_string()).size(14.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if updates > 0 {
                            if button(ui, t("Aktualisieren"), Look::Outline).clicked() {
                                self.update_all(ctx, snapshot);
                            }
                            ui.label(egui::RichText::new(if updates == 1 { t("1 Update bereit").to_string() } else { tf("{n} Updates bereit", &[("n", &updates)]) }).color(ACCENT));
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, ACCENT);
                            ui.add_space(12.0);
                        }
                        if let Some(g) = segmented(ui, self.cfg.home.grouped, t("Nach Kategorie"), t("Alle")) {
                            self.cfg.home.grouped = g;
                            self.save_cfg();
                        }
                    });
                });
                ui.add_space(6.0);
                if mine.is_empty() {
                    self.empty_home(ui);
                    return;
                }
                if self.cfg.home.grouped {
                    let groups: Vec<(Chip, Vec<&App>)> = CATEGORIES
                        .iter()
                        .map(|c| (*c, mine.iter().filter(|a| category(a) == *c).collect::<Vec<_>>()))
                        .filter(|(_, v)| !v.is_empty())
                        .collect();
                    let last = groups.len().saturating_sub(1);
                    for (i, (c, apps)) in groups.into_iter().enumerate() {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(c.label()).size(15.0).color(SOFT));
                            ui.label(egui::RichText::new(apps.len().to_string()).size(13.0).color(MUTED));
                        });
                        ui.add_space(2.0);
                        let mut items: Vec<Option<&App>> = apps.into_iter().map(Some).collect();
                        if i == last {
                            items.push(None);
                        }
                        self.tile_grid(ui, ctx, snapshot, &items);
                        ui.add_space(6.0);
                    }
                } else {
                    let mut items: Vec<Option<&App>> = mine.iter().map(Some).collect();
                    items.push(None);
                    self.tile_grid(ui, ctx, snapshot, &items);
                }
            });
        });
    }

    /// Kacheln im Raster; `None` ist die Kachel «Hinzufügen».
    fn tile_grid(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>, items: &[Option<&App>]) {
        let gap = 12.0;
        let (cols, w) = grid(ui.available_width(), 124.0, gap, 12);
        for row in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for item in row {
                    match item {
                        Some(app) => {
                            let st = snapshot.get(&app.id).cloned().unwrap_or_default();
                            self.app_tile(ui, ctx, app, &st, w);
                        }
                        None => self.add_tile(ui, w),
                    }
                }
            });
            ui.add_space(gap - 8.0);
        }
    }

    fn empty_home(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(16u8).inner_margin(28.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(t("Noch keine Programme installiert")).size(18.0).strong());
                ui.label(egui::RichText::new(t("Programme für Büro, Bild, Video, Ton und mehr – einfach auswählen und installieren.")).color(MUTED));
                ui.add_space(8.0);
                if button(ui, t("Programme entdecken"), Look::Accent).clicked() {
                    self.open_catalog(Chip::All);
                }
            });
        });
    }

    fn app_tile(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, app: &App, st: &Status, w: f32) {
        let update = st.update();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 136.0), Sense::click());
        let p = ui.painter();
        p.rect(rect, 16.0, if resp.hovered() { HOVER } else { PANEL }, Stroke::new(if update { 1.5 } else { 1.0 }, if update { ACCENT } else { BORDER }), egui::StrokeKind::Inside);
        let ir = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 50.0), Vec2::splat(64.0));
        paint_icon(ui, &mut self.textures, app, ir);
        let p = ui.painter();
        let galley = p.layout(app.name.clone(), egui::FontId::proportional(14.0), TEXT, w - 16.0);
        p.galley(Pos2::new(rect.center().x - galley.size().x / 2.0, rect.top() + 94.0), galley, TEXT);
        if update {
            let b = Rect::from_min_size(Pos2::new(rect.right() - 62.0, rect.top() + 8.0), egui::vec2(54.0, 20.0));
            p.rect_filled(b, 10.0, ACCENT);
            p.text(b.center(), egui::Align2::CENTER_CENTER, t("Update"), egui::FontId::proportional(11.5), BG);
        }
        if let Some((_, frac)) = &st.busy {
            let bar = Rect::from_min_size(Pos2::new(rect.left() + 14.0, rect.bottom() - 14.0), egui::vec2(w - 28.0, 5.0));
            p.rect_filled(bar, 3.0, BORDER);
            p.rect_filled(Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac.clamp(0.02, 1.0), 5.0)), 3.0, ACCENT);
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let mut hint = app.about_text().to_string();
        if let Some(v) = st.current() {
            hint += &format!("\n{}", tf("Version {v}", &[("v", &fmt_version(v))]));
        }
        if let (true, Some(l)) = (update, st.latest) {
            hint += &format!(" · {}", tf("neu: {v}", &[("v", &fmt_version(l))]));
        }
        if let Some((m, _)) = &st.busy {
            hint += &format!("\n{m}");
        }
        if let Some(e) = &st.error {
            hint += &format!("\n{e}");
        }
        let resp = resp.on_hover_text(hint);
        if resp.clicked() && st.busy.is_none() {
            self.launch(ctx, app, st);
        }
        resp.context_menu(|ui| {
            if st.native.is_some() && !app.web_only && ui.button(t("Starten")).clicked() {
                self.launch(ctx, app, st);
                ui.close_menu();
            }
            if !app.no_web && ui.button(t("Im Browser öffnen")).clicked() {
                self.open_web(ctx, app);
                ui.close_menu();
            }
            if update && ui.button(tf("Aktualisieren auf {v}", &[("v", &st.latest.map(fmt_version).unwrap_or_default())])).clicked() {
                self.install(ctx, app, st.update_kind(), None);
                ui.close_menu();
            }
            ui.separator();
            if ui.button(t("Entfernen")).clicked() {
                self.remove(app);
                ui.close_menu();
            }
        });
    }

    fn add_tile(&mut self, ui: &mut egui::Ui, w: f32) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 136.0), Sense::click());
        let p = ui.painter();
        let r = rect.shrink(1.0);
        let color = if resp.hovered() { ACCENT } else { OUTLINE };
        let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        p.extend(egui::Shape::dashed_line(&corners, Stroke::new(1.5, color), 6.0, 5.0));
        let c = Pos2::new(rect.center().x, rect.top() + 50.0);
        p.circle_stroke(c, 22.0, Stroke::new(1.5, color));
        p.line_segment([c - egui::vec2(9.0, 0.0), c + egui::vec2(9.0, 0.0)], Stroke::new(2.0, color));
        p.line_segment([c - egui::vec2(0.0, 9.0), c + egui::vec2(0.0, 9.0)], Stroke::new(2.0, color));
        p.text(Pos2::new(rect.center().x, rect.top() + 102.0), egui::Align2::CENTER_CENTER, t("Hinzufügen"), egui::FontId::proportional(14.0), MUTED);
        if resp.on_hover_text(t("Weitere Programme installieren")).clicked() {
            self.open_catalog(Chip::All);
        }
    }

    fn widgets(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        let home = self.cfg.home.clone();
        let n = [home.clock, home.weather, home.recent].iter().filter(|b| **b).count();
        if n == 0 {
            return;
        }
        let gap = 16.0;
        let (cols, w) = grid(ui.available_width(), 270.0, gap, n);
        let mut cards = Vec::new();
        if home.clock {
            cards.push(0);
        }
        if home.weather {
            cards.push(1);
        }
        if home.recent {
            cards.push(2);
        }
        let size = egui::vec2(w, 214.0);
        for row in cards.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for which in row {
                    match which {
                        0 => self.clock_card(ui, size),
                        1 => self.weather_card(ui, ctx, size),
                        _ => self.recent_card(ui, size, snapshot),
                    }
                }
            });
            ui.add_space(gap - 8.0);
        }
    }

    fn clock_card(&mut self, ui: &mut egui::Ui, size: Vec2) {
        use chrono::Timelike;
        let now = chrono::Local::now();
        card(ui, size, BORDER, |ui| {
            ui.label(egui::RichText::new(greeting(now.hour())).size(14.0).color(MUTED));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(now.format("%H:%M").to_string()).size(66.0).strong());
            ui.add_space(4.0);
            ui.label(egui::RichText::new(long_date(now.date_naive())).size(16.0).color(SOFT));
        });
    }

    fn weather_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, size: Vec2) {
        let place = self.place();
        let (data, error, busy) = {
            let s = self.sky.lock().unwrap();
            (s.data.clone().filter(|w| w.place == place.name), s.error.clone(), s.busy)
        };
        let mut prefs = false;
        let mut retry = false;
        card(ui, size, BORDER, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&place.name).size(14.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if link(ui, t("Ort ändern")).clicked() {
                        prefs = true;
                    }
                });
            });
            let Some(w) = data else {
                ui.add_space(30.0);
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new(t("Wetter wird geladen …")).color(MUTED));
                    });
                } else {
                    ui.label(egui::RichText::new(t("Kein Wetter verfügbar")).size(16.0));
                    if let Some(e) = &error {
                        ui.label(egui::RichText::new(t("Keine Verbindung zum Wetterdienst")).size(12.5).color(MUTED)).on_hover_text(e);
                    }
                    if link(ui, t("Erneut versuchen")).clicked() {
                        retry = true;
                    }
                }
                return;
            };
            let top = ui.cursor().min;
            let width = ui.available_width();
            ui.label(egui::RichText::new(weather::deg(w.temp)).size(46.0).strong());
            ui.label(egui::RichText::new(w.code.map(|c| t(weather::describe(c, w.sky != weather::Sky::Moon).0).to_string()).unwrap_or_else(|| w.text.clone())).size(15.0));
            let mut info = format!("H {} · T {}", weather::deg(w.today_max), weather::deg(w.today_min));
            if w.rain_prob > 0.0 {
                info += &format!(" · {}", tf("Regen {p} %", &[("p", &format!("{:.0}", w.rain_prob))]));
            }
            ui.label(egui::RichText::new(info).size(12.5).color(MUTED));
            weather::draw(ui.painter(), Pos2::new(top.x + width - 34.0, top.y + 34.0), 68.0, w.sky, ACCENT, PANEL);
            // Vorhersage
            let days = w.days.iter().take(5).collect::<Vec<_>>();
            if days.is_empty() {
                return;
            }
            let bottom = ui.max_rect().bottom();
            let rect = Rect::from_min_max(Pos2::new(top.x, (bottom - 52.0).max(ui.cursor().min.y)), Pos2::new(top.x + width, bottom));
            let cw = rect.width() / days.len() as f32;
            let p = ui.painter();
            for (i, d) in days.iter().enumerate() {
                let x = rect.left() + cw * (i as f32 + 0.5);
                p.text(Pos2::new(x, rect.top() + 6.0), egui::Align2::CENTER_CENTER, weekday_short(&d.weekday), egui::FontId::proportional(12.0), MUTED);
                weather::draw(p, Pos2::new(x, rect.top() + 24.0), 22.0, d.sky, ACCENT, PANEL);
                p.text(Pos2::new(x, rect.top() + 44.0), egui::Align2::CENTER_CENTER, weather::deg(d.max), egui::FontId::proportional(13.0), TEXT);
            }
        });
        if prefs {
            self.go(View::Prefs);
        }
        if retry {
            self.fetch_weather(ctx);
        }
    }

    fn recent_card(&mut self, ui: &mut egui::Ui, size: Vec2, snapshot: &HashMap<String, Status>) {
        let now = chrono::Local::now();
        let recent: Vec<Recent> = self.cfg.recent.iter().filter(|r| r.path.exists()).take(3).cloned().collect();
        let mut open: Option<Recent> = None;
        let mut pick = false;
        card(ui, size, BORDER, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Zuletzt geöffnet")).size(14.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if link(ui, t("Datei öffnen …")).clicked() {
                        pick = true;
                    }
                });
            });
            if recent.is_empty() {
                ui.add_space(14.0);
                ui.label(egui::RichText::new(t("Noch keine Dateien")).size(15.0));
                ui.label(egui::RichText::new(t("Dateien, die du über die Werkbank öffnest, erscheinen hier.")).size(12.5).color(MUTED));
                return;
            }
            ui.add_space(2.0);
            for r in &recent {
                let Some(app) = self.manifest.app.iter().find(|a| a.id == r.app) else { continue };
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect.expand2(egui::vec2(6.0, 0.0)), 8.0, HOVER);
                }
                paint_icon(ui, &mut self.textures, app, Rect::from_min_size(Pos2::new(rect.left(), rect.top() + 6.0), Vec2::splat(32.0)));
                let name = r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let p = ui.painter();
                let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(14.0), TEXT);
                job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 44.0);
                p.galley(Pos2::new(rect.left() + 44.0, rect.top() + 5.0), p.layout_job(job), TEXT);
                p.text(Pos2::new(rect.left() + 44.0, rect.top() + 25.0), egui::Align2::LEFT_TOP, format!("{} · {}", app.name, when(r.when, now)), egui::FontId::proportional(12.0), MUTED);
                let installed = snapshot.get(&app.id).is_some_and(|s| s.native.is_some());
                let resp = resp.on_hover_text(r.path.display().to_string());
                if resp.clicked() {
                    if installed {
                        open = Some(r.clone());
                    } else {
                        self.notice = Some(tf("{name} ist nicht mehr installiert.", &[("name", &app.name)]));
                    }
                }
            }
        });
        if let Some(r) = open {
            self.open_recent(&r);
        }
        if pick {
            self.open_file(snapshot);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Programme entdecken

    fn catalog(&mut self, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        let st = |a: &App| snapshot.get(&a.id).cloned().unwrap_or_default();
        let count = |chip: Chip| self.manifest.app.iter().filter(|a| matches(chip, a, &st(a))).count();
        let chips: Vec<(Chip, usize)> = [Chip::All, Chip::Office, Chip::Photo, Chip::Video, Chip::System, Chip::Installed, Chip::Updates]
            .into_iter()
            .map(|c| (c, count(c)))
            .filter(|(c, n)| *n > 0 || *c == self.chip || *c == Chip::All)
            .collect();
        let updates = count(Chip::Updates);
        let q = self.search.trim().to_lowercase();
        let mut list: Vec<App> = self
            .manifest
            .app
            .iter()
            .filter(|a| matches(self.chip, a, &st(a)))
            .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.about.to_lowercase().contains(&q) || a.about_text().to_lowercase().contains(&q) || a.types.iter().any(|t| t.contains(&q)))
            .cloned()
            .collect();
        // Nicht installierte zuerst; Updates noch davor.
        list.sort_by_key(|a| {
            let s = st(a);
            (!s.update(), s.installed())
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 24, right: 24, top: 10, bottom: 18 })).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let w = (ui.available_width() * 0.4).clamp(200.0, 360.0);
                ui.add_sized([w, 38.0], egui::TextEdit::singleline(&mut self.search).hint_text(t("Suchen: Name, Zweck oder Dateityp")).font(egui::FontId::proportional(15.0)).margin(egui::vec2(12.0, 9.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if updates > 0 && button(ui, &tf("Alle aktualisieren ({n})", &[("n", &updates)]), Look::Accent).clicked() {
                        self.update_all(ctx, snapshot);
                    }
                    if button(ui, t("Nach Updates suchen"), Look::Outline).clicked() {
                        self.refresh(ctx);
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for (chip, n) in &chips {
                    let on = self.chip == *chip;
                    let text = egui::RichText::new(format!("{} ({n})", chip.label())).size(13.5).color(if on { BG } else { SOFT });
                    let b = egui::Button::new(if on { text.strong() } else { text })
                        .fill(if on { ACCENT } else { PANEL })
                        .stroke(Stroke::new(1.0, if on { ACCENT } else { BORDER }))
                        .corner_radius(16u8)
                        .min_size(egui::vec2(0.0, 32.0));
                    if ui.add(b).clicked() {
                        self.chip = *chip;
                    }
                }
            });
            ui.add_space(10.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                if list.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        let msg = match self.chip {
                            Chip::Updates => t("Alles aktuell, keine Updates nötig."),
                            Chip::Installed => t("Noch nichts installiert."),
                            _ => t("Nichts gefunden."),
                        };
                        ui.label(egui::RichText::new(msg).size(16.0).color(MUTED));
                    });
                    return;
                }
                let gap = 14.0;
                let (cols, w) = grid(ui.available_width(), 300.0, gap, 4);
                for row in list.chunks(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for app in row {
                            self.catalog_card(ui, ctx, app, &st(app), w);
                        }
                    });
                    ui.add_space(gap - 8.0);
                }
            });
        });
    }

    fn catalog_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, app: &App, st: &Status, w: f32) {
        let update = st.update();
        let border = if update || st.busy.is_some() { ACCENT } else { BORDER };
        card(ui, egui::vec2(w, 178.0), border, |ui| {
            ui.horizontal(|ui| {
                icon(ui, &mut self.textures, app, 52.0);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(&app.name).size(17.0).strong());
                    let line = match (st.current(), st.latest) {
                        (Some(n), Some(l)) if update => tf("Update auf {neu} · installiert {alt}", &[("neu", &fmt_version(l)), ("alt", &fmt_version(n))]),
                        (Some(n), _) => tf("Installiert · {v}", &[("v", &fmt_version(n))]),
                        (None, Some(l)) => tf("Version {v}", &[("v", &fmt_version(l))]),
                        (None, None) => st.error.clone().unwrap_or_else(|| t("Version wird gesucht …").into()),
                    };
                    ui.label(egui::RichText::new(line).size(12.5).color(if update { ACCENT } else { MUTED }));
                });
            });
            ui.add_space(4.0);
            let mut job = egui::text::LayoutJob::simple(app.about_text().to_string(), egui::FontId::proportional(13.0), SOFT, ui.available_width());
            job.wrap.max_rows = 2;
            ui.label(job);
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                if let Some((msg, frac)) = &st.busy {
                    ui.add(egui::ProgressBar::new(*frac).desired_height(6.0).fill(ACCENT));
                    ui.label(egui::RichText::new(msg).size(12.5).color(MUTED));
                    ctx.request_repaint_after(Duration::from_millis(200));
                    return;
                }
                ui.horizontal(|ui| {
                    if st.installed() {
                        if update && button(ui, t("Aktualisieren"), Look::Accent).clicked() {
                            self.install(ctx, app, st.update_kind(), None);
                        }
                        if button(ui, t("Öffnen"), if update { Look::Outline } else { Look::Primary }).clicked() {
                            self.launch(ctx, app, st);
                        }
                        if link(ui, t("Entfernen")).clicked() {
                            self.remove(app);
                        }
                    } else {
                        let ready = st.latest.is_some();
                        if ui.add_enabled_ui(ready, |ui| button(ui, t("Installieren"), Look::Primary)).inner.clicked() {
                            self.install_default(ctx, app);
                        }
                        if !app.no_web && !app.web_only && ready && button(ui, t("Im Browser testen"), Look::Outline).clicked() {
                            self.open_web(ctx, app);
                        }
                    }
                });
            });
        });
    }

    // -----------------------------------------------------------------------------------------
    // Einstellungen

    fn prefs(&mut self, ctx: &egui::Context) {
        let mut chosen: Option<Place> = None;
        let mut search = false;
        let mut changed = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin { left: 24, right: 24, top: 10, bottom: 18 })).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(720.0);
                let section = |ui: &mut egui::Ui, title: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
                    egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(14u8).inner_margin(18.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(title).size(17.0).strong());
                        ui.add_space(4.0);
                        add(ui);
                    });
                    ui.add_space(12.0);
                };
                let mut lang = None;
                section(ui, "Sprache · Language", &mut |ui| {
                    lang = language_pick(ui);
                    ui.label(egui::RichText::new(t("Gilt für die Werkbank und alle Programme; offene Programme beim nächsten Start. Klangwerk und Schauwerk gibt es nur auf Englisch, einige Programme sind nur teilweise übersetzt.")).size(12.5).color(MUTED));
                });
                if let Some(l) = lang {
                    self.set_language(l);
                }
                let place = self.place();
                section(ui, t("Ort für das Wetter"), &mut |ui| {
                    ui.label(egui::RichText::new(format!("{}{}", place.name, place.region.as_deref().map(|r| format!(" · {r}")).unwrap_or_default())).color(SOFT));
                    ui.add_space(4.0);
                    let mut ps = self.places.lock().unwrap();
                    ui.horizontal(|ui| {
                        let r = ui.add_sized([280.0, 34.0], egui::TextEdit::singleline(&mut ps.query).hint_text(t("Anderen Ort suchen, z. B. Luzern")).margin(egui::vec2(10.0, 8.0)));
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (button(ui, t("Suchen"), Look::Outline).clicked() || enter) && !ps.busy {
                            search = true;
                        }
                        if ps.busy {
                            ui.spinner();
                        }
                    });
                    match &ps.result {
                        Some(Ok(list)) if list.is_empty() => {
                            ui.label(egui::RichText::new(t("Kein Ort gefunden.")).color(MUTED));
                        }
                        Some(Ok(list)) => {
                            for p in list {
                                let label = format!("{}{}", p.name, p.region.as_deref().map(|r| format!(" · {r}")).unwrap_or_default());
                                if ui.add(egui::Button::new(egui::RichText::new(label).size(14.0)).frame(false)).clicked() {
                                    chosen = Some(p.clone());
                                }
                            }
                        }
                        Some(Err(e)) => {
                            ui.label(egui::RichText::new(tf("Suche fehlgeschlagen: {fehler}", &[("fehler", e)])).color(WARN));
                        }
                        None => {}
                    }
                    if place != Place::sarnen() && link(ui, t("Zurück auf Sarnen")).clicked() {
                        chosen = Some(Place::sarnen());
                    }
                });
                section(ui, t("Startseite zeigt"), &mut |ui| {
                    changed |= ui.checkbox(&mut self.cfg.home.clock, t("Uhr und Datum")).changed();
                    changed |= ui.checkbox(&mut self.cfg.home.weather, tf("Wetter für {ort}", &[("ort", &place.name)])).changed();
                    changed |= ui.checkbox(&mut self.cfg.home.recent, t("Zuletzt geöffnete Dateien")).changed();
                    if !self.cfg.recent.is_empty() && link(ui, t("Liste «Zuletzt geöffnet» leeren")).clicked() {
                        let mut cfg = core::load_config();
                        cfg.recent.clear();
                        let _ = core::save_config(&cfg);
                        self.cfg.recent.clear();
                    }
                });
                let mut types = false;
                section(ui, t("Programme und Dateien"), &mut |ui| {
                    ui.label(egui::RichText::new(tf("Datenordner: {pfad}", &[("pfad", &core::data_dir().display())])).color(SOFT));
                    ui.horizontal(|ui| {
                        if button(ui, t("Datenordner öffnen"), Look::Outline).clicked() {
                            let _ = open::that(core::data_dir());
                        }
                        if assoc::supported() && button(ui, t("Dateitypen …"), Look::Outline).clicked() {
                            types = true;
                        }
                    });
                });
                if types {
                    self.open_types();
                }
                let mut about = false;
                section(ui, t("Über"), &mut |ui| {
                    ui.label(egui::RichText::new(concat!("Werkbank ", env!("CARGO_PKG_VERSION"))).color(SOFT));
                    ui.label(egui::RichText::new(t("Programme basieren auf den Open-Source-Apps des ArtCraft-Teams (MIT / Apache 2.0). Wetterdaten: Open-Meteo.com (CC BY 4.0).")).size(12.5).color(MUTED));
                    if link(ui, t("Mehr …")).clicked() {
                        about = true;
                    }
                });
                self.about |= about;
            });
        });
        if search {
            let (places, ctx) = (self.places.clone(), ctx.clone());
            let query = {
                let mut p = places.lock().unwrap();
                p.busy = true;
                p.query.clone()
            };
            std::thread::spawn(move || {
                let r = weather::search_place(&query).map_err(|e| format!("{e:#}"));
                let mut p = places.lock().unwrap();
                p.busy = false;
                p.result = Some(r);
                ctx.request_repaint();
            });
        }
        if let Some(p) = chosen {
            self.cfg.place = Some(p);
            {
                let mut ps = self.places.lock().unwrap();
                ps.result = None;
                ps.query.clear();
            }
            changed = true;
            self.sky.lock().unwrap().data = None;
        }
        if changed {
            self.save_cfg();
            self.fetch_weather(ctx);
        }
    }
}

fn matches(chip: Chip, app: &App, st: &Status) -> bool {
    match chip {
        Chip::All => true,
        Chip::Installed => st.installed(),
        Chip::Updates => st.update(),
        c => category(app) == c,
    }
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.setup.is_some() {
            self.setup_screen(ctx);
            return;
        }
        if self.types.is_some() {
            self.types_screen(ctx);
            return;
        }
        self.tick(ctx);
        let snapshot = self.state.lock().unwrap().clone();
        if self.menu {
            self.drawer(ctx, &snapshot);
        }
        match self.view {
            View::Home => {
                self.header(ctx, None);
                self.home(ctx, &snapshot);
            }
            View::Catalog => {
                let title = if self.chip == Chip::Updates { t("Updates") } else { t("Programme entdecken") };
                self.header(ctx, Some(title));
                self.catalog(ctx, &snapshot);
            }
            View::Prefs => {
                self.header(ctx, Some(t("Einstellungen")));
                self.prefs(ctx);
            }
        }
        if self.about {
            self.about_window(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deutsche_zeit() {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        assert_eq!(long_date(d), "Sonntag, 18. Oktober");
        assert_eq!(greeting(9), "Guten Morgen");
        assert_eq!(greeting(23), "Gute Nacht");
    }

    #[test]
    fn raster() {
        assert_eq!(grid(1000.0, 124.0, 12.0, 12).0, 7);
        assert_eq!(grid(100.0, 124.0, 12.0, 12).0, 1);
        assert_eq!(grid(2000.0, 270.0, 16.0, 3).0, 3);
    }

    #[test]
    fn rubriken() {
        let app = |id: &str| -> App { toml::from_str(&format!("id = \"{id}\"\nname = \"x\"\nrepo = \"r\"")).unwrap() };
        assert!(category(&app("wordcraft")) == Chip::Office);
        assert!(category(&app("diawerk")) == Chip::Video);
        assert!(category(&app("codewerk")) == Chip::System);
        let mut x = app("neu");
        x.category = Some("foto".into());
        assert!(category(&x) == Chip::Photo);
    }
}
