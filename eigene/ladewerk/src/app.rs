//! Oberfläche (egui): Linksammler und Downloads als Reiter, Einstellungen als eigenes Fenster.
//! Alle Arbeit macht der Kern in main.rs; hier wird nur angezeigt und weitergereicht.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Align, Color32, Layout, RichText};

use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::thema;
use crate::{ItemView, Snapshot, J};

// Farben aus dem gemeinsamen Stil (Logofarbe, Signalfarben).
fn accent() -> Color32 {
    stil::akzent()
}
fn green() -> Color32 {
    stil::gruen()
}
fn red() -> Color32 {
    stil::rot()
}
fn blue() -> Color32 {
    stil::blau()
}

/// Gerahmte Fläche im Stil der Karten.
fn rahmen() -> egui::Frame {
    egui::Frame::none().fill(thema::bg()).stroke(egui::Stroke::new(1.0, thema::border())).rounding(12.0).inner_margin(egui::Margin::symmetric(12.0, 10.0))
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
        thema::titelleiste(cc, [0x0E, 0x74, 0x90]);
        stil::init(&cc.egui_ctx, [0x0E, 0x74, 0x90]);
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

    /* ----- Kopfleiste ----- */
    fn top(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        stil::titel(ui, "Ladewerk");
        stil::trenner(ui);
        let n_grab = s.items.iter().filter(|i| i.stage == "grab").count();
        let n_dl = s.items.iter().filter(|i| i.stage == "queue").count();
        let l_grab = format!("{} ({})", t("Linksammler"), n_grab);
        let l_dl = format!("{} ({})", t("Downloads"), n_dl);
        let mut tab = self.tab.unwrap_or(Tab::Grab);
        ui.allocate_ui(egui::vec2(300.0, 28.0), |ui| {
            stil::wahl(ui, &mut tab, &[(Tab::Grab, l_grab.as_str()), (Tab::Downloads, l_dl.as_str())]);
        });
        self.tab = Some(tab);
        stil::trenner(ui);
        if stil::knopf(ui, Sym::OrdnerAuf, t("Ordner öffnen")).clicked() {
            crate::open_download_dir();
        }
        match tab {
            Tab::Grab => {
                if stil::leise(ui, true, Sym::Papierkorb, t("Nicht erreichbare entfernen"), "").clicked() {
                    crate::clear_items(false);
                }
            }
            Tab::Downloads => {
                if stil::leise(ui, true, Sym::Auge, t("Fertige ausblenden"), "").clicked() {
                    crate::clear_items(true);
                }
            }
        }
        stil::rechts(ui, |ui| {
            if stil::knopf(ui, Sym::Zahnrad, t("Einstellungen")).clicked() {
                self.open_settings(s);
            }
            if s.speed > 0 {
                ui.label(RichText::new(format!("{}/s", size(Some(s.speed)))).monospace());
            }
        });
    }

    /// IDs der markierten Einträge im aktuellen Reiter.
    fn chosen(&self, s: &Snapshot) -> Vec<String> {
        let stage = if self.tab == Some(Tab::Downloads) { "queue" } else { "grab" };
        s.items.iter().filter(|i| i.stage == stage && self.sel.contains(&i.id)).map(|i| i.id.clone()).collect()
    }

    /* ----- Auswahlleiste ----- */
    fn sel_bar(&mut self, ui: &mut egui::Ui, ids: Vec<String>) {
        if self.tab == Some(Tab::Downloads) {
            if stil::knopf(ui, Sym::Pause, t("Pausieren")).clicked() {
                self.act("pause", ids.clone());
            }
            if stil::knopf(ui, Sym::Play, t("Fortsetzen")).clicked() {
                self.act("resume", ids.clone());
            }
        }
        if stil::gefahr(ui, true, Sym::Papierkorb, t("Entfernen")).clicked() {
            self.act("remove", ids);
        }
        stil::rechts(ui, |ui| {
            if stil::leise(ui, true, Sym::Kreuz, t("Auswahl aufheben"), "").clicked() {
                self.sel.clear();
            }
        });
    }

    /* ----- Fussleiste ----- */
    fn foot(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let tab = self.tab.unwrap_or(Tab::Grab);
        ui.horizontal(|ui| {
            stil::rechts(ui, |ui| {
                if tab == Tab::Grab {
                    let items: Vec<&ItemView> = s.items.iter().filter(|i| i.stage == "grab").collect();
                    let mut ids: Vec<String> = items.iter().filter(|i| self.sel.contains(&i.id)).map(|i| i.id.clone()).collect();
                    let text = if ids.is_empty() { t("Alle bereiten herunterladen") } else { t("Auswahl herunterladen") };
                    if ids.is_empty() {
                        ids = items.iter().filter(|i| i.status == "ready").map(|i| i.id.clone()).collect();
                    }
                    if stil::haupt(ui, !ids.is_empty(), Sym::Download, text).on_hover_text(t("Ohne Auswahl: alle bereiten Links")).clicked() {
                        self.act("confirm", ids);
                        self.tab = Some(Tab::Downloads);
                    }
                }
                let (txt, sym) = if s.active { (t("Läuft – anhalten"), Sym::Pause) } else { (t("Angehalten – starten"), Sym::Play) };
                let r = if tab == Tab::Downloads && !s.active { stil::haupt(ui, true, sym, txt) } else { stil::knopf(ui, sym, txt) };
                if r.clicked() {
                    crate::set_active(!s.active);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    match &self.toast {
                        Some((m, at)) if at.elapsed() < Duration::from_secs(5) => {
                            ui.add(egui::Label::new(RichText::new(m.as_str()).size(13.0).color(thema::text())).truncate());
                        }
                        _ => {
                            let stage = if tab == Tab::Grab { "grab" } else { "queue" };
                            let items: Vec<&ItemView> = s.items.iter().filter(|i| i.stage == stage).collect();
                            if !items.is_empty() {
                                let zahl = if tab == Tab::Grab {
                                    let ready = items.iter().filter(|i| i.status == "ready").count();
                                    tf("{a} von {b} bereit", &[("a", &ready), ("b", &items.len())])
                                } else {
                                    let done = items.iter().filter(|i| i.status == "done").count();
                                    tf("{a} von {b} fertig", &[("a", &done), ("b", &items.len())])
                                };
                                ui.label(RichText::new(zahl).size(13.0).strong().color(thema::text()));
                                ui.label(RichText::new("·").color(thema::muted()));
                            }
                            let running = s.items.iter().filter(|i| i.status == "running").count();
                            let waiting = s.items.iter().filter(|i| i.status == "queued").count();
                            let txt = tf("{a} laufen · {b} warten · Ziel: {d}", &[("a", &running), ("b", &waiting), ("d", &s.settings.dir)]);
                            ui.add(egui::Label::new(RichText::new(txt).size(13.0).color(thema::muted())).truncate());
                        }
                    }
                });
            });
        });
    }

    /* ----- Linksammler ----- */
    fn grab_tab(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let ctx = ui.ctx().clone();
        rahmen().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::TextEdit::multiline(&mut self.links)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .hint_text(t("Links hier einfügen – beliebiger Text, die Adressen werden automatisch herausgesucht.")),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.pkg).desired_width(240.0).hint_text(t("Paketname (optional, wird Unterordner)")));
                let busy = self.grabbing.load(Ordering::SeqCst);
                if stil::knopf_an(ui, !busy, Sym::Suche, t("Links prüfen")).clicked() {
                    self.grab(false, &ctx);
                }
                if stil::leise(ui, !busy, Sym::Link, t("Seiten nach Dateien durchsuchen"), t("Lädt die angegebenen Seiten und sammelt alle darin verlinkten Dateien")).clicked() {
                    self.grab(true, &ctx);
                }
                if busy {
                    ui.spinner();
                }
            });
        });
        ui.add_space(8.0);
        let items: Vec<&ItemView> = s.items.iter().filter(|i| i.stage == "grab").collect();
        if items.is_empty() {
            stil::leer(
                ui,
                Sym::Link,
                t("Linksammler"),
                t("Noch keine Links im Linksammler. Oben einfügen und «Links prüfen» drücken – oder Links kopieren bzw. Click'n'Load auf einer Webseite nutzen."),
            );
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.list(ui, &items, true, &s.pkg_passwords);
        });
    }

    /* ----- Downloads ----- */
    fn dl_tab(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let items: Vec<&ItemView> = s.items.iter().filter(|i| i.stage == "queue").collect();
        if items.is_empty() {
            stil::leer(ui, Sym::Download, t("Downloads"), t("Keine Downloads in der Liste."));
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
            rahmen().show(ui, |ui| {
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
                    stil::symbol(ui, Sym::Ordner, 16.0, accent());
                    ui.label(RichText::new(name).strong());
                    let total: u64 = list.iter().filter_map(|i| i.size).sum();
                    let done: u64 = list.iter().map(|i| i.done).sum();
                    let unknown = list.iter().any(|i| i.size.is_none());
                    let files = if list.len() == 1 { t("1 Datei").to_string() } else { tf("{n} Dateien", &[("n", &list.len())]) };
                    let sizes = if grab { size(Some(total)) } else { format!("{} / {}", size(Some(done)), size(Some(total))) };
                    ui.weak(format!("{} · {}{}", files, sizes, if unknown { " +?" } else { "" }));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let n = pkg_pw.get(&pkg).map(|v| v.len()).unwrap_or(0);
                        let txt = if n > 0 { n.to_string() } else { String::new() };
                        if stil::leise(ui, true, Sym::Schluessel, &txt, t("Passwörter für dieses Paket")).clicked() {
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
            _ => thema::muted(),
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
                if stil::gefahr(ui, true, Sym::Papierkorb, "").on_hover_text(t("Entfernen")).clicked() {
                    self.act("remove", vec![it.id.clone()]);
                }
                if grab {
                    if it.status == "ready" && stil::leise(ui, true, Sym::Download, "", t("Herunterladen")).clicked() {
                        self.act("confirm", vec![it.id.clone()]);
                    }
                    if it.status == "offline" && stil::leise(ui, true, Sym::Aktualisieren, "", t("Erneut prüfen")).clicked() {
                        self.act("recheck", vec![it.id.clone()]);
                    }
                } else {
                    if (it.status == "running" || it.status == "queued") && stil::leise(ui, true, Sym::Pause, "", t("Pausieren")).clicked() {
                        self.act("pause", vec![it.id.clone()]);
                    }
                    if it.status == "paused" && stil::leise(ui, true, Sym::Play, "", t("Fortsetzen")).clicked() {
                        self.act("resume", vec![it.id.clone()]);
                    }
                    if it.status == "error" && stil::leise(ui, true, Sym::Aktualisieren, "", t("Erneut versuchen")).clicked() {
                        self.act("resume", vec![it.id.clone()]);
                    }
                    let can_extract = it.status == "done" && it.archive && it.extract != "wait" && it.extract != "running";
                    if can_extract && stil::leise(ui, true, Sym::Archiv, "", t("Entpacken")).clicked() {
                        self.act("extract", vec![it.id.clone()]);
                        self.say(t("Wird entpackt, sobald alle Teile fertig sind."));
                    }
                }
                stil::marke(ui, &label, color, 150.0);
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
                    ui.spacing_mut().item_spacing.y = 8.0;
                    stil::karte(ui, "ordner", Sym::Ordner, t("Download-Ordner"), true, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.form.dir).desired_width(ui.available_width() - 100.0));
                            if stil::knopf(ui, Sym::OrdnerAuf, t("Wählen …")).clicked() {
                                if let Some(d) = rfd::FileDialog::new().set_directory(&self.form.dir).pick_folder() {
                                    self.form.dir = d.to_string_lossy().to_string();
                                }
                            }
                        });
                    });
                    stil::karte(ui, "verbindung", Sym::Download, t("Downloads"), true, |ui| {
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
                        stil::feld(ui, t("Proxy"), |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.form.proxy).desired_width(f32::INFINITY).hint_text("http://host:port, socks5://host:port"));
                        });
                        stil::hinweis(ui, t("Leer: Proxy aus der Umgebung (HTTPS_PROXY, ALL_PROXY), sonst direkt."));
                    });
                    stil::karte(ui, "links", Sym::Link, t("Links annehmen"), true, |ui| {
                        stil::schalter(ui, &mut self.form.clipboard, t("Zwischenablage beobachten (kopierte Datei- und Hoster-Links landen im Linksammler)"));
                        stil::schalter(ui, &mut self.form.clicknload, t("Click'n'Load annehmen (Links von Webseiten mit Click'n'Load-Knopf)"));
                        if s.settings.clicknload && !s.cnl.is_empty() {
                            stil::hinweis(ui, &format!("{}: {}", t("Status"), t_dyn(&s.cnl)));
                        }
                    });
                    stil::karte(ui, "entpacken", Sym::Archiv, t("Entpacken"), true, |ui| {
                        stil::schalter(ui, &mut self.form.extract, t("Archive nach dem Download automatisch entpacken"));
                        stil::schalter(ui, &mut self.form.delete_archives, t("Archivdateien nach erfolgreichem Entpacken löschen"));
                        stil::feld(ui, t("Passwörter für Archive (eines pro Zeile, werden der Reihe nach probiert)"), |ui| {
                            ui.add(egui::TextEdit::multiline(&mut self.form.passwords).desired_rows(3).desired_width(f32::INFINITY));
                        });
                        let ex = self.extractor_label();
                        if ex.is_empty() {
                            ui.colored_label(red(), t("Kein Entpacker gefunden – bitte 7-Zip installieren (7-zip.org)."));
                        } else {
                            stil::hinweis(ui, &format!("{}: {}", t("Entpacker"), t_dyn(&ex)));
                        }
                    });
                    stil::karte(ui, "konten", Sym::Schluessel, t("Konten"), true, |ui| {
                        if s.accounts.is_empty() {
                            stil::hinweis(ui, t("Noch kein Konto eingetragen."));
                        }
                        for a in &s.accounts {
                            rahmen().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(format!("{} · {}", a.host, a.user)).strong());
                                        let (txt, col) = match a.ok {
                                            Some(true) => (t_dyn(&a.info), green()),
                                            Some(false) => (t_dyn(&a.info), red()),
                                            None => (t("Noch nicht geprüft").to_string(), thema::muted()),
                                        };
                                        ui.label(RichText::new(txt).small().color(col));
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if stil::gefahr(ui, true, Sym::Papierkorb, "").on_hover_text(t("Konto entfernen")).clicked() {
                                            crate::account_remove(&a.host);
                                        }
                                        let busy = self.acc_busy.load(Ordering::SeqCst);
                                        if stil::knopf_an(ui, !busy, Sym::Schild, t("Prüfen")).clicked() {
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
                        stil::feld(ui, t("Hoster"), |ui| {
                            let mut h: &'static str = crate::HOSTERS.iter().copied().find(|x| *x == self.acc_host).or_else(|| crate::HOSTERS.first().copied()).unwrap_or("");
                            let wahl: Vec<(&'static str, &str)> = crate::HOSTERS.iter().map(|x| (*x, *x)).collect();
                            if stil::liste(ui, "hoster", &mut h, &wahl) {
                                self.acc_host = h.to_string();
                            }
                        });
                        stil::feld(ui, t("Benutzername / E-Mail"), |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.acc_user).desired_width(f32::INFINITY));
                        });
                        stil::feld(ui, t("Passwort"), |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.acc_pw).password(true).desired_width(f32::INFINITY));
                        });
                        ui.horizontal(|ui| {
                            let busy = self.acc_busy.load(Ordering::SeqCst);
                            if stil::knopf_an(ui, !busy, Sym::Speichern, t("Konto speichern und prüfen")).clicked() {
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
                        stil::hinweis(ui, t("Das Passwort wird verschlüsselt auf diesem Computer abgelegt und nur an den Hoster gesendet."));
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if stil::haupt(ui, true, Sym::Speichern, t("Speichern")).clicked() && self.save_settings() {
                            close = true;
                        }
                        if stil::knopf_text(ui, true, t("Schliessen")).clicked() {
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
                            if stil::knopf(ui, Sym::Haken, t("Übernehmen")).clicked() {
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
                    if stil::knopf_an(ui, !busy, Sym::Schild, t("Jetzt prüfen")).on_hover_text(t("Berechnet die SHA-256-Prüfsumme der Datei")).clicked() {
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
                    if stil::haupt(ui, true, Sym::Speichern, t("Speichern")).clicked() {
                        crate::set_pkg_passwords(&pkg, text);
                        done = true;
                    }
                    if stil::knopf_text(ui, true, t("Schliessen")).clicked() {
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

        stil::kopfleiste(ctx, |ui| self.top(ui, &s));
        let gewaehlt = self.chosen(&s);
        if !gewaehlt.is_empty() {
            let n = gewaehlt.len();
            stil::auswahlleiste(ctx, &tf("{n} gewählt", &[("n", &n)]), |ui| self.sel_bar(ui, gewaehlt));
        }
        stil::fussleiste(ctx, |ui| self.foot(ui, &s));
        stil::mitte(ctx, |ui| match self.tab.unwrap_or(Tab::Grab) {
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
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if stil::haupt(ui, true, Sym::Play, t("Weiterladen")).clicked() {
                            self.ask_quit = false;
                        }
                        if stil::gefahr(ui, true, Sym::Kreuz, t("Beenden")).clicked() {
                            self.quit_ok = true;
                            self.ask_quit = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
        }
    }
}
