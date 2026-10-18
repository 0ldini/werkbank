//! Oberfläche (egui): oben die Verbindungsleiste, in der Mitte links die lokalen und rechts die
//! Server-Dateien, unten Warteschlange und Protokoll. Der Motor (xfer.rs) läuft in eigenen Threads;
//! die Oberfläche fragt seine Meldungen in jedem Bild ab.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align2, Color32, FontId, RichText, Sense};

use crate::model::{human_eta, human_size, human_speed, fmt_time, rjoin, rnormalize, rparent, sort_entries, Entry, Proto, RErr, SortCol, Site};
use crate::sprache::{t, tf};
use crate::store::{self, Overwrite, Settings, Trust};
use crate::xfer::{self, Cmd, Ev, Session};

pub const ACCENT: Color32 = Color32::from_rgb(0x2F, 0x7F, 0xD0);
pub const ERR: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);
pub const OK: Color32 = Color32::from_rgb(0x1F, 0x9D, 0x55);
pub const WARN: Color32 = Color32::from_rgb(0xD9, 0x77, 0x06);

// ---------------------------------------------------------------- Bausteine

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnState {
    Idle,
    Connecting,
    Connected,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Queue,
    Log,
}

/// Eine Fensterhälfte (lokal oder Server).
pub struct Pane {
    /// Aktueller Ordner (lokal: Betriebssystempfad, Server: Pfad mit «/»).
    pub cur: String,
    pub path_text: String,
    pub all: Vec<Entry>,
    pub view: Vec<Entry>,
    pub sel: BTreeSet<String>,
    pub anchor: Option<usize>,
    pub sort: (SortCol, bool),
    pub loading: bool,
    pub rect: Option<egui::Rect>,
}

impl Pane {
    fn new() -> Pane {
        Pane { cur: String::new(), path_text: String::new(), all: Vec::new(), view: Vec::new(), sel: BTreeSet::new(), anchor: None, sort: (SortCol::Name, true), loading: false, rect: None }
    }

    pub fn rebuild(&mut self, hidden: bool) {
        self.view = self.all.iter().filter(|e| hidden || !e.name.starts_with('.')).cloned().collect();
        sort_entries(&mut self.view, self.sort.0, self.sort.1);
        let names: HashSet<&str> = self.view.iter().map(|e| e.name.as_str()).collect();
        self.sel.retain(|n| names.contains(n.as_str()));
        self.anchor = None;
    }

    pub fn selected(&self) -> Vec<Entry> {
        self.view.iter().filter(|e| self.sel.contains(&e.name)).cloned().collect()
    }

    fn summary(&self) -> String {
        let dirs = self.view.iter().filter(|e| e.is_dir).count();
        let files = self.view.len() - dirs;
        let bytes: u64 = self.view.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
        let mut s = tf("{d} Ordner, {f} Dateien ({size})", &[("d", &dirs), ("f", &files), ("size", &human_size(bytes))]);
        if !self.sel.is_empty() {
            s.push_str(&format!(" – {}", tf("{n} ausgewählt", &[("n", &self.sel.len())])));
        }
        s
    }
}

/// Was die Oberfläche in einem Bild ausgelöst hat; wird danach ausgeführt.
pub enum Act {
    Go(bool, String),
    Up(bool),
    Refresh(bool),
    Open(bool, String),
    Transfer(bool, Vec<String>),
    NewFolder(bool),
    Rename(bool, String),
    Delete(bool),
    Chmod,
    Connect,
    Disconnect,
}

pub struct Drag {
    pub from_remote: bool,
    pub names: Vec<String>,
}

