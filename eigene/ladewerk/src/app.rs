//! Oberfläche (egui): Linksammler und Downloads als Reiter, Einstellungen als eigenes Fenster.
//! Alle Arbeit macht der Kern in main.rs; hier wird nur angezeigt und weitergereicht.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Align, Color32, Layout, RichText};

use crate::sprache::{t, tf};
use crate::thema;
use crate::{ItemView, Snapshot, J};

// Schrift- und Linienfarben je Design (hell/dunkel), lesbar auf dem Hintergrund.
fn accent() -> Color32 {
    thema::pick(Color32::from_rgb(0xA8, 0x55, 0x0C), Color32::from_rgb(0xF5, 0xA5, 0x24))
}
/// Füllung der Hauptschaltflächen (weisse Schrift darauf) – in beiden Designs gleich.
fn accent_fill() -> Color32 {
    Color32::from_rgb(0xC9, 0x6A, 0x12)
}
fn green() -> Color32 {
    thema::pick(Color32::from_rgb(0x15, 0x80, 0x3D), Color32::from_rgb(0x4A, 0xDE, 0x80))
}
fn red() -> Color32 {
    thema::pick(Color32::from_rgb(0xB9, 0x1C, 0x1C), Color32::from_rgb(0xF8, 0x71, 0x71))
}
fn blue() -> Color32 {
    thema::pick(Color32::from_rgb(0x25, 0x63, 0xEB), Color32::from_rgb(0x60, 0xA5, 0xFA))
}

fn size(n: Option<u64>) -> String {
    let Some(n) = n else { return "–".into() };
    let units = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut i) = (n as f64, 0);
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", n)
    } else if v < 10.0 {
        format!("{:.2} {}", v, units[i])
    } else {
        format!("{:.1} {}", v, units[i])
    }
}
fn eta(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return String::new();
    }
    let s = secs.round() as u64;
    let (h, m, x) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 {
        format!("{} h {} min", h, m)
    } else if m > 0 {
        format!("{} min {} s", m, x)
    } else {
        format!("{} s", x)
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Grab,
    Downloads,
}

/// Eingabefelder des Einstellungsfensters (werden erst mit «Speichern» übernommen).
#[derive(Default)]
struct Form {
    dir: String,
    max_parallel: u32,
    chunks: u32,
    limit_kb: u64,
    retries: u32,
    clipboard: bool,
    clicknload: bool,
    extract: bool,
    delete_archives: bool,
    passwords: String,
    proxy: String,
}

pub struct Gui {
    tab: Option<Tab>,
    links: String,
    pkg: String,
    sel: HashSet<String>,
    grabbing: Arc<AtomicBool>,
    msgs: Arc<Mutex<Vec<String>>>,
    toast: Option<(String, Instant)>,
    show_settings: bool,
    form: Form,
    acc_host: String,
    acc_user: String,
    acc_pw: String,
    acc_busy: Arc<AtomicBool>,
    extractor: (String, Option<Instant>),
    ask_quit: bool,
    quit_ok: bool,
    /// Detailfenster: Eintrag und Eingabefeld für die erwartete Prüfsumme
    details: Option<(String, String)>,
    /// Passwörter eines Pakets bearbeiten: Paketname und Text
    pkg_edit: Option<(String, String)>,
}

