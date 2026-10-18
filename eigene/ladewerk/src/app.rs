//! Oberfläche (egui): oben die Reiter Downloads, Linksammler und Einstellungen.
//! Downloads zeigt die Pakete als Karten mit ihren Links, unten die Details zum gewählten
//! Link. Die eigentliche Arbeit macht der Verteiler in engine.rs; die Oberfläche liest und
//! ändert dessen Zustand nur unter einer kurzen Sperre.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::clip;
use crate::engine::{Engine, Model};
use crate::links::{self, Found};
use crate::net::{self, lock};
use crate::plan;
use crate::sprache::{t, tf};
use crate::store::{Item, Package, Status, Unpack};
use crate::thema;
use crate::unpack;

/// Akzentfarbe (Petrol) für Schrift und Linien – je Design lesbar.
fn accent() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0x0e, 0x74, 0x90), egui::Color32::from_rgb(0x67, 0xd4, 0xe8))
}
/// Füllung mit weisser Schrift darauf – in beiden Designs gleich.
fn accent_fill() -> egui::Color32 {
    egui::Color32::from_rgb(0x0e, 0x74, 0x90)
}
fn accent_soft() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xe0, 0xf4, 0xf8), egui::Color32::from_rgb(0x16, 0x3a, 0x44))
}
fn green() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0x15, 0x80, 0x3d), egui::Color32::from_rgb(0x4a, 0xde, 0x80))
}
fn red() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xb9, 0x1c, 0x1c), egui::Color32::from_rgb(0xf8, 0x71, 0x71))
}
fn amber() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xb4, 0x53, 0x09), egui::Color32::from_rgb(0xfb, 0xbf, 0x24))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Downloads,
    Collector,
    Settings,
}

/// Was die Oberfläche nach dem Zeichnen ausführen soll (ausserhalb der Sperre, wo nötig).
enum Act {
    Pause(u64),
    Resume(u64),
    Reset(u64),
    Remove(u64, bool),
    PausePkg(u64),
    ResumePkg(u64),
    RemovePkg(u64, bool),
    Unpack(u64),
    Toggle(u64),
    OpenDir(String),
    PickDownloadDir,
    PickCollectorDir,
    StartAll,
    StopAll,
    RemoveFinished,
    Check(u64),
    Ask(Confirm),
}

/// Rückfrage vor dem Löschen von Teildateien.
#[derive(Clone, Copy)]
enum Confirm {
    RemoveItem(u64),
    RemovePkg(u64),
}

struct Collector {
    text: String,
    found: Vec<(Found, bool)>,
    name: String,
    /// Name folgt dem ersten Link, bis der Benutzer ihn ändert.
    name_auto: bool,
    dir: String,
    sub_folder: bool,
    auto_extract: bool,
    passwords: String,
}

impl Collector {
    fn refresh(&mut self) {
        let old = std::mem::take(&mut self.found);
        self.found = links::extract_links(&self.text)
            .into_iter()
            .map(|f| {
                let on = old.iter().find(|(o, _)| o.url == f.url).map(|(_, on)| *on).unwrap_or(true);
                (f, on)
            })
            .collect();
        if self.name_auto {
            self.name = self
                .found
                .first()
                .map(|(f, _)| match links::file_name_from_url(&f.url) {
                    Some(n) => plan::package_name_from(&n),
                    None => links::host_of(&f.url),
                })
                .unwrap_or_default();
        }
    }

    fn add_text(&mut self, text: &str) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
        self.text.push_str(text);
        self.refresh();
    }

    fn clear(&mut self) {
        self.text.clear();
        self.found.clear();
        self.name.clear();
        self.name_auto = true;
        self.passwords.clear();
    }
}

/// Prüfsumme eines fertigen Downloads nachträglich berechnen.
struct Check {
    id: u64,
    cancel: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Result<String, String>>>>,
}

