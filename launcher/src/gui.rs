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

mod start;
mod widgets;

use start::{accent_round, color_button, vivid};

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
    token: String,
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

fn cat_color(c: Chip) -> Color32 {
    match c {
        Chip::Office => rgb(0x2F6FDE),
        Chip::Photo => rgb(0xD9622B),
        Chip::Video => rgb(0x7B4FD6),
        _ => rgb(0x0F8B8D),
    }
}

/// `color` zu Anteil `a` über `base` gelegt.
fn mix(color: Color32, base: Color32, a: f32) -> Color32 {
    let m = |c: u8, b: u8| (b as f32 + (c as f32 - b as f32) * a).round() as u8;
    Color32::from_rgb(m(color.r(), base.r()), m(color.g(), base.g()), m(color.b(), base.b()))
}

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

/// Stand des Selbst-Updates des Launchers.
#[derive(Default, Clone)]
enum SelfUpdate {
    #[default]
    Idle,
    Checking,
    Loading(Version, f32),
    /// Neue Version eingesetzt, wirkt nach dem Neustart.
    Ready(Version),
    Current,
    Failed(String),
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
    /// Modus «Widgets anordnen».
    arrange: bool,
    note_input: String,
    folders_edit: bool,
    cal: Arc<Mutex<widgets::CalState>>,
    cal_at: Option<Instant>,
    news: Arc<Mutex<widgets::NewsState>>,
    news_at: Option<Instant>,
    sys: Arc<Mutex<widgets::SysState>>,
    sys_started: bool,
    mail: Option<widgets::MailStatus>,
    mail_at: Option<Instant>,
    timer: widgets::TimerState,
    /// Selbst-Update des Launchers.
    self_update: Arc<Mutex<SelfUpdate>>,
    /// Schnellzugriff bearbeiten (× auf den Symbolen).
    quick_edit: bool,
    /// «Alle Programme» unter dem Schnellzugriff offen, mit Suchtext.
    show_all: bool,
    all_query: String,
    all_focus: bool,
    /// Eingabefelder der Einstellungen.
    token_input: String,
    cal_input: String,
    news_input: String,
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
                manifest: Manifest { title: "Werkbank".into(), app: Vec::new() },
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
                arrange: false,
                note_input: String::new(),
                folders_edit: false,
                cal: Arc::default(),
                cal_at: None,
                news: Arc::default(),
                news_at: None,
                sys: Arc::default(),
                sys_started: false,
                mail: None,
                mail_at: None,
                timer: Default::default(),
                quick_edit: false,
                self_update: Default::default(),
                show_all: false,
                all_query: String::new(),
                all_focus: false,
                token_input: String::new(),
                cal_input: String::new(),
                news_input: String::new(),
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
                    token: String::new(),
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

// Farben: hell (Standard) oder dunkel, Bernstein als Akzent. Die Wahl gilt für alle Programme
// (WERKBANK_THEME beim Start, thema.txt neben sprache.txt).
static DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn dark() -> bool {
    DARK.load(std::sync::atomic::Ordering::Relaxed)
}

fn pick(light: Color32, dark_c: Color32) -> Color32 {
    if dark() {
        dark_c
    } else {
        light
    }
}

fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn bg() -> Color32 {
    pick(rgb(0xF3F4F6), rgb(0x17181C))
}
fn header_bg() -> Color32 {
    pick(rgb(0xFFFFFF), rgb(0x1D1F24))
}
fn panel() -> Color32 {
    pick(rgb(0xFFFFFF), rgb(0x212329))
}
fn hover() -> Color32 {
    pick(rgb(0xECEEF2), rgb(0x2A2D35))
}
fn chip() -> Color32 {
    pick(rgb(0xE6E9EE), rgb(0x343842))
}
fn border() -> Color32 {
    pick(rgb(0xE1E4EA), rgb(0x32353D))
}
fn outline() -> Color32 {
    pick(rgb(0xC3C8D0), rgb(0x454953))
}
fn text() -> Color32 {
    pick(rgb(0x17181C), rgb(0xECEEF2))
}
fn soft() -> Color32 {
    pick(rgb(0x2E323A), rgb(0xD6D9E0))
}
fn muted() -> Color32 {
    pick(rgb(0x5B616E), rgb(0xA9AEB9))
}
/// Akzent als Fläche (Knöpfe, Abzeichen); Schrift darauf immer INK.
const ACCENT: Color32 = Color32::from_rgb(0xF2, 0xB5, 0x44);
const INK: Color32 = Color32::from_rgb(0x17, 0x18, 0x1C);
/// Akzent als Schrift auf dem Hintergrund (im hellen Design dunkler, damit lesbar).
fn accent_text() -> Color32 {
    pick(rgb(0x9A5B00), rgb(0xF2B544))
}
fn warn() -> Color32 {
    pick(rgb(0xB42318), rgb(0xF08A6A))
}
fn cloud() -> Color32 {
    pick(rgb(0x9AA3B2), rgb(0xD6D9E0))
}

/// Gespeichertes Design lesen (Umgebung vor Datei, Standard hell).
fn load_dark() -> bool {
    let parse = |v: &str| match v.trim().to_lowercase().as_str() {
        "dunkel" | "dark" => Some(true),
        "hell" | "light" => Some(false),
        _ => None,
    };
    std::env::var("WERKBANK_THEME")
        .ok()
        .and_then(|v| parse(&v))
        .or_else(|| theme_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|v| parse(&v)))
        .unwrap_or(false)
}

/// Beim Programmstart: gespeichertes Design übernehmen (auch für `--open` ohne Oberfläche).
pub fn init_theme() {
    DARK.store(load_dark(), std::sync::atomic::Ordering::Relaxed);
}

fn theme_file() -> Option<std::path::PathBuf> {
    sprache::werkbank_file().map(|p| p.with_file_name("thema.txt"))
}

