//! Oberfläche (egui): Suche oben, Poster-Raster (Sammlung oder Suchergebnisse),
//! rechts das Detail mit Status, eigener Bewertung und Übernahme in die Sammlung.
//! Poster werden über egui_extras-Bildlader direkt per URL geladen. Suche läuft
//! in einem Hintergrund-Thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::{self, Entry, Filter, Kind, Status};
use crate::sprache::{t, tf};
use crate::store::{self, Config};
use crate::thema;
use crate::tmdb;

/// Bernstein für Flächen, Sterne, Logo und Auswahlrahmen – in beiden Designs gleich.
fn am() -> egui::Color32 {
    egui::Color32::from_rgb(0xe0, 0x98, 0x2a)
}
/// Dunkle Schrift auf `am()`-Flächen (weiss wäre auf Bernstein zu schwach lesbar).
fn on_am() -> egui::Color32 {
    egui::Color32::from_rgb(0x17, 0x18, 0x1c)
}
/// Bernstein als Schriftfarbe – je Design lesbar auf dem Hintergrund.
fn am_text() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0x8a, 0x5a, 0x0f), egui::Color32::from_rgb(0xfb, 0xbf, 0x24))
}
/// Hintergrund gewählter Chips.
fn am_soft() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xfb, 0xf0, 0xdb), egui::Color32::from_rgb(0x3a, 0x2e, 0x17))
}
fn red() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xb9, 0x1c, 0x1c), egui::Color32::from_rgb(0xf8, 0x71, 0x71))
}
/// Füllung der Status-Plakette auf dem Poster (weisse Schrift) – in beiden Designs gleich.
fn status_color(s: Status) -> egui::Color32 {
    match s {
        Status::Geplant => egui::Color32::from_rgb(0x3a, 0x6e, 0xa5),
        Status::AmSchauen => egui::Color32::from_rgb(0xb4, 0x53, 0x09),
        Status::Gesehen => egui::Color32::from_rgb(0x18, 0x86, 0x4a),
    }
}

#[derive(PartialEq)]
enum Mode {
    Collection,
    Search,
}

struct Job<T> {
    data: Arc<Mutex<Option<T>>>,
    running: Arc<AtomicBool>,
}
impl<T: Send + 'static> Job<T> {
    fn idle() -> Self {
        Job { data: Arc::new(Mutex::new(None)), running: Arc::new(AtomicBool::new(false)) }
    }
    fn start(&self, f: impl FnOnce() -> T + Send + 'static) {
        self.running.store(true, Ordering::SeqCst);
        let d = self.data.clone();
        let r = self.running.clone();
        std::thread::spawn(move || {
            let out = f();
            *d.lock().unwrap() = Some(out);
            r.store(false, Ordering::SeqCst);
        });
    }
    fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

