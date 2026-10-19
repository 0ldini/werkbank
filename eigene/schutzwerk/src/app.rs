//! Oberfläche (egui): linke Navigation mit vier defensiven Bereichen
//! (Passwort-Check, Netzwerk, System-Schutz, Papierkorb) und einer Übersicht.
//! Netz-/System-/Papierkorb-Abfragen laufen in Hintergrund-Threads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::netparse::{self, Device};
use crate::sprache::{t, tf};
use crate::pwcheck::{self, PwResult};
use crate::sys::{self, Check, PortInfo, RecItem, State};
use crate::stil::{self, Sym};
use crate::thema;

// Schrift-, Linien- und Statusfarben je Design (hell/dunkel), lesbar auf dem Hintergrund.
fn grn() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0x15, 0x7a, 0x3a), egui::Color32::from_rgb(0x4a, 0xde, 0x80))
}
/// Hintergrund für Gewähltes und Hinweise.
fn grn_soft() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xe7, 0xf6, 0xee), egui::Color32::from_rgb(0x1b, 0x3a, 0x2a))
}
fn amber() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xb4, 0x53, 0x09), egui::Color32::from_rgb(0xfb, 0xbf, 0x24))
}
fn orange() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xc2, 0x41, 0x0c), egui::Color32::from_rgb(0xfb, 0x92, 0x3c))
}
fn red() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xb9, 0x1c, 0x1c), egui::Color32::from_rgb(0xf8, 0x71, 0x71))
}

const COMMON: &[&str] = &[
    "123456", "123456789", "12345678", "password", "passwort", "qwertz", "qwerty", "111111",
    "123123", "000000", "iloveyou", "admin", "root", "letmein", "welcome", "monkey", "dragon",
    "sunshine", "princess", "football", "hallo", "hallo123", "schatz", "sommer", "geheim",
    "passwort1", "12341234", "1q2w3e4r", "abc123", "test123", "zaq12wsx", "superman",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Password,
    Network,
    System,
    Recycle,
    Overview,
}

struct Job<T> {
    data: Arc<Mutex<Option<T>>>,
    running: Arc<AtomicBool>,
}