pub fn theme_code() -> &'static str {
    if dark() {
        "dunkel"
    } else {
        "hell"
    }
}

fn theme(ctx: &egui::Context) {
    let mut v = if dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = bg();
    v.window_fill = panel();
    v.extreme_bg_color = pick(rgb(0xFFFFFF), rgb(0x121316));
    v.window_stroke = Stroke::new(1.0, border());
    v.override_text_color = Some(text());
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, INK);
    v.widgets.hovered.weak_bg_fill = hover();
    v.widgets.inactive.weak_bg_fill = panel();
    ctx.set_theme(if dark() { egui::Theme::Dark } else { egui::Theme::Light });
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
        Look::Primary => (text(), bg(), text()),
        Look::Accent => (ACCENT, INK, ACCENT),
        Look::Outline => (Color32::TRANSPARENT, text(), outline()),
    };
    let text = egui::RichText::new(label).color(fg).size(14.5);
    let text = if look == Look::Outline { text } else { text.strong() };
    ui.add(egui::Button::new(text).fill(fill).stroke(Stroke::new(1.0, stroke)).corner_radius(10u8).min_size(egui::vec2(0.0, 38.0)))
}

fn link(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).size(13.0).color(muted())).frame(false))
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
    let color = app.color.as_deref().and_then(|c| Color32::from_hex(c).ok()).unwrap_or(outline());
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
    Sun,
    Moon,
    Search,
    Plus,
    Down,
    Up,
    Clock,
    Cloud,
    Mail,
    Calendar,
    Timer,
    Cpu,
    Ram,
    Battery,
    News,
    Note,
    History,
    Play,
    Pause,
    Stop,
    Star,
    Pencil,
    Pin,
    Link,
    External,
    FolderOpen,
    Rss,
    Layout,
    Layers,
    Apps,
    Check,
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
        Glyph::Sun => {
            p.circle_stroke(c, 4.0 * u, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                p.line_segment([c + egui::vec2(a.cos(), a.sin()) * 7.0 * u, c + egui::vec2(a.cos(), a.sin()) * 9.5 * u], s);
            }
        }
        Glyph::Moon => {
            let pts: Vec<Pos2> = (0..=28)
                .map(|i| {
                    let a = 0.9 + i as f32 / 28.0 * 4.5;
                    Pos2::new(c.x + a.cos() * 8.0 * u, c.y + a.sin() * 8.0 * u)
                })
                .collect();
            let first = pts[0];
            let last = pts[pts.len() - 1];
            p.add(egui::Shape::line(pts, s));
            p.add(egui::Shape::QuadraticBezier(egui::epaint::QuadraticBezierShape::from_points_stroke([last, c + egui::vec2(-1.0 * u, -1.0 * u), first], false, Color32::TRANSPARENT, s)));
        }
        Glyph::Gear => {
            p.circle_stroke(c, 3.0 * u, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                p.line_segment([c + egui::vec2(a.cos(), a.sin()) * 6.0 * u, c + egui::vec2(a.cos(), a.sin()) * 9.5 * u], s);
            }
        }
        _ => glyph_more(p, r, g, color),
    }
}

/// Bogen um (cx, cy) mit Radius r von Winkel a0 bis a1 (Grad, im Uhrzeigersinn, 0° = rechts).
fn arc(p: &egui::Painter, r: Rect, cx: f32, cy: f32, rad: f32, a0: f32, a1: f32, s: Stroke) {
    let u = r.width() / 24.0;
    let n = (((a1 - a0).abs() / 12.0).ceil() as usize).max(2);
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            Pos2::new(r.left() + (cx + rad * a.cos()) * u, r.top() + (cy + rad * a.sin()) * u)
        })
        .collect();
    p.add(egui::Shape::line(pts, s));
}