pub struct App {
    cfg: Config,
    lib: Vec<Entry>,
    filter: Filter,
    search_text: String,
    mode: Mode,
    results: Job<Result<Vec<Entry>, String>>,
    sel: Option<Entry>,
    settings_open: bool,
    msg: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.03;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0x8C, 0x5A, 0x2B]);
        let cfg = store::load_config();
        let lib = store::load_library();
        let settings_open = cfg.api_key.trim().is_empty();
        App {
            cfg,
            lib,
            filter: Filter::Alle,
            search_text: String::new(),
            mode: Mode::Collection,
            results: Job::idle(),
            sel: None,
            settings_open,
            msg: String::new(),
        }
    }

    fn do_search(&mut self) {
        let q = self.search_text.trim().to_string();
        if q.is_empty() {
            self.mode = Mode::Collection;
            return;
        }
        self.mode = Mode::Search;
        self.sel = None;
        let key = self.cfg.api_key.clone();
        let lang = self.cfg.lang.clone();
        self.results.start(move || tmdb::search(&key, &q, &lang));
    }

    fn lib_index(&self, e: &Entry) -> Option<usize> {
        self.lib.iter().position(|x| x.id == e.id && x.kind == e.kind)
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.results.running() {
            ctx.request_repaint();
        }
        self.settings_window(ctx);
        self.top_bar(ctx);
        self.status_bar(ctx);
        self.detail_panel(ctx);
        self.central(ctx);
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 7.0, am());
                paint_clap(ui.painter(), r);
                ui.add_space(4.0);
                ui.heading("Mediathekwerk");
                ui.add(egui::Label::new(egui::RichText::new(t("Filme & Serien · Werkbank")).weak()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t("Einstellungen")).clicked() {
                        self.settings_open = true;
                    }
                    if ui.button(t("Suchen")).clicked() {
                        self.do_search();
                    }
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.search_text)
                            .hint_text(t("Film oder Serie suchen …"))
                            .desired_width(300.0),
                    );
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.do_search();
                    }
                });
            });
            ui.add_space(8.0);
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let connected = !self.cfg.api_key.trim().is_empty();
                let (dot, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                let col = if connected {
                    egui::Color32::from_rgb(0x1f, 0x9d, 0x55)
                } else {
                    egui::Color32::from_rgb(0xe5, 0x34, 0x2e)
                };
                ui.painter().circle_filled(dot.center(), 4.0, col);
                ui.weak(if connected { t("TMDB verbunden") } else { t("Kein TMDB-Schlüssel") });
                if !self.msg.is_empty() {
                    ui.separator();
                    ui.weak(self.msg.as_str());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(tf("{n} Titel in der Sammlung · Daten von TMDB", &[("n", &self.lib.len())]));
                });
            });
            ui.add_space(4.0);
        });
    }

    fn central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            // Kopfzeile je Modus
            match self.mode {
                Mode::Collection => {
                    ui.horizontal_wrapped(|ui| {
                        for f in Filter::all() {
                            if chip(ui, self.filter == f, t(f.label())).clicked() {
                                self.filter = f;
                            }
                        }
                    });
                }
                Mode::Search => {
                    ui.horizontal(|ui| {
                        if ui.button(t("< Zur Sammlung")).clicked() {
                            self.mode = Mode::Collection;
                            self.sel = None;
                        }
                        ui.label(egui::RichText::new(tf("Suchergebnisse: {q}", &[("q", &self.search_text.trim())])).strong());
                        if self.results.running() {
                            ui.spinner();
                        }
                    });
                }
            }
            ui.add_space(12.0);

            // Anzuzeigende Titel
            let (display, err): (Vec<Entry>, Option<String>) = match self.mode {
                Mode::Collection => {
                    let idx = model::filter_sort(&self.lib, self.filter, "");
                    (idx.iter().map(|&i| self.lib[i].clone()).collect(), None)
                }
                Mode::Search => {
                    let g = self.results.data.lock().unwrap();
                    match g.as_ref() {
                        Some(Ok(v)) => (v.clone(), None),
                        Some(Err(e)) => (Vec::new(), Some(e.clone())),
                        None => (Vec::new(), None),
                    }
                }
            };

            if let Some(e) = err {
                ui.colored_label(red(), e);
                return;
            }
            if display.is_empty() {
                ui.add_space(20.0);
                ui.weak(match self.mode {
                    Mode::Collection => t("Noch nichts in der Sammlung. Oben nach einem Titel suchen und aufnehmen."),
                    Mode::Search => if self.results.running() { t("Wird gesucht …") } else { t("Keine Treffer.") },
                });
                return;
            }

            let show_status = self.mode == Mode::Collection;
            let pw = 150.0;
            let ph = 225.0;
            let gap = 14.0;
            let cols = (((ui.available_width() + gap) / (pw + gap)).floor() as usize).max(1);
            let mut newsel: Option<Entry> = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for row in display.chunks(cols) {
                    ui.horizontal(|ui| {
                        for e in row {
                            let selected = self.sel.as_ref().map(|s| s.id == e.id && s.kind == e.kind).unwrap_or(false);
                            if poster_card(ui, e, pw, ph, selected, show_status).clicked() {
                                newsel = Some(e.clone());
                            }
                        }
                    });
                    ui.add_space(12.0);
                }
            });
            if let Some(e) = newsel {
                self.sel = Some(e);
            }
        });
    }

    fn detail_panel(&mut self, ctx: &egui::Context) {
        let sel = match &self.sel {
            Some(e) => e.clone(),
            None => return,
        };
        egui::SidePanel::right("detail")
            .resizable(false)
            .exact_width(356.0)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                let in_lib = self.lib_index(&sel);

                // Poster (zentriert)
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(170.0, 255.0), egui::Sense::hover());
                    draw_poster(ui, rect, &sel, false);
                });

                ui.add_space(12.0);
                ui.add(egui::Label::new(egui::RichText::new(&sel.title).size(18.0).strong()).truncate());
                ui.horizontal(|ui| {
                    let mut parts = vec![t(sel.kind.label()).to_string()];
                    if let Some(y) = sel.year {
                        parts.insert(0, y.to_string());
                    }
                    if let Some(r) = sel.runtime {
                        parts.push(tf("{r} Min", &[("r", &r)]));
                    }
                    if let Some(s) = sel.seasons {
                        parts.push(tf("{s} Staffeln", &[("s", &s)]));
                    }
                    ui.weak(parts.join(" · "));
                    if sel.rating > 0.0 {
                        let (sr, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        paint_star(ui.painter(), sr.center(), 6.0, true, am());
                        ui.label(egui::RichText::new(format!("{:.1}", sel.rating)).color(am_text()).strong());
                    }
                });

                ui.add_space(10.0);
                egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                    ui.label(egui::RichText::new(&sel.overview).size(12.5).color(thema::text()));
                });

                ui.add_space(14.0);
                ui.label(egui::RichText::new(t("STATUS")).size(11.0).strong().color(thema::muted()));
                ui.add_space(6.0);
                let mut set_status: Option<Status> = None;
                ui.horizontal(|ui| {
                    for s in Status::all() {
                        if chip(ui, sel.status == s, t(s.label())).clicked() {
                            set_status = Some(s);
                        }
                    }
                });

                ui.add_space(14.0);
                ui.label(egui::RichText::new(t("MEINE BEWERTUNG")).size(11.0).strong().color(thema::muted()));
                ui.add_space(6.0);
                let set_rating = star_row(ui, sel.my_rating);

                ui.add_space(16.0);
                let mut do_add = false;
                let mut do_remove = false;
                if in_lib.is_some() {
                    if ui.add(egui::Button::new(egui::RichText::new(t("Aus Sammlung entfernen")).color(egui::Color32::WHITE)).fill(egui::Color32::from_rgb(0xb9, 0x1c, 0x1c)).min_size(egui::vec2(ui.available_width(), 42.0))).clicked() {
                        do_remove = true;
                    }
                } else if ui.add(egui::Button::new(egui::RichText::new(t("In Sammlung aufnehmen")).color(on_am())).fill(am()).min_size(egui::vec2(ui.available_width(), 42.0))).clicked() {
                    do_add = true;
                }

                // Aktionen anwenden
                if let Some(s) = set_status {
                    if let Some(i) = in_lib {
                        self.lib[i].status = s;
                        store::save_library(&self.lib);
                    }
                    if let Some(e) = self.sel.as_mut() {
                        e.status = s;
                    }
                }
                if let Some(r) = set_rating {
                    if let Some(i) = in_lib {
                        self.lib[i].my_rating = r;
                        store::save_library(&self.lib);
                    }
                    if let Some(e) = self.sel.as_mut() {
                        e.my_rating = r;
                    }
                }
                if do_add {
                    self.lib.push(sel.clone());
                    store::save_library(&self.lib);
                    self.msg = t("In Sammlung aufgenommen.").into();
                }
                if do_remove {
                    if let Some(i) = in_lib {
                        self.lib.remove(i);
                        store::save_library(&self.lib);
                    }
                    self.sel = None;
                    self.msg = t("Aus Sammlung entfernt.").into();
                }
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new(t("Einstellungen"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(t("TMDB-API-Schlüssel (kostenlos bei themoviedb.org):"));
                ui.add(egui::TextEdit::singleline(&mut self.cfg.api_key).desired_width(360.0).password(false).hint_text(t("API-Schlüssel (v3 auth)")));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(t("Sprache:"));
                    egui::ComboBox::from_id_salt("lang")
                        .selected_text(self.cfg.lang.clone())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.cfg.lang, "de-DE".into(), "Deutsch");
                            ui.selectable_value(&mut self.cfg.lang, "en-US".into(), "English");
                        });
                });
                ui.add_space(12.0);
                if ui.add(egui::Button::new(egui::RichText::new(t("Speichern")).color(on_am())).fill(am())).clicked() {
                    store::save_config(&self.cfg);
                    self.msg = t("Einstellungen gespeichert.").into();
                }
                ui.add_space(4.0);
                ui.weak(t("Der Schlüssel wird nur lokal gespeichert."));
            });
        self.settings_open = open;
    }
}