impl Gui {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        Gui {
            tab: None,
            links: String::new(),
            pkg: String::new(),
            sel: HashSet::new(),
            grabbing: Arc::new(AtomicBool::new(false)),
            msgs: Arc::new(Mutex::new(Vec::new())),
            toast: None,
            show_settings: false,
            form: Form::default(),
            acc_host: crate::HOSTERS[0].to_string(),
            acc_user: String::new(),
            acc_pw: String::new(),
            acc_busy: Arc::new(AtomicBool::new(false)),
            extractor: (String::new(), None),
            ask_quit: false,
            quit_ok: false,
            details: None,
            pkg_edit: None,
        }
    }

    fn say(&mut self, m: impl Into<String>) {
        self.toast = Some((m.into(), Instant::now()));
    }

    fn act(&mut self, action: &str, ids: Vec<String>) {
        if let Err(e) = crate::do_action(action, &crate::ids_json(&ids)) {
            self.say(t_dyn(&e));
        }
    }

    /// Links prüfen (im Hintergrund, weil das Netz dauern kann).
    fn grab(&mut self, scan: bool, ctx: &egui::Context) {
        let text = self.links.clone();
        if text.trim().is_empty() {
            self.say(t("Bitte zuerst Links einfügen."));
            return;
        }
        let pkg = self.pkg.clone();
        let (busy, msgs, ctx) = (self.grabbing.clone(), self.msgs.clone(), ctx.clone());
        busy.store(true, Ordering::SeqCst);
        std::thread::spawn(move || {
            let j = crate::grab(&text, &pkg, scan);
            let found = j.get("found").u64().unwrap_or(0);
            let added = j.get("added").u64().unwrap_or(0);
            let dup = j.get("duplicates").u64().unwrap_or(0);
            let notes = j.get("notes").arr().len();
            let mut m = if found == 0 {
                if scan { t("Keine Datei-Links auf den Seiten gefunden.").to_string() } else { t("Keine Links im Text gefunden.").to_string() }
            } else {
                tf("{n} neu im Linksammler", &[("n", &added)])
            };
            if dup > 0 {
                m.push_str(&tf(", {n} schon vorhanden", &[("n", &dup)]));
            }
            if notes > 0 {
                m.push_str(&tf(" · {n} Seite(n) nicht lesbar", &[("n", &notes)]));
            }
            if let Ok(mut q) = msgs.lock() {
                q.push(m);
            }
            busy.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
        self.links.clear();
    }

    fn open_settings(&mut self, s: &Snapshot) {
        let c = &s.settings;
        self.form = Form {
            dir: c.dir.clone(),
            max_parallel: c.max_parallel as u32,
            chunks: c.chunks as u32,
            limit_kb: c.speed_limit_kb,
            retries: c.retries,
            clipboard: c.clipboard,
            clicknload: c.clicknload,
            extract: c.extract,
            delete_archives: c.delete_archives,
            passwords: c.passwords.join("\n"),
            proxy: c.proxy.clone(),
        };
        self.show_settings = true;
    }

    fn save_settings(&mut self) -> bool {
        let f = &self.form;
        let n = |v: u64| J::Num(v as f64);
        let j = J::Obj(vec![
            ("dir".into(), J::Str(f.dir.clone())),
            ("maxParallel".into(), n(f.max_parallel as u64)),
            ("chunks".into(), n(f.chunks as u64)),
            ("speedLimitKB".into(), n(f.limit_kb)),
            ("retries".into(), n(f.retries as u64)),
            ("clipboard".into(), J::Bool(f.clipboard)),
            ("clicknload".into(), J::Bool(f.clicknload)),
            ("extract".into(), J::Bool(f.extract)),
            ("deleteArchives".into(), J::Bool(f.delete_archives)),
            ("passwords".into(), J::Str(f.passwords.clone())),
            ("proxy".into(), J::Str(f.proxy.clone())),
        ]);
        match crate::apply_settings(&j) {
            Ok(()) => {
                self.say(t("Gespeichert."));
                true
            }
            Err(e) => {
                self.say(t_dyn(&e));
                false
            }
        }
    }

    fn extractor_label(&mut self) -> String {
        let stale = self.extractor.1.map(|i| i.elapsed() > Duration::from_secs(10)).unwrap_or(true);
        if stale {
            self.extractor = (crate::extractor_label(), Some(Instant::now()));
        }
        self.extractor.0.clone()
    }

    /* ----- Kopfzeile ----- */
    fn top(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Ladewerk").size(20.0).strong().color(accent()));
            ui.add_space(10.0);
            let (txt, col) = if s.active { (t("Läuft – anhalten"), green()) } else { (t("Angehalten – starten"), accent()) };
            if ui.add(egui::Button::new(RichText::new(txt).color(col).strong())).clicked() {
                crate::set_active(!s.active);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(t("Einstellungen")).clicked() {
                    self.open_settings(s);
                }
                if ui.button(t("Ordner öffnen")).clicked() {
                    crate::open_download_dir();
                }
                if s.speed > 0 {
                    ui.label(RichText::new(format!("{}/s", size(Some(s.speed)))).monospace());
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let n_grab = s.items.iter().filter(|i| i.stage == "grab").count();
            let n_dl = s.items.iter().filter(|i| i.stage == "queue").count();
            let tab = self.tab.unwrap_or(Tab::Grab);
            if ui.selectable_label(tab == Tab::Grab, format!("{} ({})", t("Linksammler"), n_grab)).clicked() {
                self.tab = Some(Tab::Grab);
            }
            if ui.selectable_label(tab == Tab::Downloads, format!("{} ({})", t("Downloads"), n_dl)).clicked() {
                self.tab = Some(Tab::Downloads);
            }
        });
        ui.add_space(4.0);
    }

    /* ----- Linksammler ----- */
    fn grab_tab(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let ctx = ui.ctx().clone();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.links)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .hint_text(t("Links hier einfügen – beliebiger Text, die Adressen werden automatisch herausgesucht.")),
            );
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.pkg).desired_width(240.0).hint_text(t("Paketname (optional, wird Unterordner)")));
                let busy = self.grabbing.load(Ordering::SeqCst);
                let primary = egui::Button::new(RichText::new(t("Links prüfen")).color(Color32::WHITE).strong()).fill(accent_fill());
                if ui.add_enabled(!busy, primary).clicked() {
                    self.grab(false, &ctx);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new(t("Seiten nach Dateien durchsuchen")))
                    .on_hover_text(t("Lädt die angegebenen Seiten und sammelt alle darin verlinkten Dateien"))
                    .clicked()
                {
                    self.grab(true, &ctx);
                }
                if busy {
                    ui.spinner();
                }
            });
        });
        ui.add_space(6.0);
        let items: Vec<&ItemView> = s.items.iter().filter(|i| i.stage == "grab").collect();
        ui.horizontal(|ui| {
            let primary = egui::Button::new(RichText::new(t("Auswahl herunterladen")).color(Color32::WHITE).strong()).fill(accent_fill());
            if ui.add(primary).on_hover_text(t("Ohne Auswahl: alle bereiten Links")).clicked() {
                let mut ids: Vec<String> = items.iter().filter(|i| self.sel.contains(&i.id)).map(|i| i.id.clone()).collect();
                if ids.is_empty() {
                    ids = items.iter().filter(|i| i.status == "ready").map(|i| i.id.clone()).collect();
                }
                if ids.is_empty() {
                    self.say(t("Nichts ausgewählt."));
                } else {
                    self.act("confirm", ids);
                    self.tab = Some(Tab::Downloads);
                }
            }
            if ui.button(t("Auswahl entfernen")).clicked() {
                let ids: Vec<String> = items.iter().filter(|i| self.sel.contains(&i.id)).map(|i| i.id.clone()).collect();
                self.act("remove", ids);
            }
            if ui.button(t("Nicht erreichbare entfernen")).clicked() {
                crate::clear_items(false);
            }
            if !items.is_empty() {
                let ready = items.iter().filter(|i| i.status == "ready").count();
                ui.weak(tf("{a} von {b} bereit", &[("a", &ready), ("b", &items.len())]));
            }
        });
        ui.add_space(4.0);
        if items.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.weak(t("Noch keine Links im Linksammler. Oben einfügen und «Links prüfen» drücken – oder Links kopieren bzw. Click'n'Load auf einer Webseite nutzen."));
            });
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.list(ui, &items, true, &s.pkg_passwords);
        });
    }

    /* ----- Downloads ----- */
    fn dl_tab(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let items: Vec<&ItemView> = s.items.iter().filter(|i| i.stage == "queue").collect();
        let chosen = |sel: &HashSet<String>| -> Vec<String> { items.iter().filter(|i| sel.contains(&i.id)).map(|i| i.id.clone()).collect() };
        ui.horizontal(|ui| {
            for (label, action) in [(t("Auswahl pausieren"), "pause"), (t("Auswahl fortsetzen"), "resume"), (t("Auswahl entfernen"), "remove")] {
                if ui.button(label).clicked() {
                    let ids = chosen(&self.sel);
                    if ids.is_empty() {
                        self.say(t("Nichts ausgewählt."));
                    } else {
                        self.act(action, ids);
                    }
                }
            }
            if ui.button(t("Fertige ausblenden")).clicked() {
                crate::clear_items(true);
            }
            if !items.is_empty() {
                let done = items.iter().filter(|i| i.status == "done").count();
                ui.weak(tf("{a} von {b} fertig", &[("a", &done), ("b", &items.len())]));
            }
        });
        ui.add_space(4.0);
        if items.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.weak(t("Keine Downloads in der Liste."));
            });
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.list(ui, &items, false, &s.pkg_passwords);
        });
    }

    /// Einträge nach Paket gruppiert.
    fn list(&mut self, ui: &mut egui::Ui, items: &[&ItemView], grab: bool, pkg_pw: &std::collections::HashMap<String, Vec<String>>) {
        let mut groups: BTreeMap<(usize, String), Vec<&ItemView>> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for it in items {
            if !order.contains(&it.pkg) {
                order.push(it.pkg.clone());
            }
            let pos = order.iter().position(|p| p == &it.pkg).unwrap_or(0);
            groups.entry((pos, it.pkg.clone())).or_default().push(it);
        }
        for ((_, pkg), list) in groups {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let all = list.iter().all(|i| self.sel.contains(&i.id));
                    let mut on = all;
                    if ui.checkbox(&mut on, "").changed() {
                        for i in &list {
                            if on {
                                self.sel.insert(i.id.clone());
                            } else {
                                self.sel.remove(&i.id);
                            }
                        }
                    }
                    let name = if pkg.is_empty() { t("Ohne Paket").to_string() } else { pkg.clone() };
                    ui.label(RichText::new(name).strong());
                    let total: u64 = list.iter().filter_map(|i| i.size).sum();
                    let done: u64 = list.iter().map(|i| i.done).sum();
                    let unknown = list.iter().any(|i| i.size.is_none());
                    let files = if list.len() == 1 { t("1 Datei").to_string() } else { tf("{n} Dateien", &[("n", &list.len())]) };
                    let sizes = if grab { size(Some(total)) } else { format!("{} / {}", size(Some(done)), size(Some(total))) };
                    ui.weak(format!("{} · {}{}", files, sizes, if unknown { " +?" } else { "" }));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let n = pkg_pw.get(&pkg).map(|v| v.len()).unwrap_or(0);
                        let txt = if n > 0 { format!("🔑 {}", n) } else { "🔑".to_string() };
                        if ui.small_button(txt).on_hover_text(t("Passwörter für dieses Paket")).clicked() {
                            let cur = pkg_pw.get(&pkg).map(|v| v.join("\n")).unwrap_or_default();
                            self.pkg_edit = Some((pkg.clone(), cur));
                        }
                    });
                });
                ui.separator();
                for it in list {
                    self.row(ui, it, grab);
                }
            });
            ui.add_space(6.0);
        }
    }

    fn row(&mut self, ui: &mut egui::Ui, it: &ItemView, grab: bool) {
        let pct = if it.status == "done" { 1.0 } else { it.size.map(|s| if s > 0 { it.done as f32 / s as f32 } else { 0.0 }).unwrap_or(0.0).min(1.0) };
        let mut label = match it.status.as_str() {
            "checking" => t("Prüfe…").to_string(),
            "ready" => t("Bereit").to_string(),
            "offline" => t("Nicht erreichbar").to_string(),
            "queued" => t("Wartet").to_string(),
            "running" => t("Lädt").to_string(),
            "paused" => t("Pausiert").to_string(),
            "done" => t("Fertig").to_string(),
            "error" => t("Fehler").to_string(),
            other => other.to_string(),
        };
        let mut color = match it.status.as_str() {
            "done" | "ready" => green(),
            "error" | "offline" => red(),
            "running" => blue(),
            _ => ui.visuals().text_color(),
        };
        let mut err = it.error.clone();
        if it.status == "running" {
            if it.conns > 1 {
                label = format!("{} ({}×)", label, it.conns);
            }
            if it.size.is_some() {
                label = format!("{} % · {}", (pct * 100.0).floor(), label);
            }
        }
        if !grab && it.status == "done" {
            match it.extract.as_str() {
                "wait" => label = t("Fertig · entpacken folgt").to_string(),
                "running" => {
                    label = t("Entpacke…").to_string();
                    color = blue();
                }
                "ok" => label = t("Fertig · entpackt").to_string(),
                "error" => {
                    label = t("Entpacken fehlgeschlagen").to_string();
                    color = red();
                    err = Some(it.extract_msg.clone());
                }
                _ => {}
            }
            match it.sha_state.as_str() {
                "running" => {
                    label = t("Prüfe Prüfsumme…").to_string();
                    color = blue();
                }
                "bad" => {
                    label = t("Prüfsumme falsch").to_string();
                    color = red();
                    err = Some(t("Die Datei stimmt nicht mit der erwarteten SHA-256-Prüfsumme überein. Details: Klick auf den Namen.").to_string());
                }
                "error" => {
                    label = t("Prüfsumme nicht berechnet").to_string();
                    color = red();
                    err = Some(it.sha_got.clone());
                }
                "ok" if it.extract.is_empty() => label = t("Fertig · Prüfsumme ok").to_string(),
                _ => {}
            }
        }
        ui.horizontal(|ui| {
            let mut on = self.sel.contains(&it.id);
            if ui.checkbox(&mut on, "").changed() {
                if on {
                    self.sel.insert(it.id.clone());
                } else {
                    self.sel.remove(&it.id);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // rechts beginnend: Knöpfe, Status, Restzeit, Tempo, Grösse
                if ui.small_button("🗑").on_hover_text(t("Entfernen")).clicked() {
                    self.act("remove", vec![it.id.clone()]);
                }
                if grab {
                    if it.status == "ready" && ui.small_button("⬇").on_hover_text(t("Herunterladen")).clicked() {
                        self.act("confirm", vec![it.id.clone()]);
                    }
                    if it.status == "offline" && ui.small_button("↻").on_hover_text(t("Erneut prüfen")).clicked() {
                        self.act("recheck", vec![it.id.clone()]);
                    }
                } else {
                    if (it.status == "running" || it.status == "queued") && ui.small_button("⏸").on_hover_text(t("Pausieren")).clicked() {
                        self.act("pause", vec![it.id.clone()]);
                    }
                    if it.status == "paused" && ui.small_button("▶").on_hover_text(t("Fortsetzen")).clicked() {
                        self.act("resume", vec![it.id.clone()]);
                    }
                    if it.status == "error" && ui.small_button("↻").on_hover_text(t("Erneut versuchen")).clicked() {
                        self.act("resume", vec![it.id.clone()]);
                    }
                    let can_extract = it.status == "done" && it.archive && it.extract != "wait" && it.extract != "running";
                    if can_extract && ui.small_button("📦").on_hover_text(t("Entpacken")).clicked() {
                        self.act("extract", vec![it.id.clone()]);
                        self.say(t("Wird entpackt, sobald alle Teile fertig sind."));
                    }
                }
                ui.add_sized([150.0, 18.0], egui::Label::new(RichText::new(label).color(color)).truncate());
                if !grab {
                    let e = if it.status == "running" && it.speed > 0 { it.size.map(|s| eta(s.saturating_sub(it.done) as f64 / it.speed as f64)).unwrap_or_default() } else { String::new() };
                    ui.add_sized([70.0, 18.0], egui::Label::new(RichText::new(e).weak()));
                    let sp = if it.status == "running" { format!("{}/s", size(Some(it.speed))) } else { String::new() };
                    ui.add_sized([80.0, 18.0], egui::Label::new(RichText::new(sp).monospace()));
                }
                let sz = if grab || it.status == "done" { size(it.size) } else { format!("{} / {}", size(Some(it.done)), size(it.size)) };
                ui.add_sized([150.0, 18.0], egui::Label::new(RichText::new(sz).monospace()));
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    let name = ui.add(egui::Label::new(it.name.as_str()).truncate().sense(egui::Sense::click())).on_hover_text(t("Klicken für Details"));
                    if name.clicked() {
                        self.details = Some((it.id.clone(), it.sha256.clone()));
                    }
                    if let Some(e) = err.filter(|e| !e.is_empty()) {
                        ui.add(egui::Label::new(RichText::new(t_dyn(&e)).small().color(red())).truncate());
                    } else if grab {
                        ui.add(egui::Label::new(RichText::new(it.url.as_str()).small().weak()).truncate());
                    } else {
                        let bar = egui::ProgressBar::new(pct).desired_width(ui.available_width()).fill(if it.status == "done" { green() } else { blue() });
                        ui.add(bar);
                    }
                });
            });
        });
    }

    /* ----- Einstellungen ----- */
    fn settings_window(&mut self, ctx: &egui::Context, s: &Snapshot) {
        let mut open = self.show_settings;
        let mut close = false;
        egui::Window::new(t("Einstellungen"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(620.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                    ui.label(t("Download-Ordner"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.form.dir).desired_width(ui.available_width() - 90.0));
                        if ui.button(t("Wählen …")).clicked() {
                            if let Some(d) = rfd::FileDialog::new().set_directory(&self.form.dir).pick_folder() {
                                self.form.dir = d.to_string_lossy().to_string();
                            }
                        }
                    });
                    ui.add_space(6.0);
                    egui::Grid::new("zahlen").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                        ui.label(t("Gleichzeitige Downloads"));
                        ui.add(egui::DragValue::new(&mut self.form.max_parallel).range(1..=10));
                        ui.end_row();
                        ui.label(t("Verbindungen pro Datei"));
                        ui.add(egui::DragValue::new(&mut self.form.chunks).range(1..=8));
                        ui.end_row();
                        ui.label(t("Tempolimit in KB/s (0 = aus)"));
                        ui.add(egui::DragValue::new(&mut self.form.limit_kb).range(0..=10_000_000).speed(50.0));
                        ui.end_row();
                        ui.label(t("Wiederholungen bei Fehler"));
                        ui.add(egui::DragValue::new(&mut self.form.retries).range(0..=10));
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(t("Proxy"));
                        ui.add(egui::TextEdit::singleline(&mut self.form.proxy).desired_width(280.0).hint_text("http://host:port, socks5://host:port"));
                    });
                    ui.weak(t("Leer: Proxy aus der Umgebung (HTTPS_PROXY, ALL_PROXY), sonst direkt."));
                    ui.add_space(6.0);
                    ui.checkbox(&mut self.form.clipboard, t("Zwischenablage beobachten (kopierte Datei- und Hoster-Links landen im Linksammler)"));
                    ui.checkbox(&mut self.form.clicknload, t("Click'n'Load annehmen (Links von Webseiten mit Click'n'Load-Knopf)"));
                    if s.settings.clicknload && !s.cnl.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(28.0);
                            ui.weak(format!("{}: {}", t("Status"), t_dyn(&s.cnl)));
                        });
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.label(RichText::new(t("Entpacken")).strong().color(accent()));
                    ui.checkbox(&mut self.form.extract, t("Archive nach dem Download automatisch entpacken"));
                    ui.checkbox(&mut self.form.delete_archives, t("Archivdateien nach erfolgreichem Entpacken löschen"));
                    ui.label(t("Passwörter für Archive (eines pro Zeile, werden der Reihe nach probiert)"));
                    ui.add(egui::TextEdit::multiline(&mut self.form.passwords).desired_rows(3).desired_width(f32::INFINITY));
                    let ex = self.extractor_label();
                    if ex.is_empty() {
                        ui.colored_label(red(), t("Kein Entpacker gefunden – bitte 7-Zip installieren (7-zip.org)."));
                    } else {
                        ui.weak(format!("{}: {}", t("Entpacker"), t_dyn(&ex)));
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.label(RichText::new(t("Konten")).strong().color(accent()));
                    if s.accounts.is_empty() {
                        ui.weak(t("Noch kein Konto eingetragen."));
                    }
                    for a in &s.accounts {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(format!("{} · {}", a.host, a.user)).strong());
                                    let (txt, col) = match a.ok {
                                        Some(true) => (t_dyn(&a.info), green()),
                                        Some(false) => (t_dyn(&a.info), red()),
                                        None => (t("Noch nicht geprüft").to_string(), ui.visuals().weak_text_color()),
                                    };
                                    ui.label(RichText::new(txt).small().color(col));
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.small_button("🗑").on_hover_text(t("Konto entfernen")).clicked() {
                                        crate::account_remove(&a.host);
                                    }
                                    let busy = self.acc_busy.load(Ordering::SeqCst);
                                    if ui.add_enabled(!busy, egui::Button::new(t("Prüfen")).small()).clicked() {
                                        let b = self.acc_busy.clone();
                                        b.store(true, Ordering::SeqCst);
                                        std::thread::spawn(move || {
                                            let _ = crate::rg_token(true);
                                            b.store(false, Ordering::SeqCst);
                                        });
                                    }
                                });
                            });
                        });
                    }
                    ui.add_space(4.0);
                    egui::Grid::new("konto").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                        ui.label(t("Hoster"));
                        ui.horizontal(|ui| {
                            for h in crate::HOSTERS {
                                ui.radio_value(&mut self.acc_host, h.to_string(), *h);
                            }
                        });
                        ui.end_row();
                        ui.label(t("Benutzername / E-Mail"));
                        ui.add(egui::TextEdit::singleline(&mut self.acc_user).desired_width(260.0));
                        ui.end_row();
                        ui.label(t("Passwort"));
                        ui.add(egui::TextEdit::singleline(&mut self.acc_pw).password(true).desired_width(260.0));
                        ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        let busy = self.acc_busy.load(Ordering::SeqCst);
                        if ui.add_enabled(!busy, egui::Button::new(t("Konto speichern und prüfen"))).clicked() {
                            let (h, u, p) = (self.acc_host.clone(), self.acc_user.clone(), std::mem::take(&mut self.acc_pw));
                            let (b, msgs) = (self.acc_busy.clone(), self.msgs.clone());
                            b.store(true, Ordering::SeqCst);
                            std::thread::spawn(move || {
                                let m = match crate::account_save(&h, &u, &p) {
                                    Ok(()) => t("Konto gespeichert.").to_string(),
                                    Err(e) => t_dyn(&e),
                                };
                                if let Ok(mut q) = msgs.lock() {
                                    q.push(m);
                                }
                                b.store(false, Ordering::SeqCst);
                            });
                            self.acc_user.clear();
                        }
                        if busy {
                            ui.spinner();
                        }
                    });
                    ui.weak(t("Das Passwort wird verschlüsselt auf diesem Computer abgelegt und nur an den Hoster gesendet."));
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let primary = egui::Button::new(RichText::new(t("Speichern")).color(Color32::WHITE).strong()).fill(accent_fill());
                        if ui.add(primary).clicked() && self.save_settings() {
                            close = true;
                        }
                        if ui.button(t("Schliessen")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        self.show_settings = open && !close;
    }
}