/// Weitere Symbole für Widgets und Knöpfe (Raster 24 × 24 wie die Strichsymbole oben).
fn glyph_more(p: &egui::Painter, r: Rect, g: Glyph, color: Color32) {
    let s = Stroke::new(1.8, color);
    let u = r.width() / 24.0;
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * u, r.top() + y * u);
    let line = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), s));
    };
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32, round: f32| {
        p.rect_stroke(Rect::from_min_max(at(x0, y0), at(x1, y1)), round * u, s, egui::StrokeKind::Middle);
    };
    match g {
        Glyph::Search => {
            p.circle_stroke(at(11.0, 11.0), 6.0 * u, s);
            line(&[(15.5, 15.5), (20.0, 20.0)]);
        }
        Glyph::Plus => {
            line(&[(12.0, 5.0), (12.0, 19.0)]);
            line(&[(5.0, 12.0), (19.0, 12.0)]);
        }
        Glyph::Down => line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Glyph::Up => line(&[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]),
        Glyph::Clock => {
            p.circle_stroke(at(12.0, 12.0), 8.5 * u, s);
            line(&[(12.0, 7.5), (12.0, 12.0), (15.0, 14.0)]);
        }
        Glyph::Cloud => {
            arc(p, r, 7.0, 15.0, 3.0, 90.0, 270.0, s);
            arc(p, r, 11.5, 12.5, 5.0, 190.0, 330.0, s);
            arc(p, r, 17.0, 14.5, 3.5, 250.0, 450.0, s);
            line(&[(17.0, 18.0), (7.0, 18.0)]);
        }
        Glyph::Mail => {
            rect(3.5, 5.5, 20.5, 18.5, 2.0);
            line(&[(4.0, 7.0), (12.0, 13.0), (20.0, 7.0)]);
        }
        Glyph::Calendar => {
            rect(4.0, 5.0, 20.0, 20.0, 2.0);
            line(&[(4.0, 10.0), (20.0, 10.0)]);
            line(&[(8.0, 3.0), (8.0, 7.0)]);
            line(&[(16.0, 3.0), (16.0, 7.0)]);
        }
        Glyph::Timer => {
            p.circle_stroke(at(12.0, 13.0), 7.5 * u, s);
            line(&[(12.0, 9.5), (12.0, 13.0), (14.5, 14.5)]);
            line(&[(10.0, 3.0), (14.0, 3.0)]);
        }
        Glyph::Cpu => {
            rect(7.0, 7.0, 17.0, 17.0, 1.5);
            for v in [10.0, 14.0] {
                line(&[(v, 3.0), (v, 7.0)]);
                line(&[(v, 17.0), (v, 21.0)]);
                line(&[(3.0, v), (7.0, v)]);
                line(&[(17.0, v), (21.0, v)]);
            }
        }
        Glyph::Ram => {
            rect(3.0, 7.0, 21.0, 17.0, 1.5);
            for x in [7.0, 11.0, 15.0] {
                line(&[(x, 10.0), (x, 14.0)]);
                line(&[(x, 17.0), (x, 19.5)]);
            }
        }
        Glyph::Battery => {
            rect(3.0, 8.0, 19.0, 17.0, 2.0);
            line(&[(21.0, 11.0), (21.0, 14.0)]);
            p.rect_filled(Rect::from_min_max(at(5.5, 10.5), at(12.0, 14.5)), 1.0, color);
        }
        Glyph::News => {
            rect(4.0, 5.0, 17.0, 19.0, 1.5);
            line(&[(17.0, 9.0), (20.0, 9.0), (20.0, 17.5), (18.5, 19.0), (17.0, 19.0)]);
            line(&[(7.5, 9.0), (13.5, 9.0)]);
            line(&[(7.5, 12.5), (13.5, 12.5)]);
            line(&[(7.5, 16.0), (11.5, 16.0)]);
        }
        Glyph::Note => {
            line(&[(5.0, 4.0), (15.0, 4.0), (19.0, 8.0), (19.0, 20.0), (5.0, 20.0), (5.0, 4.0)]);
            line(&[(15.0, 4.0), (15.0, 8.0), (19.0, 8.0)]);
            line(&[(8.5, 12.0), (15.5, 12.0)]);
            line(&[(8.5, 15.5), (13.5, 15.5)]);
        }
        Glyph::History => {
            arc(p, r, 12.0, 12.0, 8.0, 200.0, 520.0, s);
            line(&[(3.0, 5.5), (4.6, 9.3), (8.4, 7.8)]);
            line(&[(12.0, 8.0), (12.0, 12.0), (15.0, 14.0)]);
        }
        Glyph::Play => {
            p.add(egui::Shape::convex_polygon(vec![at(8.0, 5.5), at(18.5, 12.0), at(8.0, 18.5)], color, Stroke::NONE));
        }
        Glyph::Pause => {
            p.rect_filled(Rect::from_min_max(at(7.0, 5.5), at(10.5, 18.5)), 1.0, color);
            p.rect_filled(Rect::from_min_max(at(13.5, 5.5), at(17.0, 18.5)), 1.0, color);
        }
        Glyph::Stop => {
            p.rect_filled(Rect::from_min_max(at(6.5, 6.5), at(17.5, 17.5)), 2.0 * u, color);
        }
        Glyph::Star => {
            let pts: Vec<(f32, f32)> = (0..=10)
                .map(|i| {
                    let a = (-90.0 + i as f32 * 36.0_f32).to_radians();
                    let rad = if i % 2 == 0 { 8.5 } else { 3.8 };
                    (12.0 + rad * a.cos(), 12.6 + rad * a.sin())
                })
                .collect();
            line(&pts);
        }
        Glyph::Pencil => {
            line(&[(4.0, 20.0), (8.0, 20.0), (19.0, 9.0), (15.0, 5.0), (4.0, 16.0), (4.0, 20.0)]);
            line(&[(13.5, 6.5), (17.5, 10.5)]);
        }
        Glyph::Pin => {
            arc(p, r, 12.0, 10.0, 6.0, 145.0, 395.0, s);
            line(&[(16.9, 13.4), (12.0, 21.0), (7.1, 13.4)]);
            p.circle_stroke(at(12.0, 10.0), 2.0 * u, s);
        }
        Glyph::Link => {
            arc(p, r, 8.5, 15.5, 4.0, 15.0, 255.0, s);
            arc(p, r, 15.5, 8.5, 4.0, 195.0, 435.0, s);
            line(&[(10.0, 14.0), (14.0, 10.0)]);
        }
        Glyph::External => {
            line(&[(14.0, 4.0), (20.0, 4.0), (20.0, 10.0)]);
            line(&[(20.0, 4.0), (11.0, 13.0)]);
            line(&[(18.0, 14.0), (18.0, 20.0), (4.0, 20.0), (4.0, 6.0), (10.0, 6.0)]);
        }
        Glyph::FolderOpen => {
            line(&[(3.0, 19.0), (3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (19.0, 8.0), (19.0, 11.0)]);
            line(&[(3.0, 19.0), (6.0, 11.0), (22.0, 11.0), (19.0, 19.0), (3.0, 19.0)]);
        }
        Glyph::Rss => {
            arc(p, r, 5.0, 19.0, 7.0, 270.0, 360.0, s);
            arc(p, r, 5.0, 19.0, 13.0, 270.0, 360.0, s);
            p.circle_filled(at(6.0, 18.0), 1.6 * u, color);
        }
        Glyph::Layout => {
            rect(3.5, 4.0, 10.5, 11.0, 1.5);
            rect(13.5, 4.0, 20.5, 8.0, 1.5);
            rect(13.5, 11.0, 20.5, 20.0, 1.5);
            rect(3.5, 14.0, 10.5, 20.0, 1.5);
        }
        Glyph::Layers => {
            line(&[(12.0, 4.0), (20.0, 8.0), (12.0, 12.0), (4.0, 8.0), (12.0, 4.0)]);
            line(&[(4.0, 12.0), (12.0, 16.0), (20.0, 12.0)]);
            line(&[(4.0, 16.0), (12.0, 20.0), (20.0, 16.0)]);
        }
        Glyph::Apps => {
            for (x, y) in [(4.0, 4.0), (14.0, 4.0), (4.0, 14.0), (14.0, 14.0)] {
                rect(x, y, x + 6.0, y + 6.0, 1.5);
            }
        }
        Glyph::Check => line(&[(5.0, 12.5), (10.0, 17.5), (19.0, 7.0)]),
        _ => {}
    }
}