// ---------- Bausteine ----------

fn chip(ui: &mut egui::Ui, sel: bool, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .size(12.5)
                .color(if sel { am_text() } else { thema::text() }),
        )
        .fill(if sel { am_soft() } else { thema::panel() })
        .stroke(egui::Stroke::new(1.0, if sel { am() } else { thema::border() })),
    )
}

fn poster_card(ui: &mut egui::Ui, e: &Entry, pw: f32, ph: f32, selected: bool, show_status: bool) -> egui::Response {
    ui.allocate_ui(egui::vec2(pw, ph + 46.0), |ui| {
        ui.set_width(pw);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(pw, ph), egui::Sense::click());
        draw_poster(ui, rect, e, show_status);
        if selected {
            ui.painter().rect_stroke(rect.expand(2.0), 10.0, egui::Stroke::new(2.5, am()));
        }
        ui.add(egui::Label::new(egui::RichText::new(&e.title).size(12.5).strong()).truncate());
        let mut sub = Vec::new();
        if let Some(y) = e.year {
            sub.push(y.to_string());
        }
        sub.push(t(e.kind.label()).to_string());
        ui.weak(sub.join(" · "));
        resp
    })
    .inner
}

/// Poster-Fläche: Bild per URL, sonst Platzhalter mit Titel. Dazu Bewertung und
/// (in der Sammlung) ein Status-Badge als Overlay.
fn draw_poster(ui: &mut egui::Ui, rect: egui::Rect, e: &Entry, show_status: bool) {
    // Platzhalter-Fläche: dunkel wie ein Poster; im dunklen Design etwas heller, damit sie sich abhebt.
    ui.painter().rect_filled(rect, 8.0, thema::pick(egui::Color32::from_rgb(0x2a, 0x2e, 0x38), egui::Color32::from_rgb(0x33, 0x37, 0x42)));
    if let Some(path) = &e.poster {
        let url = model::image_url("w342", path);
        egui::Image::new(url.as_str()).rounding(8.0).paint_at(ui, rect);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            &e.title,
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(210),
        );
    }
    if e.rating > 0.0 {
        let pill = egui::Rect::from_min_size(egui::pos2(rect.right() - 54.0, rect.top() + 6.0), egui::vec2(48.0, 20.0));
        ui.painter().rect_filled(pill, 6.0, egui::Color32::from_black_alpha(180));
        paint_star(ui.painter(), pill.left_center() + egui::vec2(10.0, 0.0), 6.0, true, am());
        ui.painter().text(pill.left_center() + egui::vec2(18.0, 0.0), egui::Align2::LEFT_CENTER, format!("{:.1}", e.rating), egui::FontId::proportional(11.5), egui::Color32::WHITE);
    }
    if show_status {
        let badge = egui::Rect::from_min_size(egui::pos2(rect.left() + 6.0, rect.top() + 6.0), egui::vec2(72.0, 19.0));
        ui.painter().rect_filled(badge, 6.0, status_color(e.status));
        ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, t(e.status.label()), egui::FontId::proportional(10.5), egui::Color32::WHITE);
    }
}