pub struct App {
    pub quick: Site,
    pub sites: Vec<Site>,
    pub trust: Arc<Mutex<Trust>>,
    pub settings: Settings,
    pub session: Option<Session>,
    pub state: ConnState,
    pub connected_site: Option<Site>,
    pub local: Pane,
    pub remote: Pane,
    pub focus_remote: bool,
    pub drag: Option<Drag>,
    pub dialog: Option<crate::dialogs::Dialog>,
    pub sm: crate::dialogs::SiteMgr,
    tab: Tab,
    pub log: Vec<String>,
    pub status: String,
    pub status_err: bool,
    samples: HashMap<u64, (Instant, u64, f64)>,
    was_active: bool,
    pub ctx: egui::Context,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        let settings = Settings::load();
        let sites = store::load_sites();
        let mut app = App {
            quick: Site::default(),
            sites,
            trust: Arc::new(Mutex::new(Trust::load())),
            settings,
            session: None,
            state: ConnState::Idle,
            connected_site: None,
            local: Pane::new(),
            remote: Pane::new(),
            focus_remote: false,
            drag: None,
            dialog: None,
            sm: crate::dialogs::SiteMgr::default(),
            tab: Tab::Queue,
            log: Vec::new(),
            status: t("Server eintragen und «Verbinden» drücken – oder unter «Verbindungen» eine gespeicherte wählen.").to_string(),
            status_err: false,
            samples: HashMap::new(),
            was_active: false,
            ctx: cc.egui_ctx.clone(),
        };
        let start = if !app.settings.last_local.is_empty() && Path::new(&app.settings.last_local).is_dir() { PathBuf::from(&app.settings.last_local) } else { store::home_dir() };
        app.load_local(start);
        app
    }

    // ------------------------------------------------ Meldungen

    pub fn say(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_err = false;
    }

    pub fn fail(&mut self, text: impl Into<String>) {
        let s: String = text.into();
        self.log.push(format!("! {s}"));
        self.status = s;
        self.status_err = true;
    }

    // ------------------------------------------------ Lokal

    pub fn load_local(&mut self, path: PathBuf) {
        match read_local(&path, self.settings.show_hidden) {
            Ok(entries) => {
                let p = path.display().to_string();
                self.local.cur = p.clone();
                self.local.path_text = p.clone();
                self.local.all = entries;
                self.local.rebuild(self.settings.show_hidden);
                self.local.sel.clear();
                if self.settings.last_local != p {
                    self.settings.last_local = p;
                    self.settings.save();
                }
            }
            Err(e) => {
                self.local.path_text = self.local.cur.clone();
                self.fail(tf("Ordner «{p}» lässt sich nicht lesen: {e}", &[("p", &path.display()), ("e", &e)]));
            }
        }
    }

    pub fn refresh_local(&mut self) {
        let keep = self.local.sel.clone();
        self.load_local(PathBuf::from(self.local.cur.clone()));
        self.local.sel = keep;
        let names: HashSet<String> = self.local.view.iter().map(|e| e.name.clone()).collect();
        self.local.sel.retain(|n| names.contains(n));
    }

    // ------------------------------------------------ Server

    pub fn load_remote(&mut self, path: String) {
        if let Some(s) = &self.session {
            if self.state == ConnState::Connected {
                self.remote.loading = true;
                s.send(Cmd::List(path));
            }
        }
    }

    pub fn refresh_remote(&mut self) {
        let p = self.remote.cur.clone();
        self.load_remote(p);
    }

    /// Startet die Verbindung. Fehlt das Passwort, wird zuerst danach gefragt.
    pub fn connect(&mut self, site: Site) {
        if site.host.trim().is_empty() {
            self.fail(t("Bitte einen Server eintragen."));
            return;
        }
        if site.password.is_empty() && site.key_file.trim().is_empty() && !site.user.trim().is_empty() {
            self.dialog = Some(crate::dialogs::Dialog::Password { site: Box::new(site), text: String::new() });
            return;
        }
        self.connect_forced(site);
    }

    /// Verbindet ohne weitere Passwort-Rückfrage.
    pub fn connect_forced(&mut self, site: Site) {
        self.disconnect();
        let ctx = self.ctx.clone();
        let wake: xfer::Wake = Arc::new(move || ctx.request_repaint());
        self.say(tf("Verbinde mit {h} …", &[("h", &site.host)]));
        self.state = ConnState::Connecting;
        self.session = Some(Session::start(site.clone(), self.trust.clone(), self.settings.parallel, self.settings.overwrite, wake));
        self.connected_site = Some(site);
        self.remote = Pane::new();
        self.tab = Tab::Log;
    }

    pub fn disconnect(&mut self) {
        if let Some(s) = self.session.take() {
            s.stop();
        }
        self.state = ConnState::Idle;
        self.remote = Pane::new();
        self.samples.clear();
        self.was_active = false;
    }

    // ------------------------------------------------ Ereignisse des Motors

    fn poll(&mut self) {
        let mut evs: Vec<Ev> = Vec::new();
        let mut lines: Vec<String> = Vec::new();
        if let Some(s) = &self.session {
            evs = s.rx.try_iter().collect();
            lines = s.log_rx.try_iter().collect();
        }
        self.log.extend(lines);
        if self.log.len() > 3000 {
            let cut = self.log.len() - 2000;
            self.log.drain(..cut);
        }
        for ev in evs {
            self.handle(ev);
        }
    }

    fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::Connected { cwd } => {
                self.state = ConnState::Connected;
                let site = self.connected_site.clone().unwrap_or_default();
                let start = if site.remote_dir.trim().is_empty() { cwd } else { site.remote_dir.trim().to_string() };
                self.say(tf("Verbunden mit {h}.", &[("h", &site.host)]));
                if !site.local_dir.trim().is_empty() && Path::new(site.local_dir.trim()).is_dir() {
                    self.load_local(PathBuf::from(site.local_dir.trim()));
                }
                self.tab = Tab::Queue;
                self.load_remote(start);
            }
            Ev::ConnectFailed(err) => {
                let site = self.connected_site.clone().unwrap_or_default();
                if let Some(s) = self.session.take() {
                    s.stop();
                }
                self.state = ConnState::Idle;
                match err {
                    RErr::UntrustedCert { fp, detail, changed } => {
                        self.dialog = Some(crate::dialogs::Dialog::Trust { site: Box::new(site), kind: "tls", algo: String::from("TLS"), fp, detail, changed });
                    }
                    RErr::HostKey { algo, fp, changed } => {
                        self.dialog = Some(crate::dialogs::Dialog::Trust { site: Box::new(site), kind: "ssh", algo, fp, detail: String::new(), changed });
                    }
                    e => self.fail(tf("Verbindung fehlgeschlagen: {e}", &[("e", &e)])),
                }
            }
            Ev::Listing { path, entries } => {
                self.remote.cur = path.clone();
                self.remote.path_text = path;
                self.remote.all = entries;
                self.remote.rebuild(self.settings.show_hidden);
                self.remote.loading = false;
            }
            Ev::ListFailed { path, err } => {
                self.remote.loading = false;
                self.remote.path_text = self.remote.cur.clone();
                self.fail(tf("Ordner «{p}»: {e}", &[("p", &path), ("e", &err)]));
            }
            Ev::Changed(msg) => {
                if !msg.is_empty() {
                    self.say(msg);
                }
                self.refresh_remote();
            }
            Ev::Failed { what, err } => self.fail(format!("{what}: {err}")),
            Ev::Queued(n) => {
                if n > 0 {
                    self.say(tf("{n} Übertragungen eingereiht.", &[("n", &n)]));
                    self.tab = Tab::Queue;
                } else {
                    self.say(t("Nichts zu übertragen."));
                }
            }
            Ev::Lost(msg) => {
                self.fail(tf("Verbindung verloren: {e}", &[("e", &msg)]));
            }
            Ev::Busy(msg) => self.say(msg),
        }
    }

    // ------------------------------------------------ Aktionen

    fn apply(&mut self, act: Act) {
        match act {
            Act::Go(remote, path) => {
                if remote {
                    let p = rnormalize(path.trim());
                    self.load_remote(p);
                } else {
                    let p = PathBuf::from(path.trim());
                    self.load_local(p);
                }
            }
            Act::Up(remote) => {
                if remote {
                    let p = rparent(&self.remote.cur);
                    self.load_remote(p);
                } else if let Some(parent) = Path::new(&self.local.cur).parent() {
                    let p = parent.to_path_buf();
                    self.load_local(p);
                }
            }
            Act::Refresh(remote) => {
                if remote {
                    self.refresh_remote();
                } else {
                    self.refresh_local();
                }
            }
            Act::Open(remote, name) => {
                if remote {
                    let Some(e) = self.remote.view.iter().find(|e| e.name == name).cloned() else { return };
                    if e.is_dir {
                        let p = rjoin(&self.remote.cur, &name);
                        self.load_remote(p);
                    } else {
                        self.apply(Act::Transfer(true, vec![name]));
                    }
                } else {
                    let Some(e) = self.local.view.iter().find(|e| e.name == name).cloned() else { return };
                    if e.is_dir {
                        let p = Path::new(&self.local.cur).join(&name);
                        self.load_local(p);
                    } else {
                        self.apply(Act::Transfer(false, vec![name]));
                    }
                }
            }
            Act::Transfer(from_remote, names) => self.transfer(from_remote, names),
            Act::NewFolder(remote) => {
                if remote && self.state != ConnState::Connected {
                    return;
                }
                self.dialog = Some(crate::dialogs::Dialog::NewFolder { remote, name: String::new() });
            }
            Act::Rename(remote, name) => {
                self.dialog = Some(crate::dialogs::Dialog::Rename { remote, old: name.clone(), name });
            }
            Act::Delete(remote) => {
                let pane = if remote { &self.remote } else { &self.local };
                let items = pane.selected();
                if items.is_empty() {
                    return;
                }
                self.dialog = Some(crate::dialogs::Dialog::Delete { remote, items: items.into_iter().map(|e| (e.name, e.is_dir)).collect() });
            }
            Act::Chmod => {
                let items = self.remote.selected();
                if items.is_empty() {
                    return;
                }
                let mode = items[0].mode.unwrap_or(if items[0].is_dir { 0o755 } else { 0o644 });
                self.dialog = Some(crate::dialogs::Dialog::Chmod { items: items.into_iter().map(|e| (e.name, e.is_dir)).collect(), mode, text: format!("{mode:o}"), recursive: false });
            }
            Act::Connect => {
                let site = self.quick.clone();
                self.connect(site);
            }
            Act::Disconnect => {
                self.disconnect();
                self.say(t("Getrennt."));
            }
        }
    }

    /// Dateien übertragen: `from_remote` = vom Server herunterladen, sonst hochladen.
    pub fn transfer(&mut self, from_remote: bool, names: Vec<String>) {
        if self.state != ConnState::Connected {
            self.fail(t("Erst mit einem Server verbinden."));
            return;
        }
        let Some(s) = &self.session else { return };
        if from_remote {
            let items: Vec<(String, bool, u64)> = self
                .remote
                .view
                .iter()
                .filter(|e| names.contains(&e.name))
                .map(|e| (rjoin(&self.remote.cur, &e.name), e.is_dir, e.size))
                .collect();
            if !items.is_empty() {
                s.send(Cmd::PlanDownload { items, local_dir: PathBuf::from(&self.local.cur) });
            }
        } else {
            let items: Vec<PathBuf> = self.local.view.iter().filter(|e| names.contains(&e.name)).map(|e| Path::new(&self.local.cur).join(&e.name)).collect();
            if !items.is_empty() {
                s.send(Cmd::PlanUpload { items, remote_dir: self.remote.cur.clone() });
            }
        }
    }

    /// Von aussen ins Fenster gezogene Dateien hochladen.
    fn upload_paths(&mut self, paths: Vec<PathBuf>) {
        if self.state != ConnState::Connected {
            self.fail(t("Erst mit einem Server verbinden, dann Dateien hierher ziehen."));
            return;
        }
        if let Some(s) = &self.session {
            s.send(Cmd::PlanUpload { items: paths, remote_dir: self.remote.cur.clone() });
        }
    }

    // ------------------------------------------------ Zeichnen

    fn top_bar(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        egui::TopBottomPanel::top("conn").show(ctx, |ui| {
            ui.add_space(4.0);
            let idle = self.state == ConnState::Idle;
            ui.horizontal_wrapped(|ui| {
                ui.heading(RichText::new("FTPwerk").color(ACCENT));
                ui.separator();
                if ui.button(t("📇 Verbindungen …")).clicked() {
                    self.sm = crate::dialogs::SiteMgr::open(&self.sites, &self.quick);
                    self.dialog = Some(crate::dialogs::Dialog::Sites);
                }
                ui.separator();
                ui.add_enabled_ui(idle, |ui| {
                    let before = self.quick.proto;
                    egui::ComboBox::from_id_salt("proto").width(190.0).selected_text(t(self.quick.proto.label())).show_ui(ui, |ui| {
                        for p in Proto::ALL {
                            ui.selectable_value(&mut self.quick.proto, p, t(p.label()));
                        }
                    });
                    if before != self.quick.proto {
                        self.quick.port = self.quick.proto.default_port();
                    }
                    ui.label(t("Server"));
                    ui.add(egui::TextEdit::singleline(&mut self.quick.host).desired_width(170.0).hint_text("ftp.beispiel.ch"));
                    ui.label(t("Port"));
                    ui.add(egui::DragValue::new(&mut self.quick.port).range(1..=65535));
                    ui.label(t("Benutzer"));
                    ui.add(egui::TextEdit::singleline(&mut self.quick.user).desired_width(110.0));
                    ui.label(if self.quick.key_file.trim().is_empty() { t("Passwort") } else { t("Passphrase") });
                    let pw = ui.add(egui::TextEdit::singleline(&mut self.quick.password).password(true).desired_width(110.0));
                    if pw.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        acts.push(Act::Connect);
                    }
                    if self.quick.proto == Proto::Sftp {
                        ui.label(t("Schlüssel"));
                        ui.add(egui::TextEdit::singleline(&mut self.quick.key_file).desired_width(150.0).hint_text(t("(optional)")));
                        if ui.button("…").on_hover_text(t("Schlüsseldatei wählen")).clicked() {
                            if let Some(p) = rfd::FileDialog::new().set_title(t("Private Schlüsseldatei wählen")).set_directory(store::home_dir().join(".ssh")).pick_file() {
                                self.quick.key_file = p.display().to_string();
                            }
                        }
                    }
                });
                match self.state {
                    ConnState::Idle => {
                        if ui.button(RichText::new(t("Verbinden")).strong()).clicked() {
                            acts.push(Act::Connect);
                        }
                    }
                    ConnState::Connecting => {
                        ui.add_enabled(false, egui::Button::new(t("Verbinde …")));
                        if ui.button(t("Abbrechen")).clicked() {
                            acts.push(Act::Disconnect);
                        }
                    }
                    ConnState::Connected => {
                        if ui.button(t("Trennen")).clicked() {
                            acts.push(Act::Disconnect);
                        }
                        if ui.button("⚙").on_hover_text(t("Einstellungen")).clicked() {
                            self.dialog = Some(crate::dialogs::Dialog::Settings);
                        }
                    }
                }
                if idle && ui.button("⚙").on_hover_text(t("Einstellungen")).clicked() {
                    self.dialog = Some(crate::dialogs::Dialog::Settings);
                }
            });
            ui.add_space(4.0);
        });
    }

    fn bottom_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let col = if self.status_err { ERR } else { ui.visuals().text_color() };
                ui.label(RichText::new(&self.status).color(col));
                if let Some(s) = &self.session {
                    let (w, r, d, f) = s.queue.counts();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut txt = tf("Wartend {w} · Läuft {r} · Fertig {d}", &[("w", &w), ("r", &r), ("d", &d)]);
                        if f > 0 {
                            txt.push_str(&format!(" · {}", tf("Fehler {f}", &[("f", &f)])));
                        }
                        ui.label(RichText::new(txt).weak());
                    });
                }
            });
        });
    }

    fn queue_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("queue").resizable(true).default_height(190.0).min_height(90.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Queue, t("Warteschlange"));
                ui.selectable_value(&mut self.tab, Tab::Log, t("Protokoll"));
                ui.separator();
                if self.tab == Tab::Queue {
                    if let Some(s) = &self.session {
                        let paused = s.queue.paused.load(std::sync::atomic::Ordering::Relaxed);
                        if ui.button(if paused { t("▶ Fortsetzen") } else { t("⏸ Pause") }).clicked() {
                            s.queue.set_paused(!paused);
                        }
                        if ui.button(t("Alle abbrechen")).clicked() {
                            s.queue.cancel_all();
                        }
                        if ui.button(t("Fehlgeschlagene wiederholen")).clicked() {
                            s.queue.retry(None);
                        }
                        if ui.button(t("Erledigte entfernen")).clicked() {
                            s.queue.clear_finished();
                        }
                        ui.separator();
                        ui.label(t("Wenn die Datei schon existiert:"));
                        let mut ow = s.queue.overwrite();
                        let before = ow;
                        egui::ComboBox::from_id_salt("ow").selected_text(ow_label(ow)).show_ui(ui, |ui| {
                            for o in [Overwrite::Resume, Overwrite::Overwrite, Overwrite::Skip] {
                                ui.selectable_value(&mut ow, o, ow_label(o));
                            }
                        });
                        if ow != before {
                            s.queue.set_overwrite(ow);
                            self.settings.overwrite = ow;
                            self.settings.save();
                        }
                    }
                } else if ui.button(t("Leeren")).clicked() {
                    self.log.clear();
                }
            });
            ui.separator();
            match self.tab {
                Tab::Log => {
                    egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                        for l in &self.log {
                            let col = if l.starts_with('!') { ERR } else if l.starts_with('>') { ui.visuals().weak_text_color() } else { ui.visuals().text_color() };
                            ui.label(RichText::new(l).monospace().size(12.0).color(col));
                        }
                    });
                }
                Tab::Queue => self.queue_list(ui),
            }
        });
    }

    fn queue_list(&mut self, ui: &mut egui::Ui) {
        let Some(s) = &self.session else {
            ui.label(RichText::new(t("Keine Verbindung.")).weak());
            return;
        };
        let jobs = s.queue.snapshot();
        if jobs.is_empty() {
            ui.label(RichText::new(t("Die Warteschlange ist leer. Dateien auf die andere Seite ziehen oder doppelklicken.")).weak());
            return;
        }
        let now = Instant::now();
        for j in &jobs {
            if j.status() == xfer::RUN {
                let d = j.done.load(std::sync::atomic::Ordering::Relaxed);
                let e = self.samples.entry(j.id).or_insert((now, d, 0.0));
                let dt = now.duration_since(e.0).as_secs_f64();
                if dt >= 0.4 {
                    let inst = d.saturating_sub(e.1) as f64 / dt;
                    e.2 = if e.2 <= 0.0 { inst } else { 0.6 * e.2 + 0.4 * inst };
                    e.0 = now;
                    e.1 = d;
                }
            }
        }
        let mut cancel: Option<u64> = None;
        let mut retry: Option<u64> = None;
        let mut remove: Option<u64> = None;
        egui::ScrollArea::vertical().id_salt("queue_list").auto_shrink([false, false]).show(ui, |ui| {
            for j in &jobs {
                let st = j.status();
                ui.horizontal(|ui| {
                    ui.label(if j.up { "⬆" } else { "⬇" });
                    ui.add_sized([ui.available_width().min(260.0).max(120.0), 18.0], egui::Label::new(j.name()).truncate());
                    let total = j.total.load(std::sync::atomic::Ordering::Relaxed);
                    let done = j.done.load(std::sync::atomic::Ordering::Relaxed);
                    let (text, col) = match st {
                        xfer::WAIT => (t("wartet").to_string(), ui.visuals().weak_text_color()),
                        xfer::RUN => {
                            let sp = self.samples.get(&j.id).map(|e| e.2).unwrap_or(0.0);
                            let mut s = format!("{} / {}", human_size(done), if total > 0 { human_size(total) } else { "?".into() });
                            if sp > 0.0 {
                                s.push_str(&format!("  ·  {}", human_speed(sp)));
                                if total > done {
                                    s.push_str(&format!("  ·  {}", tf("noch {t}", &[("t", &human_eta((total - done) as f64 / sp))])));
                                }
                            }
                            (s, ui.visuals().text_color())
                        }
                        xfer::DONE => (format!("{} ({})", t("fertig"), human_size(total.max(done))), OK),
                        xfer::SKIPPED => (t("übersprungen").to_string(), ui.visuals().weak_text_color()),
                        xfer::CANCELLED => (t("abgebrochen").to_string(), WARN),
                        _ => (j.error_text(), ERR),
                    };
                    if st == xfer::RUN || st == xfer::WAIT {
                        ui.add(egui::ProgressBar::new(j.fraction()).desired_width(150.0).fill(ACCENT));
                    }
                    ui.add(egui::Label::new(RichText::new(text).color(col)).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match st {
                        xfer::RUN | xfer::WAIT => {
                            if ui.small_button("✖").on_hover_text(t("Abbrechen")).clicked() {
                                cancel = Some(j.id);
                            }
                        }
                        xfer::FAIL | xfer::CANCELLED => {
                            if ui.small_button("🗑").on_hover_text(t("Aus der Liste entfernen")).clicked() {
                                remove = Some(j.id);
                            }
                            if ui.small_button("🔄").on_hover_text(t("Erneut versuchen")).clicked() {
                                retry = Some(j.id);
                            }
                        }
                        _ => {
                            if ui.small_button("🗑").on_hover_text(t("Aus der Liste entfernen")).clicked() {
                                remove = Some(j.id);
                            }
                        }
                    });
                });
            }
        });
        if let Some(id) = cancel {
            s.queue.cancel(id);
        }
        if let Some(id) = retry {
            s.queue.retry(Some(id));
        }
        if let Some(id) = remove {
            s.queue.remove(id);
        }
    }

    fn panes(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let connected = self.state == ConnState::Connected;
            let dragging = self.drag.is_some();
            let mut started: Option<Drag> = None;
            let mut focus: Option<bool> = None;
            let mut local = std::mem::replace(&mut self.local, Pane::new());
            let mut remote = std::mem::replace(&mut self.remote, Pane::new());
            ui.columns(2, |cols| {
                let (l, r) = cols.split_at_mut(1);
                if pane_ui(&mut l[0], &mut local, false, true, connected, dragging, self.drag.as_ref().map(|d| d.from_remote) == Some(true), acts, &mut started, &mut focus) {
                    focus = Some(false);
                }
                if pane_ui(&mut r[0], &mut remote, true, connected, connected, dragging, self.drag.as_ref().map(|d| d.from_remote) == Some(false), acts, &mut started, &mut focus) {
                    focus = Some(true);
                }
            });
            self.local = local;
            self.remote = remote;
            if let Some(d) = started {
                self.drag = Some(d);
            }
            if let Some(f) = focus {
                self.focus_remote = f;
            }
        });
    }

    /// Ziehen und Ablegen zwischen den Fensterhälften.
    fn drag_overlay(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(d) = &self.drag else { return };
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        let ptr = ctx.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
        let target = if d.from_remote { self.local.rect } else { self.remote.rect };
        if let Some(p) = ptr {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_label")));
            if let Some(r) = target {
                painter.rect_stroke(r.shrink(2.0), 4.0, egui::Stroke::new(2.0, ACCENT));
            }
            let text = if d.names.len() == 1 { d.names[0].clone() } else { tf("{n} Einträge", &[("n", &d.names.len())]) };
            let g = painter.layout_no_wrap(text, FontId::proportional(14.0), Color32::WHITE);
            let pos = p + vec2(14.0, 14.0);
            let rect = egui::Rect::from_min_size(pos, g.size() + vec2(12.0, 6.0));
            painter.rect_filled(rect, 4.0, ACCENT);
            painter.galley(pos + vec2(6.0, 3.0), g, Color32::WHITE);
        }
        if ctx.input(|i| i.pointer.any_released()) {
            let names = d.names.clone();
            let from_remote = d.from_remote;
            if let (Some(p), Some(r)) = (ptr, target) {
                if r.contains(p) {
                    acts.push(Act::Transfer(from_remote, names));
                }
            }
            self.drag = None;
        } else {
            ctx.request_repaint();
        }
    }

    fn keys(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        if self.dialog.is_some() || ctx.wants_keyboard_input() {
            return;
        }
        let remote = self.focus_remote;
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            acts.push(Act::Refresh(false));
            acts.push(Act::Refresh(true));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
            let pane = if remote { &self.remote } else { &self.local };
            if let Some(e) = pane.selected().first() {
                acts.push(Act::Rename(remote, e.name.clone()));
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
            acts.push(Act::Delete(remote));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
            acts.push(Act::Up(remote));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            let pane = if remote { &self.remote } else { &self.local };
            if let Some(e) = pane.selected().first() {
                acts.push(Act::Open(remote, e.name.clone()));
            }
        }
    }
}