pub struct App {
    engine: Engine,
    clip: clip::Watcher,
    tab: Tab,
    sel: Option<u64>,
    col: Collector,
    /// Bearbeitungspuffer für die Passwortliste in den Einstellungen.
    pw_text: String,
    confirm: Option<Confirm>,
    check: Option<Check>,
    /// Ergebnis der letzten nachträglichen Prüfung (Link, Text, in Ordnung?).
    check_msg: Option<(u64, String, bool)>,
    clip_new: usize,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        let engine = Engine::start(cc.egui_ctx.clone());
        let (watch, dir, sub, extract, pw_text) = {
            let g = lock(&engine.model);
            let s = &g.state.settings;
            (s.watch_clipboard, s.download_dir.clone(), s.package_subfolder, s.auto_extract, s.passwords.join("\n"))
        };
        App {
            clip: clip::Watcher::start(cc.egui_ctx.clone(), watch),
            engine,
            tab: Tab::Downloads,
            sel: None,
            col: Collector {
                text: String::new(),
                found: Vec::new(),
                name: String::new(),
                name_auto: true,
                dir,
                sub_folder: sub,
                auto_extract: extract,
                passwords: String::new(),
            },
            pw_text,
            confirm: None,
            check: None,
            check_msg: None,
            clip_new: 0,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Aus der Zwischenablage gesammelte Links (nur wenn etwas Neues dabei ist, z. B. nicht
        // der eben mit «Link kopieren» kopierte Link).
        let clipped = self.clip.take();
        let queued = if clipped.is_empty() { Vec::new() } else { self.queued_urls() };
        for text in clipped {
            let fresh = links::extract_links(&text).iter().any(|f| !queued.contains(&f.url) && !self.col.found.iter().any(|(c, _)| c.url == f.url));
            if !fresh {
                continue;
            }
            let before = self.col.found.len();
            self.col.add_text(&text);
            self.clip_new += self.col.found.len().saturating_sub(before);
        }
        // Einfügen (Strg+V), während kein Textfeld aktiv ist: direkt in den Linksammler.
        let pasted: Vec<String> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Paste(s) => Some(s.clone()),
                    _ => None,
                })
                .collect()
        });
        if !pasted.is_empty() && ctx.memory(|m| m.focused().is_none()) {
            for p in &pasted {
                self.col.add_text(p);
            }
            self.tab = Tab::Collector;
        }
        if self.tab == Tab::Collector {
            self.clip_new = 0;
        }
        self.poll_check();

        let mut acts: Vec<Act> = Vec::new();
        let model = self.engine.model.clone();
        let mut g = lock(&model);
        let settings_before = g.state.settings.clone();
        self.top_bar(ctx, &g);
        match self.tab {
            Tab::Downloads => {
                self.details(ctx, &mut g, &mut acts);
                self.downloads(ctx, &g, &mut acts);
            }
            Tab::Collector => self.collector(ctx, &mut g, &mut acts),
            Tab::Settings => self.settings(ctx, &mut g, &mut acts),
        }
        if g.state.settings != settings_before {
            g.dirty = true;
        }
        let watch = g.state.settings.watch_clipboard;
        drop(g);
        self.clip.set_enabled(watch);
        self.confirm_window(ctx, &mut acts);
        self.apply(acts);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(c) = &self.check {
            c.cancel.store(true, Ordering::Relaxed);
        }
        self.engine.shutdown();
    }
}

impl App {
    fn queued_urls(&self) -> Vec<String> {
        let g = lock(&self.engine.model);
        g.state.packages.iter().flat_map(|p| p.items.iter().map(|i| i.url.clone())).collect()
    }

    fn apply(&mut self, acts: Vec<Act>) {
        for a in acts {
            match a {
                Act::PickDownloadDir => {
                    if let Some(d) = rfd::FileDialog::new().set_title(t("Download-Ordner wählen")).pick_folder() {
                        let mut g = lock(&self.engine.model);
                        g.state.settings.download_dir = d.display().to_string();
                        g.dirty = true;
                    }
                }
                Act::PickCollectorDir => {
                    if let Some(d) = rfd::FileDialog::new().set_title(t("Zielordner wählen")).pick_folder() {
                        self.col.dir = d.display().to_string();
                    }
                }
                Act::OpenDir(d) => open_dir(&d),
                Act::Ask(c) => self.confirm = Some(c),
                Act::Check(id) => self.start_check(id),
                a => {
                    let mut g = lock(&self.engine.model);
                    let m: &mut Model = &mut g;
                    match a {
                        Act::Pause(id) => m.pause(id),
                        Act::Resume(id) => m.resume(id),
                        Act::Reset(id) => m.reset(id),
                        Act::Remove(id, del) => {
                            m.remove(id, del);
                            if self.sel == Some(id) {
                                self.sel = None;
                            }
                        }
                        Act::PausePkg(p) => m.pause_package(p),
                        Act::ResumePkg(p) => m.resume_package(p),
                        Act::RemovePkg(p, del) => {
                            if self.sel.is_some_and(|s| m.item_ids(p).contains(&s)) {
                                self.sel = None;
                            }
                            m.remove_package(p, del);
                        }
                        Act::Unpack(p) => m.request_unpack(p),
                        Act::Toggle(p) => {
                            if let Some(pk) = m.state.packages.iter_mut().find(|x| x.id == p) {
                                pk.expanded = !pk.expanded;
                                m.dirty = true;
                            }
                        }
                        Act::StartAll => m.start_all(),
                        Act::StopAll => m.stop_all(),
                        Act::RemoveFinished => m.remove_finished(),
                        _ => {}
                    }
                }
            }
        }
    }