impl<T: Send + 'static> Job<T> {
    fn idle() -> Self {
        Job {
            data: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
    fn start(&self, f: impl FnOnce() -> T + Send + 'static) {
        if self.running.load(Ordering::SeqCst) {
            return;
        }
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
    section: Section,
    pw: String,
    show_pw: bool,
    ports: Job<Vec<PortInfo>>,
    lan: Job<Vec<Device>>,
    checks: Job<Vec<Check>>,
    recycle: Job<Vec<RecItem>>,
    restore_msg: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0x3D, 0x6B, 0x8C]);
        stil::init(&cc.egui_ctx, [0x3D, 0x6B, 0x8C]);
        App {
            section: Section::Password,
            pw: String::new(),
            show_pw: false,
            ports: Job::idle(),
            lan: Job::idle(),
            checks: Job::idle(),
            recycle: Job::idle(),
            restore_msg: String::new(),
        }
    }

    fn score(&self) -> Option<u32> {
        let g = self.checks.data.lock().unwrap();
        g.as_ref().map(|c| sys::score(c))
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.ports.running() || self.lan.running() || self.checks.running() || self.recycle.running() {
            ctx.request_repaint();
        }
        self.top_bar(ctx);
        self.footer(ctx);
        self.nav(ctx);
        stil::mitte(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(6.0);
                match self.section {
                    Section::Password => self.page_password(ui),
                    Section::Network => self.page_network(ui),
                    Section::System => self.page_system(ui),
                    Section::Recycle => self.page_recycle(ui),
                    Section::Overview => self.page_overview(ui),
                }
            });
        });
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        let score = self.score();
        stil::kopfleiste(ctx, |ui| {
            stil::titel(ui, "Schutzwerk");
            stil::grau(ui, t("Sicherheit · Werkbank"));
            stil::rechts(ui, |ui| match score {
                Some(s) => {
                    let col = if s >= 75 { grn() } else if s >= 50 { amber() } else { red() };
                    ui.label(egui::RichText::new(tf("Schutz: {s}/100", &[("s", &s)])).color(col).strong());
                    stil::symbol(ui, Sym::Schild, 18.0, col);
                }
                None => {
                    stil::grau(ui, t("Schutz: noch nicht geprüft"));
                    stil::symbol(ui, Sym::Schild, 18.0, thema::muted());
                }
            });
        });
    }

    /// Fussleiste: links der Stand, rechts der Hauptknopf des gewählten Bereichs.
    fn footer(&mut self, ctx: &egui::Context) {
        let busy = match self.section {
            Section::Network => self.ports.running() || self.lan.running(),
            Section::System | Section::Overview => self.checks.running(),
            Section::Recycle => self.recycle.running(),
            Section::Password => false,
        };
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                if busy {
                    ui.spinner();
                    stil::grau(ui, if self.section == Section::Recycle { t("wird gelesen …") } else { t("wird geprüft …") });
                } else if self.section == Section::Recycle && !self.restore_msg.is_empty() {
                    ui.colored_label(grn(), self.restore_msg.as_str());
                } else {
                    stil::grau(ui, t("Alles läuft lokal auf deinem Rechner. Rein defensiv – keine Angriffswerkzeuge."));
                }
                stil::rechts(ui, |ui| match self.section {
                    Section::Network => {
                        if stil::haupt(ui, !busy, Sym::Suche, t("Scannen")).clicked() {
                            self.ports.start(sys::open_ports);
                            self.lan.start(sys::lan_devices);
                        }
                    }
                    Section::System | Section::Overview => {
                        if stil::haupt(ui, !busy, Sym::Schild, t("Prüfen")).clicked() {
                            self.checks.start(sys::protection_status);
                        }
                    }
                    Section::Recycle => {
                        if stil::haupt(ui, !busy, Sym::Papierkorb, t("Papierkorb lesen")).clicked() {
                            self.restore_msg.clear();
                            self.recycle.start(sys::list_recycle);
                        }
                    }
                    Section::Password => {}
                });
            });
        });
    }

    fn nav(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(236.0)
            .frame(egui::Frame::none().fill(thema::bg()).inner_margin(egui::Margin::same(10.0)).stroke(egui::Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| {
                ui.add_space(4.0);
                nav_item(ui, &mut self.section, Section::Password, Sym::Schluessel, t("Passwort-Check"), None);
                nav_item(ui, &mut self.section, Section::Network, Sym::Link, t("Netzwerk"), None);
                nav_item(ui, &mut self.section, Section::System, Sym::Schild, t("System-Schutz"), None);
                nav_item(ui, &mut self.section, Section::Recycle, Sym::Papierkorb, t("Papierkorb"), None);
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                nav_item(ui, &mut self.section, Section::Overview, Sym::Diagramm, t("Übersicht"), None);
            });
    }

    // ---------- Passwort ----------
    fn page_password(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Passwort-Check"), t("Prüft die Stärke – vollständig offline, nichts wird gesendet oder gespeichert."));
        let res = pwcheck::analyze(&self.pw, COMMON);

        card(ui, |ui| {
            ui.horizontal(|ui| {
                let te = egui::TextEdit::singleline(&mut self.pw)
                    .password(!self.show_pw)
                    .hint_text(t("Passwort eingeben …"))
                    .desired_width(ui.available_width() - 120.0)
                    .font(egui::TextStyle::Monospace);
                ui.add(te);
                let lbl = if self.show_pw { t("verbergen") } else { t("zeigen") };
                if stil::knopf(ui, Sym::Auge, lbl).clicked() {
                    self.show_pw = !self.show_pw;
                }
            });

            ui.add_space(12.0);
            strength_meter(ui, res.level);
            ui.horizontal(|ui| {
                let col = level_color(res.level);
                ui.label(egui::RichText::new(t(pwcheck::level_label(res.level))).color(col).strong().size(15.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(crack_text(&res.crack)).strong());
                    ui.weak(t("Geschätzte Knackzeit: "));
                });
            });
        });

        card(ui, |ui| {
            egui::Grid::new("crit").num_columns(2).spacing([28.0, 10.0]).show(ui, |ui| {
                crit(ui, res.len >= 12, &tf("Mindestens 12 Zeichen ({n})", &[("n", &res.len)]));
                crit(ui, res.has_lower && res.has_upper, t("Gross- und Kleinbuchstaben"));
                ui.end_row();
                crit(ui, res.has_digit, t("Zahlen enthalten"));
                crit(ui, res.has_symbol, t("Sonderzeichen enthalten"));
                ui.end_row();
                crit(ui, !res.common && !self.pw.is_empty(), t("Kein bekanntes Passwort"));
                crit(ui, !(res.repeat || res.sequence || res.keyboard) && !self.pw.is_empty(), t("Keine Reihen/Wiederholungen"));
                ui.end_row();
            });
        });

        card(ui, |ui| {
            ui.horizontal(|ui| {
                stat(ui, &format!("{} Bit", res.entropy_bits.round() as u32), t("Entropie"));
                ui.add_space(26.0);
                stat(ui, &format!("{}", res.len), t("Zeichen"));
                ui.add_space(26.0);
                stat(ui, &format!("{} / 4", res.classes), t("Zeichenklassen"));
                ui.add_space(26.0);
                stat_colored(ui, t(pwcheck::level_label(res.level)), t("Bewertung"), level_color(res.level));
            });
        });

        note(ui, t("Tipp: Eine Passphrase aus vier zufälligen Wörtern ist leichter zu merken und oft stärker als ein kurzes kompliziertes Passwort."));
    }

    // ---------- Netzwerk ----------
    fn page_network(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Netzwerk"), t("Eigene offene Ports und Geräte im Netz – nur Lesen, keine Fremdscans."));

        card(ui, |ui| {
            ui.label(egui::RichText::new(t("OFFENE PORTS (DIESER PC)")).size(11.0).strong().color(thema::muted()));
            ui.add_space(8.0);
            let g = self.ports.data.lock().unwrap();
            match g.as_ref() {
                None => { ui.weak(t("Noch nicht geprüft.")); }
                Some(list) if list.is_empty() => { ui.weak(t("Keine offenen Ports gefunden.")); }
                Some(list) => {
                    for p in list {
                        ui.horizontal(|ui| {
                            ui.monospace(format!("{:>5} {}", p.listen.port, p.listen.proto));
                            ui.add_space(6.0);
                            match netparse::port_risk(p.listen.port) {
                                Some((name, hint, critical)) => {
                                    let c = if critical { red() } else { amber() };
                                    ui.label(egui::RichText::new(name).color(c).strong());
                                    ui.add(egui::Label::new(egui::RichText::new(t(hint)).weak()).truncate());
                                }
                                None => {
                                    ui.weak(p.process.as_str());
                                }
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.weak(p.process.as_str());
                            });
                        });
                    }
                }
            }
        });

        card(ui, |ui| {
            ui.label(egui::RichText::new(t("GERÄTE IM NETZ")).size(11.0).strong().color(thema::muted()));
            ui.add_space(8.0);
            let g = self.lan.data.lock().unwrap();
            match g.as_ref() {
                None => { ui.weak(t("Noch nicht geprüft.")); }
                Some(list) if list.is_empty() => { ui.weak(t("Keine Geräte gefunden.")); }
                Some(list) => {
                    for d in list {
                        ui.horizontal(|ui| {
                            ui.monospace(format!("{:<16}", d.ip));
                            ui.weak(d.mac.as_str());
                        });
                    }
                }
            }
        });
    }

    // ---------- System-Schutz ----------
    fn page_system(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("System-Schutz"), t("Status von Firewall, Virenschutz, Updates und SmartScreen."));
        let g = self.checks.data.lock().unwrap();
        match g.as_ref() {
            None => { stil::leer(ui, Sym::Schild, t("System-Schutz"), t("Noch nicht geprüft – auf 'Prüfen' klicken.")); }
            Some(checks) => {
                for c in checks {
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            state_dot(ui, c.state);
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(c.name.as_str()).strong().size(14.0));
                                ui.weak(c.detail.as_str());
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(state_label(c.state)).color(state_color(c.state)).strong());
                            });
                        });
                    });
                }
            }
        }
    }

    // ---------- Papierkorb ----------
    fn page_recycle(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Papierkorb"), t("Gelöschte Dateien auflisten und an ihren Ursprungsort zurückholen."));

        let mut do_restore: Option<usize> = None;
        {
            let g = self.recycle.data.lock().unwrap();
            match g.as_ref() {
                None => { ui.weak(t("Noch nicht gelesen.")); }
                Some(list) if list.is_empty() => { ui.weak(t("Der Papierkorb ist leer.")); }
                Some(list) => {
                    for (i, item) in list.iter().enumerate() {
                        card(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.add(egui::Label::new(egui::RichText::new(file_name(&item.rec.orig_path)).strong().size(13.5)).truncate());
                                    ui.weak(tf(
                                        "{pfad} · {groesse} · gelöscht {datum}",
                                        &[
                                            ("pfad", &short_path(&item.rec.orig_path)),
                                            ("groesse", &human_size(item.rec.size)),
                                            ("datum", &crate::recycle::fmt_date(item.rec.filetime)),
                                        ],
                                    ));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if stil::knopf(ui, Sym::Rueckgaengig, t("Wiederherstellen")).clicked() {
                                        do_restore = Some(i);
                                    }
                                });
                            });
                        });
                    }
                }
            }
        }
        if let Some(i) = do_restore {
            let item = {
                let g = self.recycle.data.lock().unwrap();
                g.as_ref().and_then(|l| l.get(i).cloned())
            };
            if let Some(item) = item {
                match sys::restore(&item) {
                    Ok(_) => {
                        self.restore_msg = t("Wiederhergestellt.").into();
                        self.recycle.start(sys::list_recycle);
                    }
                    Err(e) => self.restore_msg = e,
                }
            }
        }
    }

    // ---------- Übersicht ----------
    fn page_overview(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Übersicht"), t("Kurzer Gesamtstatus. Für Details die Bereiche links öffnen."));
        card(ui, |ui| {
            match self.score() {
                Some(s) => {
                    let col = if s >= 75 { grn() } else if s >= 50 { amber() } else { red() };
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{s}")).size(40.0).strong().color(col));
                        ui.label(egui::RichText::new(t("/ 100 Schutzpunkte")).size(15.0).weak());
                    });
                }
                None => { ui.weak(t("Noch keine Prüfung. Öffne 'System-Schutz' und klicke auf 'Prüfen'.")); }
            }
        });
        card(ui, |ui| {
            let g = self.checks.data.lock().unwrap();
            if let Some(checks) = g.as_ref() {
                for c in checks {
                    ui.horizontal(|ui| {
                        state_dot(ui, c.state);
                        ui.label(c.name.as_str());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(state_label(c.state)).color(state_color(c.state)));
                        });
                    });
                }
            } else {
                ui.weak(t("System-Schutz noch nicht geprüft."));
            }
        });
    }
}