fn star_row(ui: &mut egui::Ui, rating: u8) -> Option<u8> {
    let mut set = None;
    ui.horizontal(|ui| {
        for i in 0..5u8 {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
            let filled = i < rating;
            paint_star(ui.painter(), r.center(), 10.0, filled, if filled { am() } else { thema::pick(egui::Color32::from_gray(205), egui::Color32::from_rgb(0x6b, 0x70, 0x7b)) });
            if resp.clicked() {
                set = Some(if rating == i + 1 { i } else { i + 1 });
            }
        }
    });
    set
}

fn paint_star(p: &egui::Painter, c: egui::Pos2, r: f32, filled: bool, col: egui::Color32) {
    use std::f32::consts::PI;
    let mut pts = Vec::with_capacity(10);
    for k in 0..10 {
        let ang = -PI / 2.0 + k as f32 * PI / 5.0;
        let rad = if k % 2 == 0 { r } else { r * 0.42 };
        pts.push(egui::pos2(c.x + rad * ang.cos(), c.y + rad * ang.sin()));
    }
    if filled {
        // Als Dreiecksfächer vom Mittelpunkt füllen (ein Stern ist nicht konvex).
        for k in 0..10 {
            let a = pts[k];
            let b = pts[(k + 1) % 10];
            p.add(egui::Shape::convex_polygon(vec![c, a, b], col, egui::Stroke::NONE));
        }
    } else {
        p.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.4, col)));
    }
}

fn paint_clap(p: &egui::Painter, r: egui::Rect) {
    let st = egui::Stroke::new(1.8, egui::Color32::WHITE);
    let body = egui::Rect::from_center_size(r.center() + egui::vec2(0.0, 2.5), egui::vec2(15.0, 10.0));
    p.rect_stroke(body, 1.0, st);
    // Klapp-Leiste oben mit schrägen Strichen
    let top = body.top() - 1.0;
    for k in 0..3 {
        let x = body.left() + 3.0 + k as f32 * 5.0;
        p.line_segment([egui::pos2(x, top), egui::pos2(x - 2.0, top - 4.0)], st);
    }
}