fn ow_label(o: Overwrite) -> &'static str {
    match o {
        Overwrite::Resume => t("Fortsetzen / ersetzen"),
        Overwrite::Overwrite => t("Immer überschreiben"),
        Overwrite::Skip => t("Überspringen"),
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();

        // Dateien von aussen ins Fenster ziehen = hochladen
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            self.upload_paths(dropped);
        }

        // Nach dem Ende einer Übertragung beide Listen neu laden
        let active = self.session.as_ref().map(|s| {
            let (w, r, _, _) = s.queue.counts();
            w + r > 0
        });
        let active = active.unwrap_or(false);
        if self.was_active && !active {
            self.refresh_local();
            self.refresh_remote();
        }
        self.was_active = active;
        if active || self.state == ConnState::Connecting {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        let mut acts: Vec<Act> = Vec::new();
        self.top_bar(ctx, &mut acts);
        self.bottom_status(ctx);
        self.queue_panel(ctx);
        self.panes(ctx, &mut acts);
        self.drag_overlay(ctx, &mut acts);
        self.keys(ctx, &mut acts);
        self.show_dialogs(ctx, &mut acts);
        for a in acts {
            self.apply(a);
        }
    }
}

// ---------------------------------------------------------------- Lokale Dateien lesen

pub fn read_local(path: &Path, hidden: bool) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        let Ok(lmd) = ent.metadata() else { continue };
        let is_link = lmd.file_type().is_symlink();
        let md = if is_link { std::fs::metadata(ent.path()).unwrap_or(lmd) } else { lmd };
        if !hidden && (name.starts_with('.') || is_hidden_attr(&md)) {
            continue;
        }
        let mtime = md.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
        out.push(Entry { name, is_dir: md.is_dir(), is_link, size: if md.is_dir() { 0 } else { md.len() }, mtime, perms: String::new(), mode: None });
    }
    Ok(out)
}