// ---------- Bausteine ----------

fn card<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) {
    egui::Frame::none()
        .fill(thema::panel())
        .rounding(14.0)
        .stroke(egui::Stroke::new(1.0, thema::border()))
        .inner_margin(egui::Margin::symmetric(18.0, 15.0))
        .show(ui, |ui| {
            body(ui);
        });
    ui.add_space(14.0);
}

fn h1(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(egui::RichText::new(title).size(19.0).strong());
    ui.add_space(2.0);
    ui.label(egui::RichText::new(sub).size(13.0).color(thema::muted()));
    ui.add_space(16.0);
}

fn note(ui: &mut egui::Ui, text: &str) {
    egui::Frame::none()
        .fill(stil::toenung())
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(14.0, 11.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(12.5).color(thema::text()));
        });
}

fn nav_item(ui: &mut egui::Ui, cur: &mut Section, val: Section, sym: Sym, label: &str, _dot: Option<State>) {
    let sel = *cur == val;
    let fill = if sel { stil::toenung() } else { egui::Color32::TRANSPARENT };
    let resp = egui::Frame::none()
        .fill(fill)
        .rounding(9.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                stil::symbol(ui, sym, 17.0, if sel { stil::akzent() } else { thema::muted() });
                ui.add_space(4.0);
                ui.label(egui::RichText::new(label).size(13.5).color(thema::text()).strong());
                ui.allocate_space(egui::vec2(ui.available_width(), 0.0));
            });
        })
        .response
        .interact(egui::Sense::click());
    if resp.clicked() {
        *cur = val;
    }
    ui.add_space(3.0);
}