/// Meldungen aus dem Kern übersetzen, soweit sie in en.tsv stehen (sonst unverändert).
/// Eigene Tabelle, weil `sprache::t` nur feste Texte annimmt und sprache.rs in allen
/// Programmen gleich bleiben soll.
fn t_dyn(s: &str) -> String {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    if !crate::sprache::is_en() {
        return s.to_string();
    }
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let un = |x: &str| x.replace("\\n", "\n").replace("\\t", "\t");
        include_str!("en.tsv")
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .filter(|(_, en)| !en.trim().is_empty())
            .map(|(de, en)| (un(de), un(en.trim_end_matches('\r'))))
            .collect()
    });
    if let Some(e) = table.get(s) {
        return e.clone();
    }
    // «Rapidgator: …», «Premium, noch 12 Tage, …» usw.: bekannte Teile einzeln ersetzen
    let mut out = s.to_string();
    for (de, en) in [
        ("Angemeldet, aber kein Premium", "Signed in, but no premium"),
        (" Tage", " days"),
        (", noch ", ", "),
        (" GB Volumen übrig", " GB traffic left"),
        ("bereit (Port ", "ready (port "),
        (" ist belegt – läuft JDownloader noch? Ladewerk versucht es weiter.", " is in use – is JDownloader still running? Ladewerk keeps trying."),
        ("Port ", "Port "),
        ("Rapidgator meldet Fehler ", "Rapidgator reports error "),
    ] {
        out = out.replace(de, en);
    }
    out
}