fn glyph_button(ui: &mut egui::Ui, g: Glyph, hint: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 10.0, hover());
    }
    glyph(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(22.0)), g, text());
    resp.on_hover_text(hint)
}

/// Eine Karte fester Grösse: Rahmen zeichnen, Inhalt in einem Kind-Bereich.
fn card<R>(ui: &mut egui::Ui, size: Vec2, border: Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> (egui::Response, R) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect(rect, 16.0, panel(), Stroke::new(1.0, border), egui::StrokeKind::Inside);
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
        let b = egui::Button::new(egui::RichText::new(label).size(13.0).color(if active { INK } else { soft() }))
            .fill(if active { ACCENT } else { panel() })
            .stroke(Stroke::new(1.0, if active { ACCENT } else { border() }))
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
            let b = egui::Button::new(egui::RichText::new(label).size(14.0).color(if on { INK } else { soft() }))
                .fill(if on { ACCENT } else { panel() })
                .stroke(Stroke::new(1.0, if on { ACCENT } else { border() }))
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

#[cfg_attr(not(test), allow(dead_code))]
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
        if online {
            self.check_self_update(ctx, true);
        }
        if let Some(dir) = bundle {
            self.start_bundle(ctx, dir, online);
        }
        // Schritt 2 der Ersteinrichtung: Dateitypen (nur, wo das System es kann).
        if assoc::supported() && !self.cfg.types_asked {
            self.types = Some(Types { choice: assoc::all(&self.manifest), message: None, first_run: true });
        }
    }

    /// Nach einer neueren Werkbank suchen; mit `install` gleich laden und einsetzen.
    fn check_self_update(&mut self, ctx: &egui::Context, install: bool) {
        let state = self.self_update.clone();
        if let Ok(mut s) = state.lock() {
            if matches!(*s, SelfUpdate::Checking | SelfUpdate::Loading(..) | SelfUpdate::Ready(_)) {
                return;
            }
            *s = SelfUpdate::Checking;
        }
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let set = |v: SelfUpdate| {
                if let Ok(mut s) = state.lock() {
                    *s = v;
                }
                ctx.request_repaint();
            };
            match core::launcher_update_available() {
                Ok(None) => set(SelfUpdate::Current),
                Ok(Some(v)) if install => {
                    set(SelfUpdate::Loading(v, 0.0));
                    let progress = |_: &str, f: f32| {
                        if let Ok(mut s) = state.lock() {
                            *s = SelfUpdate::Loading(v, f);
                        }
                        ctx.request_repaint();
                    };
                    match core::update_launcher(v, &progress) {
                        Ok(()) => set(SelfUpdate::Ready(v)),
                        Err(e) => set(SelfUpdate::Failed(format!("{e:#}"))),
                    }
                }
                Ok(Some(v)) => set(SelfUpdate::Loading(v, 0.0)),
                Err(e) => set(SelfUpdate::Failed(format!("{e:#}"))),
            }
        });
    }

    /// Hinweis in der Kopfzeile, solange ein Update lädt oder bereitsteht.
    fn self_update_bar(&mut self, ui: &mut egui::Ui) {
        let state = self.self_update.lock().map(|s| s.clone()).unwrap_or_default();
        match state {
            SelfUpdate::Loading(v, f) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new(tf("Neue Werkbank {v} wird geladen … {p} %", &[("v", &fmt_version(v)), ("p", &format!("{:.0}", f * 100.0))])).color(soft()));
                });
            }
            SelfUpdate::Ready(v) => {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    glyph(ui.painter(), r, Glyph::Update, vivid(rgb(0xE39B0B)));
                    ui.label(egui::RichText::new(tf("Werkbank {v} ist installiert und gilt nach einem Neustart.", &[("v", &fmt_version(v))])).color(text()));
                    if button(ui, t("Jetzt neu starten"), Look::Accent).clicked() {
                        if let Err(e) = core::restart_launcher() {
                            self.notice = Some(format!("{e:#}"));
                        }
                    }
                });
            }
            _ => {}
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

    /// Hell/Dunkel umschalten: sofort im Launcher, für Programme beim nächsten Start.
    fn set_dark(&mut self, ctx: &egui::Context, d: bool) {
        DARK.store(d, std::sync::atomic::Ordering::Relaxed);
        theme(ctx);
        let saved = theme_file().ok_or_else(|| anyhow::anyhow!("kein Einstellungsordner")).and_then(|p| {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&p, theme_code())?;
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
        if !self.widget_list().iter().any(|w| w.kind == "wetter") {
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
        self.tick_widgets(ctx);
        ctx.request_repaint_after(Duration::from_secs(1));
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
        if view == View::Prefs {
            self.cal_input = self.cfg.calendar_url.clone().unwrap_or_default();
            self.news_input = self.cfg.news_url.clone().unwrap_or_default();
        }
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
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::symmetric(48, 40))).show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                ui.set_max_width(640.0);
                ui.horizontal(|ui| {
                    logo(ui, &mut self.textures, 56.0);
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(t("Willkommen in der Werkbank")).size(24.0).strong());
                        ui.label(egui::RichText::new(t("Einmalige Einrichtung")).size(13.0).color(muted()));
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
                    .color(muted()),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 140.0;
                    ui.add_sized([w, 36.0], egui::TextEdit::singleline(&mut setup.path).font(egui::FontId::proportional(15.0)));
                    if button(ui, t("Durchsuchen …"), Look::Outline).clicked() {
                        browse = true;
                    }
                });
                if !core::has_token() {
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new(t("Zugang zu GitHub")).size(16.0).strong());
                    ui.label(egui::RichText::new(t("Nur nötig, wenn das Werkbank-Repository privat ist: persönlicher Zugriffsschlüssel (fine-grained token) mit Lesezugriff auf «Contents» dieses Repositorys.")).size(13.0).color(muted()));
                    ui.add_sized([ui.available_width(), 34.0], egui::TextEdit::singleline(&mut setup.token).password(true).hint_text("github_pat_…"));
                }
                if let Some(e) = &setup.error {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(e).color(warn()));
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
                        let tok = Some(setup.token.trim().to_string()).filter(|s| !s.is_empty());
                        if tok.is_some() {
                            let mut cfg = core::load_config();
                            cfg.github_token = tok.clone();
                            let _ = core::save_config(&cfg);
                            core::set_token(tok);
                        }
                        let dark_now = dark();
                        self.set_dark(ctx, dark_now);
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
        egui::TopBottomPanel::top("typen-kopf").frame(egui::Frame::new().fill(header_bg()).inner_margin(egui::Margin::symmetric(24, 14))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if !types.first_run && glyph_button(ui, Glyph::Back, t("Zurück")).clicked() {
                    close = true;
                }
                ui.vertical(|ui| {
                    let title = if types.first_run { t("Einrichtung: Dateitypen") } else { t("Dateitypen") };
                    ui.label(egui::RichText::new(title).size(22.0).strong());
                    ui.label(egui::RichText::new(t("Angehakte Dateitypen lassen sich per Rechtsklick → «Öffnen mit» in der Werkbank öffnen.")).size(12.5).color(muted()));
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
        egui::TopBottomPanel::bottom("typen-fuss").frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::symmetric(24, 10))).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t("Welches Programm Standard ist, legt Windows fest: Rechtsklick → «Öffnen mit» → «Immer», oder hier:")).size(12.5).color(muted()));
                if link(ui, t("Standard-Apps in Windows öffnen")).clicked() {
                    assoc::open_system_settings();
                }
            });
            if let Some((m, ok)) = &types.message {
                ui.label(egui::RichText::new(m).color(if *ok { accent_text() } else { warn() }));
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::symmetric(24, 18))).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for app in &apps {
                    let exts = assoc::offered(app);
                    let chosen = types.choice.entry(app.id.clone()).or_default();
                    egui::Frame::new().fill(panel()).stroke(Stroke::new(1.0, border())).corner_radius(12u8).inner_margin(14.0).show(ui, |ui| {
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
        let mut toggle_dark = false;
        egui::TopBottomPanel::top("kopf").frame(egui::Frame::new().fill(header_bg()).stroke(Stroke::new(1.0, border())).inner_margin(egui::Margin { left: 18, right: 24, top: 12, bottom: 10 })).show(ctx, |ui| {
            ui.horizontal(|ui| {
                match title {
                    None => {
                        if glyph_button(ui, Glyph::Menu, t("Menü")).clicked() {
                            self.menu = !self.menu;
                        }
                        logo(ui, &mut self.textures, 34.0);
                    }
                    Some(title) => {
                        if glyph_button(ui, Glyph::Back, t("Zur Startseite")).clicked() {
                            self.go(View::Home);
                        }
                        ui.label(egui::RichText::new(title).size(21.0).strong());
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if self.view == View::Home && accent_round(ui, Glyph::Plus, t("Programme hinzufügen")).clicked() {
                        self.open_catalog(Chip::All);
                    }
                    if self.view != View::Home && glyph_button(ui, Glyph::Menu, t("Menü")).clicked() {
                        self.menu = !self.menu;
                    }
                    let (g, hint, c) = if dark() { (Glyph::Sun, t("Helles Design"), rgb(0xE39B0B)) } else { (Glyph::Moon, t("Dunkles Design"), rgb(0x7B4FD6)) };
                    if color_button(ui, g, hint, c, 42.0, false).clicked() {
                        toggle_dark = true;
                    }
                    if self.view == View::Home && color_button(ui, Glyph::Search, t("Programm suchen"), rgb(0x2F6FDE), 42.0, false).clicked() {
                        self.show_all = true;
                        self.all_focus = true;
                    }
                });
            });
            self.self_update_bar(ui);
            let mut clear = false;
            if let Some(n) = &self.notice {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(n).color(warn()));
                    if link(ui, t("Ausblenden")).clicked() {
                        clear = true;
                    }
                });
            }
            if clear {
                self.notice = None;
            }
        });
        if toggle_dark {
            self.set_dark(ctx, !dark());
        }
    }

    fn drawer(&mut self, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        let installed = self.manifest.app.iter().filter(|a| snapshot.get(&a.id).is_some_and(Status::installed)).count();
        let total = self.manifest.app.len();
        let updates = snapshot.values().filter(|s| s.update()).count();
        let changed = false;
        let mut toggle_dark = false;
        let panel = egui::SidePanel::left("menue")
            .exact_width(320.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(header_bg()).stroke(Stroke::new(1.0, border())).inner_margin(egui::Margin::symmetric(14, 18)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    logo(ui, &mut self.textures, 40.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(t("Werkbank")).size(20.0).strong());
                        ui.label(egui::RichText::new(tf("{installed} von {total} Programmen installiert", &[("installed", &installed), ("total", &total)])).size(12.0).color(muted()));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if glyph_button(ui, Glyph::Close, t("Menü schliessen")).clicked() {
                            self.menu = false;
                        }
                    });
                });
                ui.add_space(12.0);
                let mut item = |ui: &mut egui::Ui, g: Glyph, color: u32, label: &str, badge: Option<(usize, bool)>, active: bool| -> bool {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 48.0), Sense::click());
                    if active || resp.hovered() {
                        ui.painter().rect_filled(rect, 10.0, hover());
                    }
                    let gr = Rect::from_center_size(Pos2::new(rect.left() + 26.0, rect.center().y), Vec2::splat(22.0));
                    glyph(ui.painter(), gr, g, vivid(rgb(color)));
                    ui.painter().text(
                        Pos2::new(rect.left() + 52.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(15.0),
                        text(),
                    );
                    if let Some((n, hot)) = badge.filter(|(n, _)| *n > 0) {
                        let text = n.to_string();
                        let r = Rect::from_center_size(Pos2::new(rect.right() - 26.0, rect.center().y), egui::vec2(12.0 + 8.0 * text.len() as f32, 22.0));
                        ui.painter().rect_filled(r, 11.0, if hot { ACCENT } else { chip() });
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(12.0), if hot { INK } else { soft() });
                    }
                    resp.clicked()
                };
                if item(ui, Glyph::Home, 0x2F6FDE, t("Startseite"), None, self.view == View::Home) {
                    self.go(View::Home);
                }
                if item(ui, Glyph::Grid, 0xD9622B, t("Programme entdecken"), Some((total - installed, false)), self.view == View::Catalog && self.chip != Chip::Updates) {
                    self.open_catalog(Chip::All);
                }
                if item(ui, Glyph::Update, 0xE39B0B, t("Updates"), Some((updates, true)), self.view == View::Catalog && self.chip == Chip::Updates) {
                    self.open_catalog(Chip::Updates);
                }
                if assoc::supported() && item(ui, Glyph::File, 0x7B4FD6, t("Dateitypen"), None, false) {
                    self.open_types();
                }
                if item(ui, Glyph::Folder, 0xB7791F, t("Datenordner öffnen"), None, false) {
                    let _ = open::that(core::data_dir());
                }
                if item(ui, Glyph::Globe, 0x0F8B8D, t("Browser-Übersicht"), None, false) {
                    if let Some(a) = self.web_addr() {
                        let _ = open::that(a);
                    }
                }
                if item(ui, Glyph::Gear, 0x5B616E, t("Einstellungen"), None, self.view == View::Prefs) {
                    self.go(View::Prefs);
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
                if item(ui, Glyph::Layout, 0x0F8B8D, t("Widgets anordnen"), None, self.arrange) {
                    self.arrange = true;
                    self.cfg.home.bar_open = true;
                    self.go(View::Home);
                }
                let (g, label) = if dark() { (Glyph::Sun, t("Helles Design")) } else { (Glyph::Moon, t("Dunkles Design")) };
                if item(ui, g, if dark() { 0xE39B0B } else { 0x7B4FD6 }, label, None, false) {
                    toggle_dark = true;
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(concat!("Werkbank ", env!("CARGO_PKG_VERSION"), " ·")).size(12.0).color(muted()));
                        if link(ui, t("Über Werkbank")).clicked() {
                            self.about = true;
                        }
                    });
                });
            });
        if changed {
            self.save_cfg();
        }
        if toggle_dark {
            self.set_dark(ctx, !dark());
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
                        ui.label(egui::RichText::new(t("Programme installieren, starten und einzeln aktualisieren")).color(muted()));
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
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin { left: 24, right: 24, top: 14, bottom: 18 })).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                self.widget_bar(ui, ctx, snapshot);
                ui.add_space(18.0);
                if mine.is_empty() {
                    self.empty_home(ui);
                    return;
                }
                self.quick_access(ui, ctx, snapshot, &mine);
                if self.show_all {
                    ui.add_space(14.0);
                    self.all_programs(ui, ctx, snapshot, &mine);
                }
            });
        });
    }

    /// Eine Kachel je Kategorie; ein Klick auf ein Icon startet das Programm.
    fn category_tiles(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>, mine: &[App]) {
        let groups: Vec<(Chip, Vec<App>)> = CATEGORIES
            .iter()
            .map(|c| (*c, mine.iter().filter(|a| category(a) == *c).cloned().collect::<Vec<_>>()))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        let gap = 16.0;
        let cols = if ui.available_width() >= 1000.0 { 2 } else { 1 };
        let w = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
        for row in groups.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (c, apps) in row {
                    self.category_card(ui, ctx, snapshot, *c, apps, w);
                }
            });
            ui.add_space(gap - 8.0);
        }
    }

    fn category_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>, c: Chip, apps: &[App], w: f32) {
        let color = cat_color(c);
        let fill = mix(color, panel(), if dark() { 0.14 } else { 0.08 });
        let stroke = mix(color, border(), 0.35);
        egui::Frame::new().fill(fill).stroke(Stroke::new(1.0, stroke)).corner_radius(20u8).inner_margin(egui::Margin { left: 16, right: 16, top: 14, bottom: 10 }).show(ui, |ui| {
          ui.vertical(|ui| {
            ui.set_width(w - 34.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 5.0, color);
                ui.label(egui::RichText::new(c.label()).size(17.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(apps.len().to_string()).size(13.0).color(muted()));
                });
            });
            ui.add_space(6.0);
            let cell = if self.cfg.home.names { 96.0 } else { 84.0 };
            let per_row = ((ui.available_width() + 4.0) / cell).floor().max(1.0) as usize;
            let cw = ui.available_width() / per_row as f32;
            for chunk in apps.chunks(per_row) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for app in chunk {
                        let st = snapshot.get(&app.id).cloned().unwrap_or_default();
                        self.icon_button(ui, ctx, app, &st, cw);
                    }
                });
            }
          });
        });
    }

    /// Icon mit Namen; Klick startet, Rechtsklick zeigt das Menü des Programms.
    fn icon_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, app: &App, st: &Status, w: f32) {
        let update = st.update();
        let names = self.cfg.home.names;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, if names { 98.0 } else { 80.0 }), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect.shrink(4.0), 14.0, hover());
        }
        let ir = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 40.0), Vec2::splat(56.0));
        paint_icon(ui, &mut self.textures, app, ir);
        let quick = self.in_quick(&app.id);
        let p = ui.painter();
        if names {
            let mut job = egui::text::LayoutJob::simple_singleline(app.name.clone(), egui::FontId::proportional(12.5), soft());
            job.wrap = egui::text::TextWrapping::truncate_at_width(w - 6.0);
            let galley = p.layout_job(job);
            p.galley(Pos2::new(rect.center().x - galley.size().x / 2.0, ir.bottom() + 6.0), galley, soft());
        }
        if quick {
            let c = Pos2::new(ir.right() - 1.0, ir.bottom() - 1.0);
            p.circle_filled(c, 9.0, ACCENT);
            glyph(p, Rect::from_center_size(c, Vec2::splat(13.0)), Glyph::Star, INK);
        }
        if update {
            p.circle_filled(Pos2::new(ir.right() - 2.0, ir.top() + 2.0), 6.0, ACCENT);
            p.circle_stroke(Pos2::new(ir.right() - 2.0, ir.top() + 2.0), 6.0, Stroke::new(2.0, panel()));
        }
        if let Some((_, frac)) = &st.busy {
            let bar = Rect::from_min_size(Pos2::new(rect.left() + 10.0, rect.bottom() - 6.0), egui::vec2(w - 20.0, 4.0));
            p.rect_filled(bar, 2.0, border());
            p.rect_filled(Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac.clamp(0.02, 1.0), 4.0)), 2.0, ACCENT);
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let mut hint = format!("{}\n{}", app.name, app.about_text());
        if let Some(v) = st.current() {
            hint += &format!("\n{}", tf("Version {v}", &[("v", &fmt_version(v))]));
        }
        if let (true, Some(l)) = (update, st.latest) {
            hint += &format!(" · {}", tf("neu: {v}", &[("v", &fmt_version(l))]));
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
            if ui.button(if quick { t("Aus dem Schnellzugriff") } else { t("In den Schnellzugriff") }).clicked() {
                self.toggle_quick(&app.id);
                ui.close_menu();
            }
            if ui.button(t("Entfernen")).clicked() {
                self.remove(app);
                ui.close_menu();
            }
        });
    }

    fn empty_home(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().fill(panel()).stroke(Stroke::new(1.0, border())).corner_radius(16u8).inner_margin(28.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(t("Noch keine Programme installiert")).size(18.0).strong());
                ui.label(egui::RichText::new(t("Programme für Büro, Bild, Video, Ton und mehr – einfach auswählen und installieren.")).color(muted()));
                ui.add_space(8.0);
                if button(ui, t("Programme entdecken"), Look::Accent).clicked() {
                    self.open_catalog(Chip::All);
                }
            });
        });
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
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin { left: 24, right: 24, top: 10, bottom: 18 })).show(ctx, |ui| {
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
                    let text = egui::RichText::new(format!("{} ({n})", chip.label())).size(13.5).color(if on { INK } else { soft() });
                    let b = egui::Button::new(if on { text.strong() } else { text })
                        .fill(if on { ACCENT } else { panel() })
                        .stroke(Stroke::new(1.0, if on { ACCENT } else { border() }))
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
                        ui.label(egui::RichText::new(msg).size(16.0).color(muted()));
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
        let border = if update || st.busy.is_some() { ACCENT } else { border() };
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
                    ui.label(egui::RichText::new(line).size(12.5).color(if update { accent_text() } else { muted() }));
                });
            });
            ui.add_space(4.0);
            let mut job = egui::text::LayoutJob::simple(app.about_text().to_string(), egui::FontId::proportional(13.0), soft(), ui.available_width());
            job.wrap.max_rows = 2;
            ui.label(job);
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                if let Some((msg, frac)) = &st.busy {
                    ui.add(egui::ProgressBar::new(*frac).desired_height(6.0).fill(ACCENT));
                    ui.label(egui::RichText::new(msg).size(12.5).color(muted()));
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
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin { left: 24, right: 24, top: 10, bottom: 18 })).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(720.0);
                let section = |ui: &mut egui::Ui, title: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
                    egui::Frame::new().fill(panel()).stroke(Stroke::new(1.0, border())).corner_radius(14u8).inner_margin(18.0).show(ui, |ui| {
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
                    ui.label(egui::RichText::new(t("Gilt für die Werkbank und alle Programme; offene Programme beim nächsten Start. Klangwerk und Schauwerk gibt es nur auf Englisch, einige Programme sind nur teilweise übersetzt.")).size(12.5).color(muted()));
                });
                if let Some(l) = lang {
                    self.set_language(l);
                }
                let place = self.place();
                section(ui, t("Ort für das Wetter"), &mut |ui| {
                    ui.label(egui::RichText::new(format!("{}{}", place.name, place.region.as_deref().map(|r| format!(" · {r}")).unwrap_or_default())).color(soft()));
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
                            ui.label(egui::RichText::new(t("Kein Ort gefunden.")).color(muted()));
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
                            ui.label(egui::RichText::new(tf("Suche fehlgeschlagen: {fehler}", &[("fehler", e)])).color(warn()));
                        }
                        None => {}
                    }
                    if place != Place::sarnen() && link(ui, t("Zurück auf Sarnen")).clicked() {
                        chosen = Some(Place::sarnen());
                    }
                });
                let mut theme_pick = None;
                section(ui, t("Design"), &mut |ui| {
                    if let Some(v) = segmented(ui, dark(), t("Dunkel"), t("Hell")) {
                        theme_pick = Some(v);
                    }
                    ui.label(egui::RichText::new(t("Gilt für die Werkbank und alle Programme; offene Programme beim nächsten Start.")).size(12.5).color(muted()));
                });
                if let Some(v) = theme_pick {
                    self.set_dark(ctx, v);
                }
                let mut look_changed = false;
                section(ui, t("Aussehen der Startseite"), &mut |ui| {
                    look_changed = self.look_settings(ui);
                });
                if look_changed {
                    self.save_cfg();
                }
                let mut arrange = false;
                let mut widget_save = false;
                section(ui, t("Widgets"), &mut |ui| {
                    ui.label(egui::RichText::new(t("Diese Widgets stehen auf der Startseite. Grösse und Reihenfolge änderst du mit «Widgets anordnen».")).size(12.5).color(muted()));
                    ui.add_space(6.0);
                    self.widget_settings(ui);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if button(ui, t("Widgets anordnen"), Look::Outline).clicked() {
                            arrange = true;
                        }
                        if link(ui, t("Vorgabe wiederherstellen")).clicked() {
                            self.set_widgets(Vec::new());
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(t("Kalender für «Termine» (ICS-Adresse, z. B. «Privatadresse im iCal-Format» aus Google Kalender oder ein veröffentlichter Outlook-Kalender)")).size(13.0));
                    ui.add(egui::TextEdit::singleline(&mut self.cal_input).hint_text("https://…/basic.ics").desired_width(f32::INFINITY));
                    ui.label(egui::RichText::new(t("Nachrichten-Quelle (RSS-Adresse, leer = SRF News)")).size(13.0));
                    ui.add(egui::TextEdit::singleline(&mut self.news_input).hint_text(widgets::SRF_NEWS).desired_width(f32::INFINITY));
                    if button(ui, t("Übernehmen"), Look::Outline).clicked() {
                        widget_save = true;
                    }
                    if !self.cfg.recent.is_empty() && link(ui, t("Liste «Zuletzt geöffnet» leeren")).clicked() {
                        let mut cfg = core::load_config();
                        cfg.recent.clear();
                        let _ = core::save_config(&cfg);
                        self.cfg.recent.clear();
                    }
                });
                if arrange {
                    self.arrange = true;
                    self.go(View::Home);
                }
                if widget_save {
                    let mut cfg = core::load_config();
                    cfg.calendar_url = Some(self.cal_input.trim().to_string()).filter(|s| !s.is_empty());
                    cfg.news_url = Some(self.news_input.trim().to_string()).filter(|s| !s.is_empty());
                    let _ = core::save_config(&cfg);
                    self.cfg.calendar_url = cfg.calendar_url;
                    self.cfg.news_url = cfg.news_url;
                    self.cal_at = None;
                    self.news_at = None;
                    self.notice = Some(t("Gespeichert.").to_string());
                }
                let mut token_save = false;
                section(ui, t("Zugang zu GitHub"), &mut |ui| {
                    let state = if core::has_token() { t("Schlüssel ist gespeichert.") } else { t("Kein Schlüssel gespeichert.") };
                    ui.label(egui::RichText::new(state).color(soft()));
                    ui.label(egui::RichText::new(t("Nur nötig, wenn das Werkbank-Repository privat ist: persönlicher Zugriffsschlüssel (fine-grained token) mit Lesezugriff auf «Contents» dieses Repositorys.")).size(12.5).color(muted()));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.token_input).password(true).hint_text("github_pat_…").desired_width(360.0));
                        if button(ui, t("Speichern"), Look::Outline).clicked() {
                            token_save = true;
                        }
                    });
                });
                if token_save {
                    let tok = Some(self.token_input.trim().to_string()).filter(|s| !s.is_empty());
                    let mut cfg = core::load_config();
                    cfg.github_token = tok.clone();
                    let _ = core::save_config(&cfg);
                    core::set_token(tok);
                    self.token_input.clear();
                    self.refresh(ctx);
                    self.notice = Some(t("Gespeichert. Programme und Updates werden neu gesucht.").to_string());
                }
                let mut types = false;
                section(ui, t("Programme und Dateien"), &mut |ui| {
                    ui.label(egui::RichText::new(tf("Datenordner: {pfad}", &[("pfad", &core::data_dir().display())])).color(soft()));
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
                let mut check = false;
                let upd = self.self_update.lock().map(|s| s.clone()).unwrap_or_default();
                section(ui, t("Über"), &mut |ui| {
                    ui.label(egui::RichText::new(concat!("Werkbank ", env!("CARGO_PKG_VERSION"))).color(soft()));
                    ui.horizontal(|ui| {
                        if button(ui, t("Nach neuer Werkbank suchen"), Look::Outline).clicked() {
                            check = true;
                        }
                        let msg = match &upd {
                            SelfUpdate::Checking => Some((t("Suche …").to_string(), muted())),
                            SelfUpdate::Current => Some((t("Diese Werkbank ist aktuell.").to_string(), muted())),
                            SelfUpdate::Ready(v) => Some((tf("Werkbank {v} gilt nach einem Neustart.", &[("v", &fmt_version(*v))]), accent_text())),
                            SelfUpdate::Failed(e) => Some((tf("Selbst-Update fehlgeschlagen: {fehler}", &[("fehler", e)]), warn())),
                            _ => None,
                        };
                        if let Some((m, c)) = msg {
                            ui.label(egui::RichText::new(m).size(13.0).color(c));
                        }
                    });
                    ui.label(egui::RichText::new(t("Die Werkbank sucht beim Start selbst nach einer neuen Version und setzt sie ein.")).size(12.5).color(muted()));
                    ui.label(egui::RichText::new(t("Programme basieren auf den Open-Source-Apps des ArtCraft-Teams (MIT / Apache 2.0). Wetterdaten: Open-Meteo.com (CC BY 4.0).")).size(12.5).color(muted()));
                    if link(ui, t("Mehr …")).clicked() {
                        about = true;
                    }
                });
                self.about |= about;
                if check {
                    if let Ok(mut s) = self.self_update.lock() {
                        if matches!(*s, SelfUpdate::Current | SelfUpdate::Failed(_)) {
                            *s = SelfUpdate::Idle;
                        }
                    }
                    self.check_self_update(ctx, true);
                }
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