    fn start_check(&mut self, id: u64) {
        let path = {
            let g = lock(&self.engine.model);
            g.state.find_item(id).and_then(|(p, i)| {
                let pk = g.state.packages.get(p)?;
                let it = pk.items.get(i)?;
                Some(Path::new(&pk.dir).join(&it.file_name))
            })
        };
        let Some(path) = path else { return };
        if let Some(old) = self.check.take() {
            old.cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let result: Arc<Mutex<Option<Result<String, String>>>> = Arc::default();
        let (c, r) = (cancel.clone(), result.clone());
        std::thread::spawn(move || {
            let res = match net::sha256_file(&path, &c) {
                Ok(Some(h)) => Ok(h),
                Ok(None) => Err(String::new()),
                Err(e) => Err(e.to_string()),
            };
            *lock(&r) = Some(res);
        });
        self.check = Some(Check { id, cancel, result });
        self.check_msg = None;
    }

    fn poll_check(&mut self) {
        let Some(c) = &self.check else { return };
        let Some(res) = lock(&c.result).take() else { return };
        let id = c.id;
        self.check = None;
        let mut g = lock(&self.engine.model);
        let m: &mut Model = &mut g;
        let expected = m.state.item_mut(id).map(|i| i.sha256.clone()).unwrap_or_default();
        match res {
            Ok(h) => {
                let ok = links::parse_sha256(&expected).map(|e| e == h);
                if let (Some(ok), Some(it)) = (ok, m.state.item_mut(id)) {
                    it.sha_ok = Some(ok);
                    m.dirty = true;
                }
                self.check_msg = Some((id, h, ok.unwrap_or(true)));
            }
            Err(e) if e.is_empty() => {}
            Err(e) => self.check_msg = Some((id, tf("Prüfsumme nicht berechenbar: {e}", &[("e", &e)]), false)),
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context, m: &Model) {
        let count: usize = m.state.packages.iter().map(|p| p.items.len()).sum();
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::symmetric(14.0, 8.0)).stroke(egui::Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 7.0, accent_fill());
                    paint_logo(ui.painter(), r);
                    ui.add_space(4.0);
                    ui.heading("Ladewerk");
                    ui.add_space(12.0);
                    let tabs = [
                        (Tab::Downloads, tf("Downloads ({n})", &[("n", &count)])),
                        (
                            Tab::Collector,
                            if self.clip_new > 0 { tf("Linksammler (+{n})", &[("n", &self.clip_new)]) } else { t("Linksammler").to_string() },
                        ),
                        (Tab::Settings, t("Einstellungen").to_string()),
                    ];
                    for (tab, label) in tabs {
                        let sel = self.tab == tab;
                        let text = egui::RichText::new(label).color(if sel { accent() } else { thema::text() });
                        let text = if sel { text.strong() } else { text };
                        if ui.selectable_label(sel, text).clicked() {
                            self.tab = tab;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let speed = plan::format_speed(m.total_speed());
                        let n = m.active_count();
                        ui.label(egui::RichText::new(speed).strong().color(accent()));
                        ui.label(egui::RichText::new(tf("{n} aktiv", &[("n", &n)])).color(thema::muted()));
                        if !m.active {
                            ui.label(egui::RichText::new(t("Gestoppt")).color(amber()));
                        }
                        if m.state.settings.speed_limit_kib > 0 {
                            let lim = plan::format_speed(m.state.settings.speed_limit_kib as f64 * 1024.0);
                            ui.label(egui::RichText::new(tf("Grenze {v}", &[("v", &lim)])).color(thema::muted()));
                        }
                    });
                });
            });
    }

    fn downloads(&mut self, ctx: &egui::Context, m: &Model, acts: &mut Vec<Act>) {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(thema::bg()).inner_margin(12.0)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if m.active {
                    if ui.button(t("⏹ Alle stoppen")).on_hover_text(t("Laufende Downloads anhalten; sie bleiben in der Warteschlange.")).clicked() {
                        acts.push(Act::StopAll);
                    }
                } else if ui
                    .add(egui::Button::new(egui::RichText::new(t("▶ Starten")).color(egui::Color32::WHITE)).fill(accent_fill()))
                    .clicked()
                {
                    acts.push(Act::StartAll);
                }
                if ui.button(t("+ Links hinzufügen")).clicked() {
                    self.tab = Tab::Collector;
                }
                if ui.button(t("Fertige entfernen")).on_hover_text(t("Entfernt fertige Links aus der Liste. Die Dateien bleiben erhalten.")).clicked() {
                    acts.push(Act::RemoveFinished);
                }
            });
            ui.add_space(8.0);
            if m.state.packages.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(t("Noch keine Downloads.")).size(16.0).color(thema::muted()));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(t("Links kopieren und hier mit Strg+V einfügen – oder im Linksammler einfügen.")).color(thema::muted()));
                });
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                for p in &m.state.packages {
                    self.package_card(ui, m, p, acts);
                    ui.add_space(8.0);
                }
            });
        });
    }

    fn package_card(&mut self, ui: &mut egui::Ui, m: &Model, p: &Package, acts: &mut Vec<Act>) {
        let done_items = p.items.iter().filter(|i| i.status == Status::Done).count();
        let total: u64 = p.items.iter().map(|i| i.size.unwrap_or(i.done)).sum();
        let done: u64 = p.items.iter().map(|i| i.done).sum();
        let all_known = p.items.iter().all(|i| i.size.is_some());
        let speed: f64 = p.items.iter().map(|i| m.speed(i.id)).sum();
        let busy = p.items.iter().any(|i| matches!(i.status, Status::Waiting | Status::Running));
        egui::Frame::none()
            .fill(thema::panel())
            .stroke(egui::Stroke::new(1.0, thema::border()))
            .rounding(8.0)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if ui.small_button(if p.expanded { "▼" } else { "▶" }).clicked() {
                        acts.push(Act::Toggle(p.id));
                    }
                    ui.add_sized([240.0, 20.0], egui::Label::new(egui::RichText::new(&p.name).strong()).truncate()).on_hover_text(p.dir.as_str());
                    ui.label(egui::RichText::new(tf("{a}/{b} fertig", &[("a", &done_items), ("b", &p.items.len())])).color(thema::muted()));
                    let frac = if all_known { plan::fraction(done, Some(total)) } else { 0.0 };
                    let size_txt = if all_known {
                        format!("{} / {}", plan::format_bytes(done), plan::format_bytes(total))
                    } else {
                        plan::format_bytes(done)
                    };
                    ui.add(egui::ProgressBar::new(frac).desired_width(180.0).text(size_txt).fill(if done_items == p.items.len() { green() } else { accent_fill() }));
                    if speed > 0.0 {
                        ui.label(egui::RichText::new(plan::format_speed(speed)).color(accent()));
                    }
                    match p.unpack {
                        Unpack::Running => {
                            ui.label(egui::RichText::new(if p.unpack_msg.is_empty() { t("Entpacke …").to_string() } else { p.unpack_msg.clone() }).color(accent()));
                        }
                        Unpack::Done if !p.unpack_msg.is_empty() => {
                            ui.label(egui::RichText::new(format!("✔ {}", p.unpack_msg)).color(green()));
                        }
                        Unpack::Failed => {
                            ui.label(egui::RichText::new(t("Entpacken fehlgeschlagen")).color(red())).on_hover_text(p.unpack_msg.as_str());
                        }
                        _ => {}
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("✖", |ui| {
                            if ui.button(t("Paket entfernen (Dateien bleiben erhalten)")).clicked() {
                                acts.push(Act::RemovePkg(p.id, false));
                                ui.close_menu();
                            }
                            if ui.button(t("Entfernen und Teildateien löschen …")).clicked() {
                                acts.push(Act::Ask(Confirm::RemovePkg(p.id)));
                                ui.close_menu();
                            }
                        });
                        if ui.small_button("📂").on_hover_text(t("Ordner öffnen")).clicked() {
                            acts.push(Act::OpenDir(p.dir.clone()));
                        }
                        let names: Vec<String> = p.items.iter().map(|i| i.file_name.clone()).collect();
                        let has_archives = !plan::archives_to_extract(&names).is_empty();
                        if has_archives && done_items == p.items.len() && p.unpack != Unpack::Running && ui.small_button(t("Entpacken")).clicked() {
                            acts.push(Act::Unpack(p.id));
                        }
                        if busy {
                            if ui.small_button("⏸").on_hover_text(t("Paket pausieren")).clicked() {
                                acts.push(Act::PausePkg(p.id));
                            }
                        } else if done_items < p.items.len() && ui.small_button("▶").on_hover_text(t("Paket fortsetzen")).clicked() {
                            acts.push(Act::ResumePkg(p.id));
                        }
                    });
                });
                if p.expanded {
                    ui.add_space(4.0);
                    for it in &p.items {
                        self.item_row(ui, m, it, acts);
                    }
                }
            });
    }

    fn item_row(&mut self, ui: &mut egui::Ui, m: &Model, it: &Item, acts: &mut Vec<Act>) {
        let selected = self.sel == Some(it.id);
        let name = if it.file_name.is_empty() {
            links::file_name_from_url(&it.url).unwrap_or_else(|| it.url.clone())
        } else {
            it.file_name.clone()
        };
        let speed = m.speed(it.id);
        let frame = egui::Frame::none().fill(if selected { accent_soft() } else { egui::Color32::TRANSPARENT }).rounding(6.0).inner_margin(egui::Margin::symmetric(6.0, 3.0));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                let resp = ui
                    .add_sized([280.0, 18.0], egui::Label::new(egui::RichText::new(&name).color(thema::text())).truncate().sense(egui::Sense::click()))
                    .on_hover_text(it.url.as_str());
                if resp.clicked() {
                    self.sel = Some(it.id);
                }
                resp.context_menu(|ui| {
                    self.sel = Some(it.id);
                    item_menu(ui, it, acts);
                });
                let size = it.size.map(plan::format_bytes).unwrap_or_else(|| "?".to_string());
                ui.add_sized([80.0, 18.0], egui::Label::new(egui::RichText::new(size).color(thema::muted())));
                let frac = if it.status == Status::Done { 1.0 } else { plan::fraction(it.done, it.size) };
                let pct = format!("{:.0} %", frac * 100.0);
                let fill = match it.status {
                    Status::Done => green(),
                    Status::Failed => red(),
                    Status::Paused => thema::muted(),
                    _ => accent_fill(),
                };
                ui.add(egui::ProgressBar::new(frac).desired_width(150.0).text(pct).fill(fill));
                let (speed_txt, eta_txt) = if it.status == Status::Running && m.running(it.id) {
                    let eta = it.size.and_then(|s| plan::eta(s.saturating_sub(it.done), speed)).map(plan::format_eta).unwrap_or_default();
                    (plan::format_speed(speed), eta)
                } else {
                    (String::new(), String::new())
                };
                ui.add_sized([86.0, 18.0], egui::Label::new(egui::RichText::new(speed_txt).color(accent())));
                ui.add_sized([60.0, 18.0], egui::Label::new(egui::RichText::new(eta_txt).color(thema::muted())));
                let (status, color) = status_text(m, it);
                let st = ui.add_sized([150.0, 18.0], egui::Label::new(egui::RichText::new(status).color(color)).truncate());
                if !it.error.is_empty() {
                    st.on_hover_text(it.error.as_str());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = ui.menu_button("…", |ui| item_menu(ui, it, acts)).response.on_hover_text(t("Weitere Aktionen"));
                    match it.status {
                        Status::Waiting | Status::Running => {
                            if ui.small_button("⏸").on_hover_text(t("Pausieren")).clicked() {
                                acts.push(Act::Pause(it.id));
                            }
                        }
                        Status::Paused => {
                            if ui.small_button("▶").on_hover_text(t("Fortsetzen")).clicked() {
                                acts.push(Act::Resume(it.id));
                            }
                        }
                        Status::Failed => {
                            if ui.small_button("↻").on_hover_text(t("Erneut versuchen")).clicked() {
                                acts.push(Act::Resume(it.id));
                            }
                        }
                        Status::Done => {}
                    }
                });
            });
        });
    }

    fn details(&mut self, ctx: &egui::Context, m: &mut Model, acts: &mut Vec<Act>) {
        let Some(id) = self.sel else { return };
        let Some((pi, ii)) = m.state.find_item(id) else {
            self.sel = None;
            return;
        };
        let dir = m.state.packages.get(pi).map(|p| p.dir.clone()).unwrap_or_default();
        let checking = self.check.as_ref().is_some_and(|c| c.id == id);
        let check_msg = self.check_msg.clone().filter(|(cid, _, _)| *cid == id);
        let mut changed = false;
        egui::TopBottomPanel::bottom("details")
            .frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::symmetric(14.0, 10.0)).stroke(egui::Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| {
                let Some(it) = m.state.packages.get_mut(pi).and_then(|p| p.items.get_mut(ii)) else { return };
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t("DETAILS")).size(11.0).strong().color(thema::muted()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✖").on_hover_text(t("Schliessen")).clicked() {
                            self.sel = None;
                        }
                    });
                });
                egui::Grid::new("details_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label(egui::RichText::new(t("Link")).color(thema::muted()));
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(it.url.as_str()).truncate());
                        if ui.small_button(t("Kopieren")).clicked() {
                            ui.output_mut(|o| o.copied_text = it.url.clone());
                        }
                    });
                    ui.end_row();
                    ui.label(egui::RichText::new(t("Datei")).color(thema::muted()));
                    ui.horizontal(|ui| {
                        let file = if it.file_name.is_empty() { t("(noch unbekannt)").to_string() } else { Path::new(&dir).join(&it.file_name).display().to_string() };
                        ui.add(egui::Label::new(file).truncate());
                        if ui.small_button(t("Ordner öffnen")).clicked() {
                            acts.push(Act::OpenDir(dir.clone()));
                        }
                    });
                    ui.end_row();
                    ui.label(egui::RichText::new(t("SHA-256")).color(thema::muted()));
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut it.sha256).desired_width(470.0).hint_text(t("optional: erwartete Prüfsumme")).font(egui::TextStyle::Monospace));
                        if r.changed() {
                            it.sha_ok = None;
                            changed = true;
                        }
                        let valid = links::parse_sha256(&it.sha256).is_some();
                        if !it.sha256.trim().is_empty() && !valid {
                            ui.label(egui::RichText::new(t("ungültig (64 Hex-Zeichen)")).color(red()));
                        }
                        match it.sha_ok {
                            Some(true) => {
                                ui.label(egui::RichText::new(t("✔ stimmt")).color(green()));
                            }
                            Some(false) => {
                                ui.label(egui::RichText::new(t("✖ stimmt nicht")).color(red()));
                            }
                            None => {}
                        }
                        if it.status == Status::Done {
                            if checking {
                                ui.spinner();
                            } else if ui.small_button(t("Jetzt prüfen")).on_hover_text(t("SHA-256 der fertigen Datei berechnen")).clicked() {
                                acts.push(Act::Check(id));
                            }
                        }
                    });
                    ui.end_row();
                    if let Some((_, msg, ok)) = &check_msg {
                        ui.label("");
                        ui.add(egui::Label::new(egui::RichText::new(msg).monospace().color(if *ok { thema::muted() } else { red() })).wrap());
                        ui.end_row();
                    }
                    if !it.error.is_empty() {
                        ui.label(egui::RichText::new(t("Meldung")).color(thema::muted()));
                        ui.add(egui::Label::new(egui::RichText::new(&it.error).color(if it.status == Status::Failed { red() } else { amber() })).wrap());
                        ui.end_row();
                    }
                });
            });
        if changed {
            m.dirty = true;
        }
    }

    fn collector(&mut self, ctx: &egui::Context, m: &mut Model, acts: &mut Vec<Act>) {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(thema::bg()).inner_margin(12.0)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                card(ui, |ui| {
                    ui.label(egui::RichText::new(t("LINKSAMMLER")).size(11.0).strong().color(thema::muted()));
                    ui.label(egui::RichText::new(t("Text mit Links hier einfügen – Links werden automatisch erkannt. Steht hinter einem Link eine SHA-256-Prüfsumme, wird sie übernommen.")).color(thema::muted()));
                    ui.add_space(4.0);
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut self.col.text)
                            .desired_rows(6)
                            .desired_width(f32::INFINITY)
                            .hint_text(t("https://… (mehrere Links, ganze Texte oder HTML gehen auch)")),
                    );
                    if r.changed() {
                        self.col.refresh();
                    }
                    ui.horizontal(|ui| {
                        if ui.button(t("Leeren")).clicked() {
                            self.col.clear();
                        }
                        let watch = &mut m.state.settings.watch_clipboard;
                        ui.checkbox(watch, t("Zwischenablage beobachten")).on_hover_text(t("Kopierte Links landen automatisch hier."));
                    });
                });
                ui.add_space(10.0);
                card(ui, |ui| {
                    let n = self.col.found.len();
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(tf("GEFUNDENE LINKS ({n})", &[("n", &n)])).size(11.0).strong().color(thema::muted()));
                        if n > 0 {
                            if ui.small_button(t("Alle")).clicked() {
                                self.col.found.iter_mut().for_each(|f| f.1 = true);
                            }
                            if ui.small_button(t("Keine")).clicked() {
                                self.col.found.iter_mut().for_each(|f| f.1 = false);
                            }
                        }
                    });
                    if n == 0 {
                        ui.label(egui::RichText::new(t("Noch keine Links erkannt.")).color(thema::muted()));
                    }
                    for (f, on) in self.col.found.iter_mut() {
                        let queued = m.state.packages.iter().any(|p| p.items.iter().any(|i| i.url == f.url));
                        ui.horizontal(|ui| {
                            ui.checkbox(on, "");
                            if queued {
                                ui.label(egui::RichText::new(t("schon in der Warteschlange")).small().color(amber()));
                            }
                            let name = links::file_name_from_url(&f.url).unwrap_or_else(|| links::host_of(&f.url));
                            ui.add_sized([220.0, 18.0], egui::Label::new(egui::RichText::new(name).strong()).truncate());
                            ui.add(egui::Label::new(egui::RichText::new(&f.url).color(thema::muted())).truncate());
                            if f.sha256.is_some() {
                                ui.label(egui::RichText::new("SHA-256").small().color(green()));
                            }
                        });
                    }
                });
                ui.add_space(10.0);
                card(ui, |ui| {
                    ui.label(egui::RichText::new(t("PAKET")).size(11.0).strong().color(thema::muted()));
                    egui::Grid::new("collector_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label(t("Paketname"));
                        if ui.add(egui::TextEdit::singleline(&mut self.col.name).desired_width(360.0).hint_text(t("Name des Pakets"))).changed() {
                            self.col.name_auto = false;
                        }
                        ui.end_row();
                        ui.label(t("Zielordner"));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.col.dir).desired_width(360.0));
                            if ui.button(t("Wählen …")).clicked() {
                                acts.push(Act::PickCollectorDir);
                            }
                        });
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut self.col.sub_folder, t("Eigener Unterordner mit dem Paketnamen"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut self.col.auto_extract, t("Archive nach dem Download entpacken"));
                        ui.end_row();
                        ui.label(t("Passwörter"));
                        ui.add(egui::TextEdit::multiline(&mut self.col.passwords).desired_rows(2).desired_width(360.0).hint_text(t("für Archive in diesem Paket, eines pro Zeile")));
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    let chosen = self.col.found.iter().filter(|f| f.1).count();
                    let target = self.package_dir();
                    ui.label(egui::RichText::new(tf("Speichern in: {dir}", &[("dir", &target.display())])).color(thema::muted()));
                    ui.add_space(4.0);
                    let label = egui::RichText::new(tf("{n} Link(s) zur Warteschlange hinzufügen", &[("n", &chosen)])).color(egui::Color32::WHITE);
                    if ui.add_enabled(chosen > 0, egui::Button::new(label).fill(accent_fill())).clicked() {
                        self.add_collected(m, target);
                    }
                });
            });
        });
    }

    fn package_dir(&self) -> PathBuf {
        let base = PathBuf::from(self.col.dir.trim());
        if self.col.sub_folder {
            let name = links::sanitize_file_name(&self.col.name).unwrap_or_else(|| t("Paket").to_string());
            base.join(name)
        } else {
            base
        }
    }

    fn add_collected(&mut self, m: &mut Model, dir: PathBuf) {
        let name = if self.col.name.trim().is_empty() { t("Paket").to_string() } else { self.col.name.trim().to_string() };
        let id = m.state.new_id();
        let mut p = Package::new(id, &name, &dir.to_string_lossy());
        p.auto_extract = self.col.auto_extract;
        p.passwords = self.col.passwords.lines().map(|l| l.trim_end_matches('\r').to_string()).filter(|l| !l.is_empty()).collect();
        for (f, on) in &self.col.found {
            if !*on {
                continue;
            }
            let mut it = Item::new(m.state.new_id(), &f.url);
            it.sha256 = f.sha256.clone().unwrap_or_default();
            p.items.push(it);
        }
        if p.items.is_empty() {
            return;
        }
        m.add_package(p);
        self.col.clear();
        self.tab = Tab::Downloads;
    }

    fn settings(&mut self, ctx: &egui::Context, m: &mut Model, acts: &mut Vec<Act>) {
        egui::CentralPanel::default().frame(egui::Frame::none().fill(thema::bg()).inner_margin(12.0)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                let s = &mut m.state.settings;
                card(ui, |ui| {
                    ui.label(egui::RichText::new(t("DOWNLOADS")).size(11.0).strong().color(thema::muted()));
                    egui::Grid::new("settings_dl").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label(t("Download-Ordner"));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut s.download_dir).desired_width(380.0));
                            if ui.button(t("Wählen …")).clicked() {
                                acts.push(Act::PickDownloadDir);
                            }
                        });
                        ui.end_row();
                        ui.label(t("Gleichzeitige Downloads"));
                        ui.add(egui::DragValue::new(&mut s.max_parallel).range(1..=20).speed(0.1));
                        ui.end_row();
                        ui.label(t("Geschwindigkeitsgrenze"));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut s.speed_limit_kib).range(0..=10_000_000).speed(16.0).suffix(" KB/s"));
                            ui.label(egui::RichText::new(t("0 = unbegrenzt, gilt für alle Downloads zusammen")).color(thema::muted()));
                        });
                        ui.end_row();
                        ui.label(t("Wiederholen bei Fehlern"));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut s.retries).range(0..=100).speed(0.1));
                            ui.label(t("Mal, erste Wartezeit"));
                            ui.add(egui::DragValue::new(&mut s.retry_delay_s).range(1..=3600).speed(0.2).suffix(" s"));
                        });
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut s.package_subfolder, t("Neue Pakete in einen eigenen Unterordner"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut s.autostart, t("Downloads beim Programmstart fortsetzen"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut s.watch_clipboard, t("Zwischenablage beobachten"));
                        ui.end_row();
                    });
                });
                ui.add_space(10.0);
                card(ui, |ui| {
                    ui.label(egui::RichText::new(t("ENTPACKEN")).size(11.0).strong().color(thema::muted()));
                    egui::Grid::new("settings_unpack").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("");
                        ui.checkbox(&mut s.auto_extract, t("Neue Pakete nach dem Download automatisch entpacken"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut s.extract_subfolder, t("Jedes Archiv in einen eigenen Ordner entpacken"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut s.delete_archives, t("Archive nach erfolgreichem Entpacken löschen"));
                        ui.end_row();
                        ui.label(t("Passwortliste"));
                        let r = ui.add(egui::TextEdit::multiline(&mut self.pw_text).desired_rows(4).desired_width(380.0).hint_text(t("ein Passwort pro Zeile")));
                        if r.changed() {
                            s.passwords = self.pw_text.lines().map(|l| l.trim_end_matches('\r').to_string()).filter(|l| !l.is_empty()).collect();
                        }
                        ui.end_row();
                        ui.label(t("7-Zip"));
                        let txt = match unpack::find_7z() {
                            Some(p) => tf("gefunden: {p}", &[("p", &p.display())]),
                            None => t("nicht gefunden – ZIP geht trotzdem; für 7z und RAR 7-Zip installieren").to_string(),
                        };
                        ui.label(egui::RichText::new(txt).color(thema::muted()));
                        ui.end_row();
                    });
                });
                ui.add_space(10.0);
                card(ui, |ui| {
                    ui.label(egui::RichText::new(t("VERBINDUNG")).size(11.0).strong().color(thema::muted()));
                    egui::Grid::new("settings_net").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label(t("Proxy"));
                        ui.add(egui::TextEdit::singleline(&mut s.proxy).desired_width(380.0).hint_text(t("leer = aus der Umgebung (HTTPS_PROXY, ALL_PROXY)")));
                        ui.end_row();
                    });
                    ui.label(egui::RichText::new(t("Beispiele: http://proxy:3128 oder socks5://127.0.0.1:1080")).color(thema::muted()));
                });
                if !m.save_error.is_empty() {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(tf("Speichern fehlgeschlagen: {e}", &[("e", &m.save_error)])).color(red()));
                }
            });
        });
    }

    fn confirm_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(c) = self.confirm else { return };
        let mut close = false;
        egui::Window::new(t("Teildateien löschen?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(t("Unfertige Downloads werden entfernt und ihre .part-Dateien gelöscht. Fertige Dateien bleiben erhalten."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(egui::RichText::new(t("Löschen")).color(egui::Color32::WHITE)).fill(red())).clicked() {
                        acts.push(match c {
                            Confirm::RemoveItem(id) => Act::Remove(id, true),
                            Confirm::RemovePkg(id) => Act::RemovePkg(id, true),
                        });
                        close = true;
                    }
                    if ui.button(t("Abbrechen")).clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.confirm = None;
        }
    }
}