fn strength_meter(ui: &mut egui::Ui, level: u8) {
    let col = level_color(level);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), egui::Sense::hover());
    let gap = 4.0;
    let seg = (w - gap * 4.0) / 5.0;
    for i in 0..5usize {
        let x = rect.left() + i as f32 * (seg + gap);
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(seg, 10.0));
        let c = if i <= level as usize { col } else { thema::pick(egui::Color32::from_gray(228), egui::Color32::from_rgb(0x3a, 0x3d, 0x46)) };
        ui.painter().rect_filled(r, 5.0, c);
    }
    ui.add_space(8.0);
}

fn level_color(level: u8) -> egui::Color32 {
    match level {
        0 => red(),
        1 => orange(),
        2 => amber(),
        _ => grn(),
    }
}

fn crit(ui: &mut egui::Ui, ok: bool, label: &str) {
    ui.horizontal(|ui| {
        let (c, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        if ok {
            ui.painter().circle_filled(c.center(), 9.0, grn_soft());
            let p = c.center();
            let st = egui::Stroke::new(2.2, grn());
            ui.painter().line_segment([egui::pos2(p.x - 3.5, p.y), egui::pos2(p.x - 1.0, p.y + 2.8)], st);
            ui.painter().line_segment([egui::pos2(p.x - 1.0, p.y + 2.8), egui::pos2(p.x + 3.8, p.y - 2.8)], st);
        } else {
            ui.painter().circle_filled(c.center(), 9.0, thema::hover());
            ui.painter().line_segment(
                [egui::pos2(c.center().x - 3.0, c.center().y), egui::pos2(c.center().x + 3.0, c.center().y)],
                egui::Stroke::new(2.0, thema::muted()),
            );
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new(label).size(13.0).color(thema::text()));
    });
}

fn stat(ui: &mut egui::Ui, value: &str, key: &str) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(value).size(22.0).strong());
        ui.label(egui::RichText::new(key).size(11.5).color(thema::muted()));
    });
}
fn stat_colored(ui: &mut egui::Ui, value: &str, key: &str, col: egui::Color32) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(value).size(22.0).strong().color(col));
        ui.label(egui::RichText::new(key).size(11.5).color(thema::muted()));
    });
}