impl Gui {
    fn details_window(&mut self, ctx: &egui::Context, s: &Snapshot) {
        let Some((id, _)) = self.details.clone() else { return };
        let Some(it) = s.items.iter().find(|i| i.id == id) else {
            self.details = None;
            return;
        };
        let mut open = true;
        let mut msg: Option<String> = None;
        egui::Window::new(t("Details"))
            .open(&mut open)
            .collapsible(false)
            .default_width(620.0)
            .show(ctx, |ui| {
                egui::Grid::new("details").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(t("Name"));
                    ui.add(egui::Label::new(it.name.as_str()).selectable(true));
                    ui.end_row();
                    ui.label(t("Link"));
                    ui.add(egui::Label::new(it.url.as_str()).selectable(true));
                    ui.end_row();
                    if !it.dir.is_empty() {
                        ui.label(t("Datei"));
                        ui.add(egui::Label::new(std::path::Path::new(&it.dir).join(&it.name).to_string_lossy().to_string()).selectable(true));
                        ui.end_row();
                    }
                    ui.label(t("Paket"));
                    ui.label(if it.pkg.is_empty() { t("Ohne Paket").to_string() } else { it.pkg.clone() });
                    ui.end_row();
                    if let Some(e) = it.error.as_deref().filter(|e| !e.is_empty()) {
                        ui.label(t("Meldung"));
                        ui.colored_label(red(), t_dyn(e));
                        ui.end_row();
                    }
                    ui.label(t("SHA-256 erwartet"));
                    ui.horizontal(|ui| {
                        if let Some((_, h)) = self.details.as_mut() {
                            ui.add(egui::TextEdit::singleline(h).desired_width(430.0).font(egui::TextStyle::Monospace).hint_text(t("optional, 64 Zeichen")));
                            if ui.button(t("Übernehmen")).clicked() {
                                msg = Some(match crate::set_hash(&id, h) {
                                    Ok(()) => t("Prüfsumme übernommen.").to_string(),
                                    Err(e) => t_dyn(&e),
                                });
                            }
                        }
                    });
                    ui.end_row();
                    if !it.sha_got.is_empty() {
                        let (txt, col) = match it.sha_state.as_str() {
                            "ok" => (format!("{}  ✔", it.sha_got), green()),
                            "bad" => (it.sha_got.clone(), red()),
                            "error" => (t_dyn(&it.sha_got), red()),
                            _ => (it.sha_got.clone(), ui.visuals().text_color()),
                        };
                        ui.label(t("SHA-256 der Datei"));
                        ui.add(egui::Label::new(RichText::new(txt).monospace().color(col)).selectable(true));
                        ui.end_row();
                    }
                });
                if it.status == "done" && it.stage == "queue" {
                    ui.add_space(6.0);
                    let busy = it.sha_state == "running";
                    if ui.add_enabled(!busy, egui::Button::new(t("Jetzt prüfen"))).on_hover_text(t("Berechnet die SHA-256-Prüfsumme der Datei")).clicked() {
                        let _ = crate::do_action("verify", &crate::ids_json(&[id.clone()]));
                    }
                }
            });
        if let Some(m) = msg {
            self.say(m);
        }
        if !open {
            self.details = None;
        }
    }

    fn pkg_window(&mut self, ctx: &egui::Context) {
        let Some((pkg, _)) = self.pkg_edit.clone() else { return };
        let mut open = true;
        let mut done = false;
        let title = if pkg.is_empty() { t("Passwörter: Ohne Paket").to_string() } else { format!("{}: {}", t("Passwörter"), pkg) };
        egui::Window::new(title).id(egui::Id::new("paket-pw")).open(&mut open).collapsible(false).default_width(420.0).show(ctx, |ui| {
            ui.label(t("Passwörter für die Archive dieses Pakets, eines pro Zeile. Sie werden vor der allgemeinen Liste probiert."));
            if let Some((_, text)) = self.pkg_edit.as_mut() {
                ui.add(egui::TextEdit::multiline(text).desired_rows(4).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new(t("Speichern")).color(Color32::WHITE).strong()).fill(accent_fill())).clicked() {
                        crate::set_pkg_passwords(&pkg, text);
                        done = true;
                    }
                    if ui.button(t("Schliessen")).clicked() {
                        done = true;
                    }
                });
            }
        });
        if done {
            self.say(t("Gespeichert."));
        }
        if !open || done {
            self.pkg_edit = None;
        }
    }
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(500));
        let s = crate::snapshot();
        if self.tab.is_none() {
            let any_grab = s.items.iter().any(|i| i.stage == "grab");
            let any_dl = s.items.iter().any(|i| i.stage == "queue");
            self.tab = Some(if any_dl && !any_grab { Tab::Downloads } else { Tab::Grab });
        }
        let ids: HashSet<&str> = s.items.iter().map(|i| i.id.as_str()).collect();
        self.sel.retain(|i| ids.contains(i.as_str()));
        let pending: Vec<String> = self.msgs.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
        for m in pending {
            self.say(m);
        }

        // Schliessen, während noch geladen wird: nachfragen
        let busy = s.items.iter().any(|i| i.status == "running");
        if ctx.input(|i| i.viewport().close_requested()) && busy && !self.quit_ok {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ask_quit = true;
        }

        egui::TopBottomPanel::top("kopf").show(ctx, |ui| self.top(ui, &s));
        egui::TopBottomPanel::bottom("fuss").show(ctx, |ui| {
            ui.horizontal(|ui| {
                match &self.toast {
                    Some((m, at)) if at.elapsed() < Duration::from_secs(5) => {
                        ui.label(m);
                    }
                    _ => {
                        let running = s.items.iter().filter(|i| i.status == "running").count();
                        let waiting = s.items.iter().filter(|i| i.status == "queued").count();
                        ui.weak(tf("{a} laufen · {b} warten · Ziel: {d}", &[("a", &running), ("b", &waiting), ("d", &s.settings.dir)]));
                    }
                }
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| match self.tab.unwrap_or(Tab::Grab) {
            Tab::Grab => self.grab_tab(ui, &s),
            Tab::Downloads => self.dl_tab(ui, &s),
        });
        if self.show_settings {
            self.settings_window(ctx, &s);
        }
        self.details_window(ctx, &s);
        self.pkg_window(ctx);

        // Strg+V im Fenster (ohne aktives Textfeld): Text direkt in den Linksammler
        let focused = ctx.memory(|m| m.focused().is_some());
        let pasted = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Paste(p) = e { Some(p.clone()) } else { None }));
        if let Some(p) = pasted.filter(|p| !p.trim().is_empty()) {
            if !focused && !self.show_settings && self.details.is_none() && self.pkg_edit.is_none() {
                self.links = p;
                self.tab = Some(Tab::Grab);
                self.grab(false, ctx);
            }
        }
        if self.ask_quit {
            egui::Window::new(t("Ladewerk beenden?"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(t("Es laufen noch Downloads. Sie werden angehalten und beim nächsten Start fortgesetzt."));
                    ui.horizontal(|ui| {
                        if ui.button(t("Beenden")).clicked() {
                            self.quit_ok = true;
                            self.ask_quit = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button(t("Weiterladen")).clicked() {
                            self.ask_quit = false;
                        }
                    });
                });
        }
    }
}