fn item_menu(ui: &mut egui::Ui, it: &Item, acts: &mut Vec<Act>) {
    if ui.button(t("Link kopieren")).clicked() {
        ui.output_mut(|o| o.copied_text = it.url.clone());
        ui.close_menu();
    }
    if ui.button(t("Neu laden (von vorne)")).clicked() {
        acts.push(Act::Reset(it.id));
        ui.close_menu();
    }
    if ui.button(t("Aus der Liste entfernen")).clicked() {
        acts.push(Act::Remove(it.id, false));
        ui.close_menu();
    }
    if it.status != Status::Done && ui.button(t("Entfernen und Teildatei löschen …")).clicked() {
        acts.push(Act::Ask(Confirm::RemoveItem(it.id)));
        ui.close_menu();
    }
}

fn status_text(m: &Model, it: &Item) -> (String, egui::Color32) {
    match it.status {
        Status::Waiting => {
            let retry = m.rt.get(&it.id).and_then(|r| r.retry_at).filter(|at| *at > Instant::now());
            match retry {
                Some(at) => {
                    let secs = at.saturating_duration_since(Instant::now()).as_secs();
                    (tf("Neuer Versuch in {s} s", &[("s", &secs)]), amber())
                }
                None => (t("Wartet").to_string(), thema::muted()),
            }
        }
        Status::Running => match m.phase(it.id) {
            Some(net::PHASE_CHECK) => (t("Prüft SHA-256 …").to_string(), accent()),
            Some(net::PHASE_LOAD) => (t("Lädt").to_string(), accent()),
            _ => (t("Verbinde …").to_string(), accent()),
        },
        Status::Paused => (t("Pausiert").to_string(), thema::muted()),
        Status::Done => {
            if it.sha_ok == Some(true) {
                (t("Fertig ✔ SHA-256").to_string(), green())
            } else {
                (t("Fertig").to_string(), green())
            }
        }
        Status::Failed => (tf("Fehler: {e}", &[("e", &it.error)]), red()),
    }
}

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(thema::panel())
        .stroke(egui::Stroke::new(1.0, thema::border()))
        .rounding(8.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Pfeil nach unten in eine Schale – das Zeichen von Ladewerk.
fn paint_logo(p: &egui::Painter, r: egui::Rect) {
    let c = r.center();
    let s = egui::Stroke::new(2.2, egui::Color32::WHITE);
    p.line_segment([c + egui::vec2(0.0, -7.0), c + egui::vec2(0.0, 3.0)], s);
    p.line_segment([c + egui::vec2(-4.5, -1.5), c + egui::vec2(0.0, 3.0)], s);
    p.line_segment([c + egui::vec2(4.5, -1.5), c + egui::vec2(0.0, 3.0)], s);
    p.line_segment([c + egui::vec2(-7.0, 3.5), c + egui::vec2(-7.0, 7.0)], s);
    p.line_segment([c + egui::vec2(-7.0, 7.0), c + egui::vec2(7.0, 7.0)], s);
    p.line_segment([c + egui::vec2(7.0, 7.0), c + egui::vec2(7.0, 3.5)], s);
}

/// Ordner im Dateimanager öffnen (wird bei Bedarf angelegt).
fn open_dir(dir: &str) {
    let _ = std::fs::create_dir_all(dir);
    let mut cmd = if cfg!(windows) { std::process::Command::new("explorer") } else { std::process::Command::new("xdg-open") };
    cmd.arg(dir);
    unpack::no_window(&mut cmd);
    let _ = cmd.spawn();
}
