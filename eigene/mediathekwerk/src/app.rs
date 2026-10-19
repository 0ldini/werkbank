//! Oberfläche (egui): Suche oben, Poster-Raster (Sammlung oder Suchergebnisse),
//! rechts das Detail mit Status, eigener Bewertung und Übernahme in die Sammlung.
//! Poster werden über egui_extras-Bildlader direkt per URL geladen. Suche läuft
//! in einem Hintergrund-Thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::{self, Entry, Filter, Kind, Status};
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::store::{self, Config};
use crate::thema;
use crate::tmdb;

/// Bernstein für Flächen, Sterne, Logo und Auswahlrahmen – in beiden Designs gleich.
fn am() -> egui::Color32 {
    egui::Color32::from_rgb(0xe0, 0x98, 0x2a)
}
/// Bernstein als Schriftfarbe – je Design lesbar auf dem Hintergrund.
fn am_text() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0x8a, 0x5a, 0x0f), egui::Color32::from_rgb(0xfb, 0xbf, 0x24))
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
        stil::init(&cc.egui_ctx, [0x8C, 0x5A, 0x2B]);
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
        stil::kopfleiste(ctx, |ui| {
            stil::titel(ui, "Mediathekwerk");
            stil::grau(ui, t("Filme & Serien · Werkbank"));
            stil::trenner(ui);
            let resp = ui.add_sized(
                [300.0, 28.0],
                egui::TextEdit::singleline(&mut self.search_text).hint_text(t("Film oder Serie suchen …")),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.do_search();
            }
            if stil::knopf(ui, Sym::Suche, t("Suchen")).clicked() {
                self.do_search();
            }
            stil::rechts(ui, |ui| {
                if stil::leise(ui, true, Sym::Zahnrad, t("Einstellungen"), t("TMDB-Schlüssel und Sprache")).clicked() {
                    self.settings_open = true;
                }
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let connected = !self.cfg.api_key.trim().is_empty();
        let in_lib = self.sel.as_ref().and_then(|e| self.lib_index(e));
        let has_sel = self.sel.is_some();
        let mut do_add = false;
        let mut do_remove = false;
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, if connected { stil::gruen() } else { stil::rot() });
                stil::grau(ui, if connected { t("TMDB verbunden") } else { t("Kein TMDB-Schlüssel") });
                if !self.msg.is_empty() {
                    stil::trenner(ui);
                    stil::grau(ui, self.msg.as_str());
                }
                stil::trenner(ui);
                stil::grau(ui, &tf("{n} Titel in der Sammlung · Daten von TMDB", &[("n", &self.lib.len())]));
                stil::rechts(ui, |ui| {
                    if in_lib.is_some() {
                        if stil::gefahr(ui, true, Sym::Papierkorb, t("Aus Sammlung entfernen")).clicked() {
                            do_remove = true;
                        }
                    } else if stil::haupt(ui, has_sel, Sym::Plus, t("In Sammlung aufnehmen")).clicked() {
                        do_add = true;
                    }
                });
            });
        });
        if do_add {
            if let Some(sel) = self.sel.clone() {
                if self.lib_index(&sel).is_none() {
                    self.lib.push(sel);
                    store::save_library(&self.lib);
                    self.msg = t("In Sammlung aufgenommen.").into();
                }
            }
        }
        if do_remove {
            if let Some(i) = in_lib {
                self.lib.remove(i);
                store::save_library(&self.lib);
            }
            self.sel = None;
            self.msg = t("Aus Sammlung entfernt.").into();
        }
    }

    fn central(&mut self, ctx: &egui::Context) {
        stil::mitte(ctx, |ui| {
            ui.add_space(4.0);
            // Kopfzeile je Modus
            match self.mode {
                Mode::Collection => {
                    let opts: Vec<(Filter, &str)> = Filter::all().iter().map(|f| (*f, t(f.label()))).collect();
                    let w = ui.available_width().min(640.0);
                    ui.allocate_ui(egui::vec2(w, 28.0), |ui| {
                        stil::wahl(ui, &mut self.filter, &opts);
                    });
                }
                Mode::Search => {
                    ui.horizontal(|ui| {
                        if stil::knopf(ui, Sym::Rueckgaengig, t("< Zur Sammlung")).clicked() {
                            self.mode = Mode::Collection;
                            self.sel = None;
                        }
                        ui.label(egui::RichText::new(tf("Suchergebnisse: {q}", &[("q", &self.search_text.trim())])).strong().color(thema::text()));
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
                    (idx.iter().filter_map(|&i| self.lib.get(i).cloned()).collect(), None)
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
                stil::leer(ui, Sym::Warnung, t("Suche fehlgeschlagen"), &e);
                return;
            }
            if display.is_empty() {
                match self.mode {
                    Mode::Collection => stil::leer(ui, Sym::Film, t("Sammlung leer"), t("Noch nichts in der Sammlung. Oben nach einem Titel suchen und aufnehmen.")),
                    Mode::Search => {
                        if self.results.running() {
                            stil::leer(ui, Sym::Suche, t("Wird gesucht …"), "");
                        } else {
                            stil::leer(ui, Sym::Suche, t("Keine Treffer."), t("Anderen Suchbegriff versuchen."));
                        }
                    }
                }
                return;
            }

            let show_status = self.mode == Mode::Collection;
            let pw = 150.0;
            let ph = 225.0;
            let gap = 14.0;
            let cols = (((ui.available_width() + gap) / (pw + gap)).floor() as usize).max(1);
            let mut newsel: Option<Entry> = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
        let in_lib = self.lib_index(&sel);
        let mut set_status: Option<Status> = None;
        let mut set_rating: Option<u8> = None;
        let mut close = false;
        egui::SidePanel::right("detail")
            .resizable(false)
            .exact_width(356.0)
            .frame(egui::Frame::none().fill(thema::bg()).inner_margin(egui::Margin::same(12.0)).stroke(egui::Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    stil::rechts(ui, |ui| {
                        if stil::leise(ui, true, Sym::Kreuz, "", t("Detail schliessen")).clicked() {
                            close = true;
                        }
                    });
                });

                // Poster (zentriert)
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(170.0, 255.0), egui::Sense::hover());
                    draw_poster(ui, rect, &sel, false);
                });

                ui.add_space(12.0);
                ui.add(egui::Label::new(egui::RichText::new(&sel.title).size(18.0).strong().color(thema::text())).truncate());
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
                    stil::grau(ui, &parts.join(" · "));
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
                stil::feld(ui, t("STATUS"), |ui| {
                    let mut st = sel.status;
                    let opts: Vec<(Status, &str)> = Status::all().iter().map(|s| (*s, t(s.label()))).collect();
                    if stil::wahl(ui, &mut st, &opts) {
                        set_status = Some(st);
                    }
                });

                ui.add_space(14.0);
                stil::feld(ui, t("MEINE BEWERTUNG"), |ui| {
                    set_rating = star_row(ui, sel.my_rating);
                });
                if in_lib.is_none() {
                    ui.add_space(10.0);
                    stil::hinweis(ui, t("Status und Bewertung werden gespeichert, sobald der Titel in der Sammlung ist."));
                }
            });

        // Aktionen anwenden
        if let Some(s) = set_status {
            if let Some(i) = in_lib {
                if let Some(e) = self.lib.get_mut(i) {
                    e.status = s;
                }
                store::save_library(&self.lib);
            }
            if let Some(e) = self.sel.as_mut() {
                e.status = s;
            }
        }
        if let Some(r) = set_rating {
            if let Some(i) = in_lib {
                if let Some(e) = self.lib.get_mut(i) {
                    e.my_rating = r;
                }
                store::save_library(&self.lib);
            }
            if let Some(e) = self.sel.as_mut() {
                e.my_rating = r;
            }
        }
        if close {
            self.sel = None;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new(t("Einstellungen"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                stil::feld(ui, t("TMDB-API-Schlüssel (kostenlos bei themoviedb.org):"), |ui| {
                    ui.add_sized(
                        [ui.available_width(), 28.0],
                        egui::TextEdit::singleline(&mut self.cfg.api_key).password(false).hint_text(t("API-Schlüssel (v3 auth)")),
                    );
                });
                ui.add_space(6.0);
                stil::feld(ui, t("Sprache:"), |ui| {
                    let cur = self.cfg.lang.clone();
                    let mut v: &str = cur.as_str();
                    if stil::liste(ui, "lang", &mut v, &[("de-DE", "Deutsch"), ("en-US", "English")]) {
                        self.cfg.lang = v.to_string();
                    }
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if stil::knopf(ui, Sym::Speichern, t("Speichern")).clicked() {
                        store::save_config(&self.cfg);
                        self.msg = t("Einstellungen gespeichert.").into();
                    }
                });
                ui.add_space(4.0);
                stil::hinweis(ui, t("Der Schlüssel wird nur lokal gespeichert."));
            });
        self.settings_open = open;
    }
}

// ---------- Bausteine ----------

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