#[cfg(windows)]
fn is_hidden_attr(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x2 != 0
}

#[cfg(not(windows))]
fn is_hidden_attr(_md: &std::fs::Metadata) -> bool {
    false
}

/// Laufwerksbuchstaben (nur Windows).
pub fn drives() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    ('A'..='Z').map(|c| format!("{c}:\\")).filter(|d| Path::new(d).exists()).collect()
}

// ---------------------------------------------------------------- Eine Fensterhälfte zeichnen

const ROW_H: f32 = 22.0;

/// Gibt `true` zurück, wenn der Benutzer in dieser Hälfte geklickt hat (dann hat sie den Fokus).
#[allow(clippy::too_many_arguments)]
fn pane_ui(
    ui: &mut egui::Ui,
    p: &mut Pane,
    remote: bool,
    enabled: bool,
    connected: bool,
    dragging: bool,
    _is_source: bool,
    acts: &mut Vec<Act>,
    started: &mut Option<Drag>,
    _focus: &mut Option<bool>,
) -> bool {
    let mut clicked_here = false;
    ui.horizontal(|ui| {
        ui.strong(if remote { t("Server") } else { t("Lokal") });
        if !remote {
            let ds = drives();
            if !ds.is_empty() {
                let cur_drive: String = p.cur.chars().take(3).collect();
                egui::ComboBox::from_id_salt("drives").width(56.0).selected_text(cur_drive).show_ui(ui, |ui| {
                    for d in ds {
                        if ui.selectable_label(false, &d).clicked() {
                            acts.push(Act::Go(false, d));
                        }
                    }
                });
            }
            if ui.button("🏠").on_hover_text(t("Heimordner")).clicked() {
                acts.push(Act::Go(false, store::home_dir().display().to_string()));
            }
        }
        if ui.add_enabled(enabled, egui::Button::new("⬆")).on_hover_text(t("Ordner nach oben")).clicked() {
            acts.push(Act::Up(remote));
        }
        let w = (ui.available_width() - 76.0).max(80.0);
        let te = ui.add_enabled(enabled, egui::TextEdit::singleline(&mut p.path_text).desired_width(w));
        if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            acts.push(Act::Go(remote, p.path_text.clone()));
        }
        if ui.add_enabled(enabled, egui::Button::new("🔄")).on_hover_text(t("Neu laden (F5)")).clicked() {
            acts.push(Act::Refresh(remote));
        }
        if ui.add_enabled(enabled, egui::Button::new("➕📁")).on_hover_text(t("Neuer Ordner")).clicked() {
            acts.push(Act::NewFolder(remote));
        }
    });

    // Spaltenköpfe zum Sortieren
    ui.horizontal(|ui| {
        let arrow = |col: SortCol, p: &Pane| if p.sort.0 == col { if p.sort.1 { " ▲" } else { " ▼" } } else { "" };
        let mut sort_to: Option<SortCol> = None;
        if ui.selectable_label(p.sort.0 == SortCol::Name, format!("{}{}", t("Name"), arrow(SortCol::Name, p))).clicked() {
            sort_to = Some(SortCol::Name);
        }
        if ui.selectable_label(p.sort.0 == SortCol::Size, format!("{}{}", t("Grösse"), arrow(SortCol::Size, p))).clicked() {
            sort_to = Some(SortCol::Size);
        }
        if ui.selectable_label(p.sort.0 == SortCol::Time, format!("{}{}", t("Geändert"), arrow(SortCol::Time, p))).clicked() {
            sort_to = Some(SortCol::Time);
        }
        if let Some(c) = sort_to {
            p.sort = if p.sort.0 == c { (c, !p.sort.1) } else { (c, true) };
            let hidden = true;
            let _ = hidden;
            sort_entries(&mut p.view, p.sort.0, p.sort.1);
        }
        if p.loading {
            ui.spinner();
        }
    });

    let font: FontId = egui::TextStyle::Body.resolve(ui.style());
    let mods = ui.input(|i| i.modifiers);
    let (w_perm, w_date, w_size) = (if remote { 84.0 } else { 0.0 }, 118.0, 74.0);
    let list_height = (ui.available_height() - 24.0).max(60.0);
    let mut select_click: Option<usize> = None;
    let mut ctx_actions: Vec<Act> = Vec::new();
    let can_act = enabled;
    let n = p.view.len();

    let out = egui::ScrollArea::vertical().id_salt(("pane", remote)).max_height(list_height).auto_shrink([false, false]).show_rows(ui, ROW_H, n, |ui, range| {
        for i in range {
            let e = p.view[i].clone();
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
            let selected = p.sel.contains(&e.name);
            if selected {
                ui.painter().rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
            } else if resp.hovered() && !dragging {
                ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
            }
            let col = if selected { ui.visuals().selection.stroke.color } else { ui.visuals().text_color() };
            let weak = ui.visuals().weak_text_color();
            let cy = rect.center().y;
            let painter = ui.painter_at(rect);
            let mut x = rect.right() - 8.0;
            if remote {
                painter.text(pos2(x, cy), Align2::RIGHT_CENTER, &e.perms, FontId::monospace(11.0), weak);
                x -= w_perm;
            }
            if let Some(m) = e.mtime {
                painter.text(pos2(x, cy), Align2::RIGHT_CENTER, fmt_time(m), font.clone(), weak);
            }
            x -= w_date;
            if !e.is_dir {
                painter.text(pos2(x, cy), Align2::RIGHT_CENTER, human_size(e.size), font.clone(), weak);
            }
            x -= w_size;
            let icon = if e.is_dir { "📁" } else if e.is_link { "🔗" } else { "📄" };
            let name_clip = ui.painter().with_clip_rect(egui::Rect::from_min_max(rect.min, pos2((x + 4.0).max(rect.left() + 20.0), rect.max.y)));
            name_clip.text(pos2(rect.left() + 6.0, cy), Align2::LEFT_CENTER, format!("{icon} {}", e.name), font.clone(), col);

            if resp.clicked() {
                select_click = Some(i);
            }
            if resp.secondary_clicked() && !p.sel.contains(&e.name) {
                p.sel.clear();
                p.sel.insert(e.name.clone());
                p.anchor = Some(i);
            }
            if resp.double_clicked() {
                ctx_actions.push(Act::Open(remote, e.name.clone()));
            }
            if resp.drag_started() && can_act && connected {
                if !p.sel.contains(&e.name) {
                    p.sel.clear();
                    p.sel.insert(e.name.clone());
                    p.anchor = Some(i);
                }
                *started = Some(Drag { from_remote: remote, names: p.sel.iter().cloned().collect() });
            }
            let name = e.name.clone();
            resp.context_menu(|ui| {
                let multi = p.sel.len() > 1;
                if connected {
                    let label = if remote { t("⬇ Herunterladen") } else { t("⬆ Hochladen") };
                    if ui.button(label).clicked() {
                        ctx_actions.push(Act::Transfer(remote, p.sel.iter().cloned().collect()));
                        ui.close_menu();
                    }
                }
                if e.is_dir && ui.button(t("Öffnen")).clicked() {
                    ctx_actions.push(Act::Open(remote, name.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if !multi && ui.button(t("Umbenennen (F2)")).clicked() {
                    ctx_actions.push(Act::Rename(remote, name.clone()));
                    ui.close_menu();
                }
                if remote && ui.button(t("Rechte ändern …")).clicked() {
                    ctx_actions.push(Act::Chmod);
                    ui.close_menu();
                }
                if ui.button(t("Löschen (Entf)")).clicked() {
                    ctx_actions.push(Act::Delete(remote));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(t("Neuer Ordner …")).clicked() {
                    ctx_actions.push(Act::NewFolder(remote));
                    ui.close_menu();
                }
                if ui.button(t("Neu laden (F5)")).clicked() {
                    ctx_actions.push(Act::Refresh(remote));
                    ui.close_menu();
                }
            });
        }
    });
    p.rect = Some(out.inner_rect);

    if n == 0 {
        let msg = if remote && !connected { t("Nicht verbunden.") } else { t("Dieser Ordner ist leer.") };
        ui.painter().text(out.inner_rect.center_top() + vec2(0.0, 30.0), Align2::CENTER_CENTER, msg, FontId::proportional(14.0), ui.visuals().weak_text_color());
    }

    // Auswahl: Klick, Strg+Klick, Umschalt+Klick
    if let Some(i) = select_click {
        clicked_here = true;
        let name = p.view[i].name.clone();
        if mods.shift && p.anchor.is_some() {
            let a = p.anchor.unwrap_or(i);
            if !(mods.ctrl || mods.command) {
                p.sel.clear();
            }
            for k in a.min(i)..=a.max(i) {
                if let Some(e) = p.view.get(k) {
                    p.sel.insert(e.name.clone());
                }
            }
        } else if mods.ctrl || mods.command {
            if !p.sel.remove(&name) {
                p.sel.insert(name);
            }
            p.anchor = Some(i);
        } else {
            p.sel.clear();
            p.sel.insert(name);
            p.anchor = Some(i);
        }
    }
    acts.extend(ctx_actions);

    ui.add_space(2.0);
    ui.label(RichText::new(p.summary()).weak().size(12.0));
    clicked_here
}