fn state_dot(ui: &mut egui::Ui, s: State) {
    let (c, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    ui.painter().circle_filled(c.center(), 6.0, state_color(s));
    ui.add_space(4.0);
}
fn state_color(s: State) -> egui::Color32 {
    match s {
        State::Ok => grn(),
        State::Warn => amber(),
        State::Bad => red(),
        State::Unknown => thema::muted(),
    }
}
fn state_label(s: State) -> &'static str {
    match s {
        State::Ok => t("In Ordnung"),
        State::Warn => t("Achtung"),
        State::Bad => t("Aus"),
        State::Unknown => t("Unbekannt"),
    }
}

/// Knackzeit aus `pwcheck` (deutscher Text, z. B. «ca. 3 Tage») in die gewählte Sprache.
fn crack_text(s: &str) -> String {
    match s {
        "sofort" => return t("sofort").to_string(),
        "ein paar Sekunden" => return t("ein paar Sekunden").to_string(),
        "praktisch unknackbar" => return t("praktisch unknackbar").to_string(),
        _ => {}
    }
    if let Some(rest) = s.strip_prefix("ca. ") {
        if let Some((n, unit)) = rest.split_once(' ') {
            let tpl = match unit {
                "Minuten" => Some("ca. {n} Minuten"),
                "Stunden" => Some("ca. {n} Stunden"),
                "Tage" => Some("ca. {n} Tage"),
                "Monate" => Some("ca. {n} Monate"),
                "Jahre" => Some("ca. {n} Jahre"),
                "Jahrhunderte" => Some("ca. {n} Jahrhunderte"),
                _ => None,
            };
            if let Some(tpl) = tpl {
                return tf(tpl, &[("n", &n)]);
            }
        }
    }
    s.to_string()
}


fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        path.to_string()
    } else {
        format!("…\\{}\\{}", parts[parts.len() - 2], parts[parts.len() - 1])
    }
}
fn human_size(b: u64) -> String {
    if b < 1024 {
        return format!("{b} B");
    }
    let u = ["KB", "MB", "GB", "TB"];
    let mut v = b as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < u.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, u[i]).replace(".0", "")
}
