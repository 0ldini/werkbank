//! Oberfläche (egui): links die Liste aller Bilder und Videos, rechts die
//! Einstellungen, unten Ausgabedatei, Start und Fortschritt. Das Rendern läuft
//! in einem Hintergrundthread (render.rs); die Oberfläche fragt den Stand ab.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::model::{self, Item, Kind, OutFmt, Quality, Settings, Transition};
use crate::render::{self, Control, Preview};
use crate::sicht;
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::thema;
use crate::tool;

/// Meldungen aus model.rs/render.rs übersetzen. Diese Dateien bleiben ohne Sprachmodul
/// (sie werden für Tests einzeln mit rustc gebaut), darum geschieht es hier bei der Anzeige.
fn engine_msg(s: &str) -> String {
    let whole: [&'static str; 13] = [
        "Die Liste ist leer.",
        "Abgebrochen.",
        "Bildlader ist unerwartet beendet.",
        "Nichts zum Anzeigen.",
        "Keine Schrift gefunden – unter «Vor- und Abspann» eine Schriftdatei (.ttf) wählen",
        "kein Medium",
        "Dateityp nicht unterstützt",
        "Videolänge unbekannt",
        "keine Bild-/Videodaten gefunden",
        "kein ffmpeg-Ausgang",
        "kein ffmpeg-Eingang",
        "Schreibthread abgestürzt",
        "unbekannter Fehler",
    ];
    if let Some(de) = whole.iter().copied().find(|de| *de == s) {
        return t(de).to_string();
    }
    // «Anfang: Rest» – der Rest kann selbst wieder eine Meldung sein.
    let prefixes: [&'static str; 10] = [
        "ffmpeg startet nicht",
        "ffmpeg konnte nicht gestartet werden",
        "ffmpeg (Kodierer) startet nicht",
        "ffmpeg hat beim Kodieren abgebrochen",
        "Als schwarzes Bild eingesetzt",
        "Musik nicht lesbar, ausgelassen",
        "Textdatei",
        "Vorspann",
        "Abspann",
        "Nicht lesbar",
    ];
    for de in prefixes {
        if let Some(rest) = s.strip_prefix(de).and_then(|r| r.strip_prefix(": ")) {
            return format!("{}: {}", t(de), engine_msg(rest));
        }
    }
    if let Some(name) = s.strip_suffix(" lässt sich nicht vollständig lesen (beschädigt?)") {
        return tf("{name} lässt sich nicht vollständig lesen (beschädigt?)", &[("name", &name)]);
    }
    if let Some((name, e)) = s.strip_prefix("Video ").and_then(|r| r.split_once(" nicht lesbar: ")) {
        let e = engine_msg(e);
        return tf("Video {name} nicht lesbar: {e}", &[("name", &name), ("e", &e)]);
    }
    s.to_string()
}

const RES: [(&str, u32, u32); 7] = [
    ("Full HD 1920×1080", 1920, 1080),
    ("4K 3840×2160", 3840, 2160),
    ("HD 1280×720", 1280, 720),
    ("Vorschau 640×360", 640, 360),
    ("Instagram Reel/Story 1080×1920 (hoch)", 1080, 1920),
    ("Instagram Beitrag 1080×1350 (4:5)", 1080, 1350),
    ("Quadrat 1080×1080", 1080, 1080),
];
/// So viele Standbilder behält die Ansicht im Speicher.
const THUMB_CACHE: usize = 30;

/// Index von «Instagram Reel/Story» in RES.
const RES_REEL: usize = 4;
/// Instagram-Reels dürfen höchstens so lang sein (Sekunden).
const REEL_MAX_SECS: f32 = 180.0;

/// Ergebnis des Ausmessens einer Datei, vom Hilfsthread gemeldet.
enum ProbeMsg {
    One(usize, Result<Item, String>),
    Done,
}

struct Probing {
    rx: Receiver<ProbeMsg>,
    pending: Vec<Option<Result<Item, String>>>,
    got: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum PvMode {
    Start,
    FromSel,
    OnlySel,
}

pub struct App {
    items: Vec<Item>,
    /// Gewählte Zeilen (Mehrfachwahl mit Strg/Umschalt).
    sel: BTreeSet<usize>,
    anchor: Option<usize>,
    /// Zeile, an der gerade gezogen wird (Umsortieren per Maus).
    drag: Option<usize>,
    /// Zielnummer für „An Nr. verschieben“ (1-basiert).
    goto: usize,
    preview: Option<Arc<Preview>>,
    pv_open: bool,
    pv_label: String,
    pv_speed: u32,
    pv_tex: Option<egui::TextureHandle>,
    pv_mode: Option<PvMode>,
    s: Settings,
    res: usize,
    /// true: «Diashow erstellen» nimmt nur die gewählten Einträge.
    only_sel: bool,
    ffmpeg: Option<String>,
    out_path: String,
    recursive: bool,
    probing: Option<Probing>,
    job: Option<Arc<Control>>,
    started: Instant,
    status: String,
    status_ok: bool,
    notes: Vec<String>,
    /// Ansicht des gewählten Eintrags unter der Liste.
    show_view: bool,
    /// Geladene Standbilder (Pfad → Bild, Grösse), älteste vorne in `thumb_order`.
    thumbs: HashMap<PathBuf, (egui::TextureHandle, [usize; 2])>,
    thumb_order: VecDeque<PathBuf>,
    /// Standbild, das gerade im Hilfsthread geladen wird.
    thumb_job: Option<(PathBuf, Receiver<Result<sicht::Still, String>>)>,
    thumb_err: HashMap<PathBuf, String>,
    /// Video, das in der Ansicht läuft.
    player: Option<sicht::Player>,
    player_tex: Option<egui::TextureHandle>,
    /// Rückfrage «Hochformat entfernen?» offen.
    confirm_portrait: bool,
    /// Nach Pfeiltaste: diese Zeile in der Liste sichtbar machen.
    scroll_to: Option<usize>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0x2E, 0x9B, 0xD6]);
        stil::init(&cc.egui_ctx, [0x2E, 0x9B, 0xD6]);
        let ffmpeg = tool::find_ffmpeg();
        let status = if ffmpeg.is_some() {
            t("Bilder und Videos hineinziehen oder oben hinzufügen.").to_string()
        } else {
            t("ffmpeg wurde nicht gefunden. ffmpeg.exe neben das Programm legen oder installieren.").to_string()
        };
        App {
            items: Vec::new(),
            sel: BTreeSet::new(),
            anchor: None,
            drag: None,
            goto: 1,
            preview: None,
            pv_open: false,
            pv_label: String::new(),
            pv_speed: 100,
            pv_tex: None,
            pv_mode: None,
            s: Settings::default(),
            res: 0,
            only_sel: false,
            ffmpeg,
            out_path: tool::default_output().display().to_string(),
            recursive: true,
            probing: None,
            job: None,
            started: Instant::now(),
            status,
            status_ok: true,
            notes: Vec::new(),
            show_view: true,
            thumbs: HashMap::new(),
            thumb_order: VecDeque::new(),
            thumb_job: None,
            thumb_err: HashMap::new(),
            player: None,
            player_tex: None,
            confirm_portrait: false,
            scroll_to: None,
        }
    }

    // ---------------------------------------------------------- Dateien hinzufügen

    /// Nimmt Dateien und Ordner entgegen; gemessen wird im Hintergrund.
    fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let Some(ff) = self.ffmpeg.clone() else {
            self.status = t("ffmpeg fehlt – ohne ffmpeg kann Diawerk nichts einlesen.").into();
            self.status_ok = false;
            return;
        };
        if self.probing.is_some() {
            self.status = t("Es wird noch eingelesen – bitte einen Moment warten.").into();
            return;
        }
        let mut files: Vec<PathBuf> = Vec::new();
        for p in paths {
            if p.is_dir() {
                files.extend(model::collect_files(&p, self.recursive));
            } else if model::detect_kind(&p).is_some() {
                files.push(p);
            }
        }
        let known: HashSet<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        files.retain(|p| !known.contains(p) && seen.insert(p.clone()));
        if files.is_empty() {
            self.status = t("Keine neuen Bilder oder Videos gefunden.").into();
            return;
        }
        let n = files.len();
        let (tx, rx) = channel::<ProbeMsg>();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let next = AtomicUsize::new(0);
            let workers = 4.min(n).max(1);
            std::thread::scope(|sc| {
                for _ in 0..workers {
                    let (tx, files, ff, next, ctx2) = (tx.clone(), &files, &ff, &next, &ctx2);
                    sc.spawn(move || loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        if i >= files.len() {
                            break;
                        }
                        let r = model::probe(ff, &files[i]).map_err(|e| {
                            format!(
                                "{}: {}",
                                files[i].file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                                engine_msg(&e)
                            )
                        });
                        let _ = tx.send(ProbeMsg::One(i, r));
                        ctx2.request_repaint();
                    });
                }
            });
            let _ = tx.send(ProbeMsg::Done);
            ctx2.request_repaint();
        });
        self.status = tf("Lese {n} Dateien ein …", &[("n", &n)]);
        self.status_ok = true;
        self.probing = Some(Probing { rx, pending: (0..n).map(|_| None).collect(), got: 0 });
    }

    fn poll_probe(&mut self) {
        let mut finished = false;
        if let Some(p) = self.probing.as_mut() {
            while let Ok(msg) = p.rx.try_recv() {
                match msg {
                    ProbeMsg::One(i, r) => {
                        p.pending[i] = Some(r);
                        p.got += 1;
                    }
                    ProbeMsg::Done => finished = true,
                }
            }
            if !finished {
                self.status = tf("Lese ein … {n} / {m}", &[("n", &p.got), ("m", &p.pending.len())]);
            }
        }
        if finished {
            if let Some(p) = self.probing.take() {
                let mut ok = 0;
                let mut bad = Vec::new();
                for r in p.pending.into_iter().flatten() {
                    match r {
                        Ok(it) => {
                            self.items.push(it);
                            ok += 1;
                        }
                        Err(e) => bad.push(e),
                    }
                }
                self.status = tf("{ok} Dateien hinzugefügt.", &[("ok", &ok)]);
                self.status_ok = true;
                if !bad.is_empty() {
                    self.status.push(' ');
                    self.status.push_str(&tf("{n} nicht lesbar (siehe Hinweise).", &[("n", &bad.len())]));
                    self.notes.extend(bad.into_iter().map(|b| tf("Nicht lesbar: {b}", &[("b", &b)])));
                }
            }
        }
    }

    // ---------------------------------------------------------- Rendern

    fn start_render(&mut self) {
        let Some(ff) = self.ffmpeg.clone() else { return };
        let mut out = PathBuf::from(self.out_path.trim());
        if out.as_os_str().is_empty() {
            self.status = t("Bitte eine Ausgabedatei wählen.").into();
            self.status_ok = false;
            return;
        }
        let ext = self.s.format.ext();
        if out.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some(ext.to_string()) {
            out.set_extension(ext);
            self.out_path = out.display().to_string();
        }
        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() && !parent.is_dir() {
                self.status = t("Der Ausgabeordner existiert nicht.").into();
                self.status_ok = false;
                return;
            }
        }
        self.stop_preview();
        self.player = None;
        self.notes.clear();
        self.started = Instant::now();
        self.status = t("Diashow wird gerechnet …").into();
        self.status_ok = true;
        let list = self.render_list();
        if list.is_empty() {
            self.status = t("Keine Einträge gewählt.").into();
            self.status_ok = false;
            return;
        }
        self.job = Some(render::start(ff, model::with_credits(&list, &self.s), self.s.clone(), out));
    }

    /// Was in den Film kommt: alles, oder nur die Auswahl (in Listenreihenfolge).
    fn render_list(&self) -> Vec<Item> {
        if self.only_sel && !self.sel.is_empty() {
            self.sel.iter().filter_map(|&i| self.items.get(i).cloned()).collect()
        } else {
            self.items.clone()
        }
    }

    /// Kurzer Werbefilm fürs Handy: Hochformat, kürzere Standzeit, schnellere Übergänge.
    fn instagram_preset(&mut self) {
        self.res = RES_REEL;
        self.s.width = RES[RES_REEL].1;
        self.s.height = RES[RES_REEL].2;
        self.s.fps = 30;
        self.s.still_secs = 3.0;
        self.s.trans_secs = 0.6;
        self.s.format = model::OutFmt::Mp4;
        self.s.quality = model::Quality::Good;
        self.s.pano_min_secs = 5.0;
        self.s.pano_max_secs = 10.0;
        if self.s.video_max_secs == 0.0 {
            self.s.video_max_secs = 6.0;
        }
        self.s.credits.card_secs = self.s.credits.card_secs.min(3.0);
        if !self.sel.is_empty() {
            self.only_sel = true;
        }
        let out = PathBuf::from(self.out_path.trim());
        if let (Some(dir), Some(stem)) = (out.parent(), out.file_stem()) {
            let stem = stem.to_string_lossy();
            if !stem.ends_with("-Instagram") {
                self.out_path = dir.join(format!("{stem}-Instagram.mp4")).display().to_string();
            }
        }
        self.status = t("Für Instagram eingestellt: Hochformat 1080×1920, 3 s pro Bild, MP4.").into();
        self.status_ok = true;
    }

    fn poll_job(&mut self) {
        let Some(job) = self.job.clone() else { return };
        if !job.finished.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(mut l) = job.log.lock() {
            let msgs: Vec<String> = l.drain(..).map(|m| engine_msg(&m)).collect();
            self.notes.extend(msgs);
        }
        let res = job.result.lock().ok().and_then(|mut r| r.take());
        let took = self.started.elapsed().as_secs_f32();
        match res {
            Some(Ok(())) => {
                let zeit = model::fmt_hms(took);
                self.status = tf("Fertig nach {zeit} – {datei}", &[("zeit", &zeit), ("datei", &self.out_path)]);
                self.status_ok = true;
            }
            Some(Err(e)) => {
                self.status = engine_msg(&e);
                self.status_ok = false;
            }
            None => {}
        }
        self.job = None;
    }

    // ---------------------------------------------------------- Liste bearbeiten

    fn move_sel(&mut self, up: bool) {
        self.sel = model::move_selected(&mut self.items, &self.sel, up);
    }

    fn to_start(&mut self) {
        self.sel = model::place_at(&mut self.items, &self.sel, 0);
    }

    fn to_end(&mut self) {
        self.sel = model::place_at(&mut self.items, &self.sel, usize::MAX);
    }

    /// Der erste gewählte Eintrag soll danach die Nummer `nr` (ab 1) haben.
    fn to_number(&mut self, nr: usize) {
        self.sel = model::place_at(&mut self.items, &self.sel, nr.saturating_sub(1));
    }

    fn remove_sel(&mut self) {
        let first = self.sel.iter().next().copied().unwrap_or(0);
        let mut i = 0;
        let sel = std::mem::take(&mut self.sel);
        self.items.retain(|_| {
            let keep = !sel.contains(&i);
            i += 1;
            keep
        });
        self.sel = if self.items.is_empty() { BTreeSet::new() } else { [first.min(self.items.len() - 1)].into() };
        self.anchor = None;
    }

    fn spread(&mut self, videos: bool, panos: bool) {
        let items = std::mem::take(&mut self.items);
        self.items = model::spread_items(items, &self.s, videos, panos);
        self.sel.clear();
        self.anchor = None;
    }

    /// Alle Einträge im Hochformat aus der Liste nehmen (die Dateien bleiben unberührt).
    fn remove_portrait(&mut self) {
        let rows = model::portrait_rows(&self.items);
        if rows.is_empty() {
            return;
        }
        let n = rows.len();
        self.sel = rows.into_iter().collect();
        self.remove_sel();
        self.sel.clear();
        self.status = tf("{n} Einträge im Hochformat entfernt.", &[("n", &n)]);
        self.status_ok = true;
    }

    // ---------------------------------------------------------- Ansicht des gewählten Eintrags

    /// Der Eintrag, den die Ansicht zeigt: der zuletzt angeklickte, sonst der erste gewählte.
    fn current(&self) -> Option<usize> {
        let c = match self.anchor {
            Some(a) if self.sel.contains(&a) => Some(a),
            _ => self.sel.iter().next().copied(),
        };
        c.filter(|&i| i < self.items.len())
    }

    /// Fertig geladene Standbilder übernehmen; danach das nächste anstossen
    /// (zuerst der gewählte Eintrag, dann seine Nachbarn, damit das Blättern schnell geht).
    fn poll_thumbs(&mut self, ctx: &egui::Context) {
        if let Some((path, rx)) = &self.thumb_job {
            match rx.try_recv() {
                Ok(r) => {
                    let path = path.clone();
                    self.thumb_job = None;
                    match r {
                        Ok(st) => {
                            let size = [st.w as usize, st.h as usize];
                            let img = egui::ColorImage::from_rgb(size, &st.rgb);
                            let tex = ctx.load_texture(path.display().to_string(), img, egui::TextureOptions::LINEAR);
                            self.thumbs.insert(path.clone(), (tex, size));
                            self.thumb_order.push_back(path);
                            while self.thumb_order.len() > THUMB_CACHE {
                                if let Some(old) = self.thumb_order.pop_front() {
                                    self.thumbs.remove(&old);
                                }
                            }
                        }
                        Err(e) => {
                            self.thumb_err.insert(path, engine_msg(&e));
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.thumb_job = None,
            }
        }
        if !self.show_view || self.thumb_job.is_some() {
            return;
        }
        let Some(ff) = self.ffmpeg.clone() else { return };
        let Some(cur) = self.current() else { return };
        let wanted = [Some(cur), cur.checked_add(1), cur.checked_sub(1)];
        let next = wanted
            .into_iter()
            .flatten()
            .filter_map(|i| self.items.get(i))
            .find(|it| !self.thumbs.contains_key(&it.path) && !self.thumb_err.contains_key(&it.path))
            .cloned();
        if let Some(it) = next {
            let path = it.path.clone();
            let (tx, rx) = channel();
            let ctx2 = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(sicht::load_still(&ff, &it));
                ctx2.request_repaint();
            });
            self.thumb_job = Some((path, rx));
        }
    }

    /// Pfeiltasten wählen den vorigen/nächsten Eintrag, Entf nimmt die Auswahl aus der Liste.
    fn keys(&mut self, ctx: &egui::Context, can_edit: bool) {
        if self.items.is_empty() || ctx.wants_keyboard_input() {
            return;
        }
        let (down, up, del) = ctx.input(|i| {
            (i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::Delete))
        });
        if down || up {
            let last = self.items.len() - 1;
            let i = match self.current() {
                Some(c) if down => (c + 1).min(last),
                Some(c) => c.saturating_sub(1),
                None => 0,
            };
            self.sel = [i].into();
            self.anchor = Some(i);
            self.scroll_to = Some(i);
        }
        if del && can_edit && !self.sel.is_empty() {
            self.remove_sel();
            self.anchor = self.sel.iter().next().copied();
            self.scroll_to = self.anchor;
        }
    }

    /// Unter der Liste: das gewählte Bild bzw. Video gross, daneben die Angaben.
    fn view_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, slots: &[model::Slot], can_edit: bool) {
        let Some(cur) = self.current() else {
            self.player = None;
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(t("Einen Eintrag anklicken, um ihn hier zu sehen. Mit ⬆ ⬇ blättern.")).weak());
            });
            return;
        };
        let it = self.items[cur].clone();
        if self.player.as_ref().is_some_and(|p| p.path != it.path) {
            self.player = None;
        }
        // neues Videobild übernehmen
        let mut playing = false;
        let mut play_pos = 0.0;
        if let Some(p) = &self.player {
            if let Some(rgb) = p.take_frame() {
                let img = egui::ColorImage::from_rgb([p.w as usize, p.h as usize], &rgb);
                match &mut self.player_tex {
                    Some(tx) => tx.set(img, egui::TextureOptions::LINEAR),
                    None => self.player_tex = Some(ctx.load_texture("ansicht-video", img, egui::TextureOptions::LINEAR)),
                }
            }
            playing = !p.finished();
            play_pos = p.position();
        }
        if self.player.as_ref().is_some_and(|p| p.finished()) {
            self.player = None;
        }
        let pano = slots.get(cur).is_some_and(|s| s.pano);
        let portrait = model::is_portrait(&it);
        let mut toggle_play = false;
        let mut remove = false;
        ui.add_space(6.0);
        let avail = ui.available_size();
        ui.horizontal(|ui| {
            // ---- Bild links, so gross wie Platz ist
            let info_w = 250.0_f32.min(avail.x * 0.45);
            let box_w = (avail.x - info_w - 16.0).max(80.0);
            let box_h = (avail.y - 6.0).max(60.0);
            let ratio = it.w.max(1) as f32 / it.h.max(1) as f32;
            let (mut w, mut h) = (box_w, box_w / ratio);
            if h > box_h {
                h = box_h;
                w = box_h * ratio;
            }
            let size = egui::vec2(w.max(8.0), h.max(8.0));
            let tex = if self.player.is_some() { self.player_tex.as_ref().map(|t| t.id()) } else { None }
                .or_else(|| self.thumbs.get(&it.path).map(|(t, _)| t.id()));
            ui.allocate_ui_with_layout(egui::vec2(box_w, box_h), egui::Layout::top_down(egui::Align::Center), |ui| {
                match tex {
                    Some(id) => {
                        ui.add(egui::Image::new((id, size)));
                    }
                    None => {
                        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(24));
                        let msg = match self.thumb_err.get(&it.path) {
                            Some(e) => e.clone(),
                            None => t("Wird geladen …").to_string(),
                        };
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            msg,
                            egui::FontId::proportional(14.0),
                            egui::Color32::GRAY,
                        );
                    }
                }
            });
            ui.add_space(8.0);
            // ---- Angaben rechts
            ui.vertical(|ui| {
                ui.set_width(info_w);
                ui.label(egui::RichText::new(format!("{}. {}", cur + 1, it.name)).strong());
                let (kind, col) = match it.kind {
                    Kind::Video if portrait => (t("Video im Hochformat"), stil::gruen()),
                    Kind::Video => (t("Video"), stil::violett()),
                    _ if pano => (t("Panorama"), stil::orange()),
                    _ if portrait => (t("Hochformat"), stil::gruen()),
                    _ if it.w == it.h => (t("Quadrat"), stil::blau()),
                    _ => (t("Querformat"), stil::blau()),
                };
                stil::marke(ui, kind, col, 0.0);
                let seite = format!("{:.2} : 1", it.w as f32 / it.h.max(1) as f32);
                ui.label(format!("{} × {} · {}", it.w, it.h, seite));
                if it.kind == Kind::Video {
                    let laenge = model::fmt_hms(it.secs);
                    ui.label(tf("Länge {laenge}", &[("laenge", &laenge)]));
                }
                if let Some(sl) = slots.get(cur) {
                    let s = format!("{:.1}", sl.frames as f32 / self.s.fps.max(1) as f32);
                    ui.label(tf("Im Film {s} s", &[("s", &s)]));
                }
                if let Ok(md) = std::fs::metadata(&it.path) {
                    let mb = format!("{:.1}", md.len() as f64 / 1_048_576.0);
                    ui.label(egui::RichText::new(tf("{mb} MB", &[("mb", &mb)])).weak());
                }
                if portrait && it.kind == Kind::Image && self.s.width > self.s.height {
                    ui.small(t("Im Querformat-Film steht es ganz im Bild vor unscharfem Hintergrund."));
                }
                ui.add_space(6.0);
                if it.kind == Kind::Video && self.ffmpeg.is_some() {
                    ui.horizontal(|ui| {
                        let (s, lab) = if playing { (Sym::Pause, t("Anhalten")) } else { (Sym::Play, t("Abspielen")) };
                        if stil::knopf(ui, s, lab).clicked() {
                            toggle_play = true;
                        }
                        if playing {
                            ui.label(format!("{} / {}", model::fmt_hms(play_pos), model::fmt_hms(it.secs)));
                        }
                    });
                    if playing {
                        ui.add(egui::ProgressBar::new((play_pos / it.secs.max(0.1)).clamp(0.0, 1.0)).fill(stil::farbe()));
                    }
                }
                if stil::gefahr(ui, can_edit, Sym::Papierkorb, t("Aus der Liste entfernen"))
                    .on_hover_text(t("Nur aus der Liste, die Datei bleibt erhalten (Taste Entf)"))
                    .clicked()
                {
                    remove = true;
                }
            });
        });
        if toggle_play {
            if playing {
                self.player = None;
            } else if let Some(ff) = self.ffmpeg.clone() {
                let ctx2 = ctx.clone();
                match sicht::Player::start(&ff, &it, play_pos, Box::new(move || ctx2.request_repaint())) {
                    Ok(p) => {
                        self.player_tex = None;
                        self.player = Some(p);
                    }
                    Err(e) => {
                        self.status = engine_msg(&e);
                        self.status_ok = false;
                    }
                }
            }
        }
        if remove {
            self.player = None;
            self.sel = [cur].into();
            self.remove_sel();
            self.anchor = self.sel.iter().next().copied();
            self.scroll_to = self.anchor;
        }
    }

    // ---------------------------------------------------------- Vorschau

    fn stop_preview(&mut self) {
        if let Some(p) = self.preview.take() {
            p.stop();
        }
    }

    fn start_preview(&mut self, mode: PvMode) {
        let Some(ff) = self.ffmpeg.clone() else { return };
        if self.items.is_empty() {
            return;
        }
        let first = self.sel.iter().next().copied().unwrap_or(0).min(self.items.len() - 1);
        let c = &self.s.credits;
        let (list, intro, outro, label): (Vec<Item>, bool, bool, String) = match mode {
            PvMode::Start => (self.items.clone(), c.intro, c.outro, t("ab Anfang").into()),
            PvMode::FromSel => (self.items[first..].to_vec(), false, c.outro, tf("ab Nr. {n}", &[("n", &(first + 1))])),
            PvMode::OnlySel => (
                self.sel.iter().filter_map(|&i| self.items.get(i).cloned()).collect(),
                false,
                false,
                tf("{n} gewählte", &[("n", &self.sel.len())]),
            ),
        };
        self.stop_preview();
        self.player = None;
        let items = model::with_cards(&list, &self.s, intro, outro);
        self.preview = Some(render::preview(ff, items, &self.s, self.pv_speed));
        self.pv_mode = Some(mode);
        self.pv_label = label;
        self.pv_open = true;
    }

    fn show_preview(&mut self, ctx: &egui::Context) {
        let Some(pv) = self.preview.clone() else {
            self.pv_open = false;
            return;
        };
        // neuesten Frame als Bild übernehmen
        if let Some((_, rgb)) = pv.take_frame() {
            let img = egui::ColorImage::from_rgb([pv.w, pv.h], &rgb);
            match &mut self.pv_tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.pv_tex = Some(ctx.load_texture("vorschau", img, egui::TextureOptions::LINEAR)),
            }
        }
        let running = pv.running();
        if running {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        let mut open = self.pv_open;
        let mut restart = false;
        let mut stop = false;
        egui::Window::new(tf("Vorschau – {name}", &[("name", &self.pv_label)]))
            .open(&mut open)
            .default_width(680.0)
            .resizable(true)
            .show(ctx, |ui| {
                // Hochformat: nach der Höhe richten, damit das Fenster nicht zu lang wird.
                let ratio = pv.h as f32 / pv.w.max(1) as f32;
                let w = ui.available_width().clamp(320.0, 1400.0).min(if ratio > 1.0 { 640.0 / ratio } else { f32::MAX });
                let size = egui::vec2(w, w * ratio);
                match &self.pv_tex {
                    Some(t) => {
                        ui.add(egui::Image::new((t.id(), size)));
                    }
                    None => {
                        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
                        if running {
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                t("Bilder werden vorbereitet …"),
                                egui::FontId::proportional(16.0),
                                egui::Color32::GRAY,
                            );
                        }
                    }
                }
                let (pos, len) = (pv.position(), pv.length());
                ui.add(egui::ProgressBar::new(if len > 0.0 { (pos / len).clamp(0.0, 1.0) } else { 0.0 }).text(format!(
                    "{} / {}",
                    model::fmt_hms(pos),
                    model::fmt_hms(len)
                )));
                ui.horizontal(|ui| {
                    if stil::knopf(ui, Sym::Aktualisieren, t("Neu starten")).clicked() {
                        restart = true;
                    }
                    if stil::knopf_an(ui, running, Sym::Stopp, t("Stopp")).clicked() {
                        stop = true;
                    }
                    ui.separator();
                    ui.label(t("Tempo:"));
                    for (lab, v) in [("1×", 100u32), ("2×", 200), ("4×", 400)] {
                        if ui.selectable_label(self.pv_speed == v, lab).clicked() {
                            self.pv_speed = v;
                            pv.speed.store(v, Ordering::Relaxed);
                        }
                    }
                });
                if let Ok(l) = pv.ctl.log.lock() {
                    if let Some(last) = l.last() {
                        ui.colored_label(stil::orange(), engine_msg(last));
                    }
                }
                ui.small(t("Verkleinerte Vorschau (640 Pixel breit) – Übergänge, Schwenks und Dauer wie im fertigen Film, ohne Musik."));
            });
        self.pv_open = open;
        if stop {
            pv.stop();
        }
        if restart {
            if let Some(m) = self.pv_mode {
                self.start_preview(m);
            }
        }
        if !self.pv_open {
            self.stop_preview();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_probe();
        self.poll_job();
        let busy = self.job.is_some() || self.probing.is_some();
        self.sel.retain(|&i| i < self.items.len());
        self.keys(ctx, self.job.is_none() && self.probing.is_none());
        self.poll_thumbs(ctx);
        if busy {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        // Dateien/Ordner aufs Fenster ziehen
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() && self.job.is_none() {
            self.add_paths(ctx, dropped);
        }

        let running = self.job.is_some();
        let film = self.render_list();
        let tl = model::build_timeline(&film, &self.s);
        // Für die Liste: Zeitplan aller Einträge (Index = Zeile).
        let tl_all = model::build_timeline(&self.items, &self.s);
        let total_len = model::build_timeline(&model::with_credits(&film, &self.s), &self.s).total_secs(self.s.fps);
        let n_vid = film.iter().filter(|i| i.kind == Kind::Video).count();
        let n_pano = tl.slots.iter().filter(|s| s.pano).count();
        let n_img = film.len() - n_vid - n_pano;

        let ffmpeg_ok = self.ffmpeg.is_some();
        let mut pv_req: Option<PvMode> = None;
        let can_edit = !running && self.probing.is_none();
        let can_prev = ffmpeg_ok && !running && !self.items.is_empty() && self.probing.is_none();
        let n_base = self.items.len().saturating_sub(n_vid + n_pano);
        let n_hoch = model::portrait_rows(&self.items).len();

        // ---------------- Kopfleiste
        stil::kopfleiste(ctx, |ui| {
            stil::titel(ui, "Diawerk");
            stil::trenner(ui);
            if stil::knopf_an(ui, can_edit, Sym::Plus, t("Bilder / Videos")).on_hover_text(t("Bilder und Videos wählen")).clicked() {
                let mut exts: Vec<&str> = model::IMAGE_EXT.to_vec();
                exts.extend_from_slice(model::VIDEO_EXT);
                if let Some(files) = rfd::FileDialog::new().set_title(t("Bilder und Videos wählen")).add_filter(t("Bilder und Videos"), &exts).pick_files() {
                    self.add_paths(ctx, files);
                }
            }
            stil::menue(ui, can_edit, Sym::Ordner, t("Ordner"), |ui| {
                if stil::eintrag(ui, true, Sym::OrdnerAuf, t("Ordner wählen …")).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().set_title(t("Ordner wählen")).pick_folder() {
                        self.add_paths(ctx, vec![dir]);
                    }
                }
                let lab = if self.recursive { t("Mit Unterordnern (an)") } else { t("Mit Unterordnern (aus)") };
                if stil::eintrag(ui, true, if self.recursive { Sym::Haken } else { Sym::Kreuz }, lab).clicked() {
                    self.recursive = !self.recursive;
                }
            });
            stil::trenner(ui);
            let many = can_edit && self.items.len() > 1;
            stil::menue(ui, many, Sym::Sortieren, t("Sortieren"), |ui| {
                if stil::eintrag(ui, true, Sym::Text, t("Nach Namen")).clicked() {
                    self.items.sort_by(|a, b| model::natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy()));
                    self.sel.clear();
                }
                if stil::eintrag(ui, true, Sym::Kalender, t("Nach Datum")).clicked() {
                    self.items.sort_by_key(|i| i.modified);
                    self.sel.clear();
                }
            });
            stil::menue(ui, many && (n_vid > 0 || n_pano > 0), Sym::Mischen, t("Einstreuen"), |ui| {
                if stil::eintrag(ui, n_vid > 0 && self.items.len() > n_vid, Sym::Film, t("Videos einstreuen")).on_hover_text(t("Verteilt die Videos gleichmässig zwischen die Bilder")).clicked() {
                    self.spread(true, false);
                }
                if stil::eintrag(ui, n_pano > 0 && n_base + n_vid > 0, Sym::Panorama, t("Panoramen einstreuen")).on_hover_text(t("Verteilt die Panoramen gleichmässig zwischen die übrigen Einträge")).clicked() {
                    self.spread(false, true);
                }
                if stil::eintrag(ui, n_pano > 0 && n_vid > 0 && n_base > 0, Sym::Mischen, t("Beides einstreuen")).on_hover_text(t("Videos und Panoramen gleichmässig zwischen die normalen Bilder verteilen")).clicked() {
                    self.spread(true, true);
                }
            });
            if n_hoch > 0 {
                stil::menue(ui, can_edit, Sym::Bild, &tf("{n} im Hochformat", &[("n", &n_hoch)]), |ui| {
                    if stil::eintrag(ui, true, Sym::Haken, t("Alle wählen")).on_hover_text(t("Alle Bilder und Videos im Hochformat auswählen")).clicked() {
                        self.sel = model::portrait_rows(&self.items).into_iter().collect();
                        self.anchor = self.sel.iter().next().copied();
                        self.scroll_to = self.anchor;
                    }
                    if stil::eintrag(ui, true, Sym::Papierkorb, t("Alle entfernen …")).on_hover_text(t("Alle Bilder und Videos im Hochformat aus der Liste nehmen")).clicked() {
                        self.confirm_portrait = true;
                    }
                });
            }
            stil::trenner(ui);
            stil::menue(ui, can_prev, Sym::Play, t("Vorschau"), |ui| {
                if stil::eintrag(ui, true, Sym::Play, t("Ab Anfang")).clicked() {
                    pv_req = Some(PvMode::Start);
                }
                if stil::eintrag(ui, !self.sel.is_empty(), Sym::Pfeil, t("Ab Auswahl")).clicked() {
                    pv_req = Some(PvMode::FromSel);
                }
                if stil::eintrag(ui, !self.sel.is_empty(), Sym::Haken, t("Nur Auswahl")).clicked() {
                    pv_req = Some(PvMode::OnlySel);
                }
            });
            stil::rechts(ui, |ui| {
                stil::menue(ui, true, Sym::Mehr, "", |ui| {
                    if stil::eintrag(ui, can_edit && !self.items.is_empty(), Sym::Haken, t("Alle wählen")).clicked() {
                        self.sel = (0..self.items.len()).collect();
                    }
                    if stil::eintrag(ui, can_edit && !self.items.is_empty(), Sym::Papierkorb, t("Liste leeren")).clicked() {
                        self.items.clear();
                        self.sel.clear();
                    }
                });
                let (s, h) = if self.show_view { (Sym::Auge, t("Ansicht ausblenden")) } else { (Sym::Auge, t("Ansicht einblenden")) };
                if stil::leise(ui, true, s, "", h).clicked() {
                    self.show_view = !self.show_view;
                }
            });
        });

        // ---------------- Auswahlleiste (nur mit Auswahl)
        if !self.sel.is_empty() && !self.items.is_empty() {
            let n = self.sel.len();
            stil::auswahlleiste(ctx, &tf("{n} gewählt", &[("n", &n)]), |ui| {
                if stil::leise(ui, can_edit, Sym::Pfeil, "", t("Eine Stelle nach oben")).clicked() {
                    self.move_sel(true);
                }
                if stil::leise(ui, can_edit, Sym::PfeilRunter, "", t("Eine Stelle nach unten")).clicked() {
                    self.move_sel(false);
                }
                if stil::leise(ui, can_edit, Sym::Anfang, "", t("An den Anfang")).clicked() {
                    self.to_start();
                }
                if stil::leise(ui, can_edit, Sym::Ende, "", t("Ans Ende")).clicked() {
                    self.to_end();
                }
                stil::trenner(ui);
                ui.label(egui::RichText::new(t("an Nr.")).color(thema::muted()));
                ui.add_sized([56.0, 26.0], egui::DragValue::new(&mut self.goto).range(1..=self.items.len().max(1)));
                if stil::knopf_text(ui, can_edit, t("dorthin")).on_hover_text(t("Gewählte Einträge an diese Nummer setzen")).clicked() {
                    let nr = self.goto;
                    self.to_number(nr);
                }
                stil::trenner(ui);
                if stil::leise(ui, can_prev, Sym::Play, t("Nur Auswahl abspielen"), "").clicked() {
                    pv_req = Some(PvMode::OnlySel);
                }
                if stil::gefahr(ui, can_edit, Sym::Papierkorb, t("Entfernen")).on_hover_text(t("Nur aus der Liste, die Dateien bleiben (Taste Entf)")).clicked() {
                    self.remove_sel();
                }
                stil::rechts(ui, |ui| {
                    if stil::leise(ui, true, Sym::Kreuz, t("Auswahl aufheben"), "").clicked() {
                        self.sel.clear();
                        self.anchor = None;
                    }
                });
            });
        }

        // ---------------- Fussleiste: Zusammenfassung, Ausgabe, Hauptknopf
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if !self.items.is_empty() {
                        let laenge = model::fmt_hms(total_len);
                        ui.label(egui::RichText::new(tf(
                            "{bilder} Bilder · {panos} Panoramen · {videos} Videos · Länge ca. {laenge}",
                            &[("bilder", &n_img), ("panos", &n_pano), ("videos", &n_vid), ("laenge", &laenge)],
                        )).size(13.0).color(thema::muted()));
                    }
                    let col = if self.status_ok { thema::text() } else { stil::rot() };
                    ui.label(egui::RichText::new(&self.status).size(13.0).color(col));
                    if self.s.height > self.s.width && total_len > REEL_MAX_SECS {
                        ui.label(egui::RichText::new(t("Länger als 3 Minuten: zu lang für ein Instagram-Reel.")).size(12.5).color(stil::rot()));
                    }
                });
                stil::rechts(ui, |ui| {
                    if let Some(job) = self.job.clone() {
                        if stil::knopf(ui, Sym::Stopp, t("Abbrechen")).clicked() {
                            job.cancel.store(true, Ordering::SeqCst);
                        }
                        let done = job.done.load(Ordering::Relaxed) as f32;
                        let total = job.total.load(Ordering::Relaxed).max(1) as f32;
                        let frac = (done / total).clamp(0.0, 1.0);
                        let el = self.started.elapsed().as_secs_f32();
                        if frac > 0.02 {
                            let zeit = model::fmt_hms(el / frac - el);
                            ui.label(egui::RichText::new(tf("noch ca. {zeit}", &[("zeit", &zeit)])).color(thema::muted()));
                        }
                        ui.add(egui::ProgressBar::new(frac).show_percentage().desired_width(260.0).fill(stil::farbe()));
                    } else {
                        let can_start = self.ffmpeg.is_some() && !self.items.is_empty() && self.probing.is_none() && (!self.only_sel || !self.sel.is_empty());
                        if stil::haupt(ui, can_start, Sym::Play, t("Diashow erstellen")).clicked() {
                            self.start_render();
                        }
                        ui.add_space(4.0);
                        ui.scope(|ui| {
                            ui.set_width(150.0);
                            stil::schalter(ui, &mut self.only_sel, &tf("nur Auswahl ({n})", &[("n", &self.sel.len())]));
                        })
                        .response
                        .on_hover_text(t("Nur die gewählten Bilder und Videos kommen in den Film (z. B. für einen kurzen Werbefilm)."));
                        if stil::leise(ui, true, Sym::OrdnerAuf, "", t("Ausgabedatei wählen …")).clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .set_title(t("Diashow speichern unter"))
                                .set_file_name(format!("Diashow.{}", self.s.format.ext()))
                                .add_filter(t("Video"), &[self.s.format.ext()])
                                .save_file()
                            {
                                self.out_path = p.display().to_string();
                            }
                        }
                        ui.add(egui::TextEdit::singleline(&mut self.out_path).desired_width(300.0).hint_text(t("Ausgabedatei")));
                        if !self.notes.is_empty() {
                            stil::menue(ui, true, Sym::Warnung, &tf("Hinweise ({n})", &[("n", &self.notes.len())]), |ui| {
                                ui.set_max_width(520.0);
                                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                                    for n in &self.notes {
                                        ui.label(n);
                                    }
                                });
                            });
                        }
                    }
                });
            });
        });

        // ---------------- rechts: Einstellungen als Karten
        stil::seitenleiste(ctx, 330.0, |ui| {
            ui.add_enabled_ui(!running, |ui| {
                stil::karte(ui, "film", Sym::Film, t("Film"), true, |ui| {
                    if stil::leise(ui, true, Sym::Kamera, t("Für Instagram einstellen"), t("Hochformat 1080×1920, 3 s pro Bild, schnelle Übergänge, MP4. Mit Auswahl: nur die gewählten Einträge.")).clicked() {
                        self.instagram_preset();
                    }
                    stil::feld(ui, t("Auflösung"), |ui| {
                        let opts: Vec<(usize, &str)> = RES.iter().enumerate().map(|(i, r)| (i, t(r.0))).collect();
                        stil::liste(ui, "aufloesung", &mut self.res, &opts);
                    });
                    self.s.width = RES[self.res].1;
                    self.s.height = RES[self.res].2;
                    ui.columns(2, |c| {
                        stil::feld(&mut c[0], t("Bilder pro Sekunde"), |ui| {
                            let opts: Vec<(u32, String)> = [24u32, 25, 30, 50, 60].iter().map(|f| (*f, f.to_string())).collect();
                            let refs: Vec<(u32, &str)> = opts.iter().map(|(f, s)| (*f, s.as_str())).collect();
                            stil::liste(ui, "fps", &mut self.s.fps, &refs);
                        });
                        stil::feld(&mut c[1], t("Qualität"), |ui| {
                            let opts: Vec<(Quality, &str)> = Quality::ALL.iter().map(|q| (*q, t(q.label()))).collect();
                            stil::liste(ui, "qualitaet", &mut self.s.quality, &opts);
                        });
                    });
                    stil::feld(ui, t("Format"), |ui| {
                        let before = self.s.format;
                        let opts: Vec<(OutFmt, &str)> = OutFmt::ALL.iter().map(|f| (*f, t(f.label()))).collect();
                        stil::liste(ui, "format", &mut self.s.format, &opts);
                        if self.s.format != before {
                            let mut p = PathBuf::from(self.out_path.trim());
                            if !p.as_os_str().is_empty() {
                                p.set_extension(self.s.format.ext());
                                self.out_path = p.display().to_string();
                            }
                        }
                    });
                });
                stil::karte(ui, "bilder", Sym::Bild, t("Bilder"), true, |ui| {
                    stil::feld(ui, t("Sekunden pro Bild"), |ui| {
                        stil::schieber(ui, &mut self.s.still_secs, 2.0..=15.0, "s", 1);
                    });
                    stil::feld(ui, t("sanfter Zoom"), |ui| {
                        stil::schieber(ui, &mut self.s.zoom_pct, 0.0..=10.0, "%", 0);
                    });
                    stil::schalter(ui, &mut self.s.crop_to_fill, t("Bilder nahe 16:9 füllen den Schirm"));
                    stil::hinweis(ui, t("Aus: nichts wird abgeschnitten. Hoch- und 4:3-Bilder stehen immer ganz im Bild, vor unscharfem Hintergrund."));
                });
                stil::karte_farbe(ui, "panoramen", Sym::Panorama, stil::orange(), &format!("{} · {}", t("Panoramen"), n_pano), false, |ui| {
                    stil::feld(ui, t("ab Seitenverhältnis"), |ui| {
                        stil::schieber(ui, &mut self.s.pano_ratio, 1.8..=4.0, ": 1", 1);
                    });
                    stil::feld(ui, t("Tempo"), |ui| {
                        stil::schieber(ui, &mut self.s.pano_speed, 60.0..=500.0, "px/s", 0);
                    });
                    stil::feld(ui, t("mindestens"), |ui| {
                        stil::schieber(ui, &mut self.s.pano_min_secs, 3.0..=20.0, "s", 0);
                    });
                    stil::feld(ui, t("höchstens"), |ui| {
                        stil::schieber(ui, &mut self.s.pano_max_secs, 8.0..=60.0, "s", 0);
                    });
                    stil::feld(ui, t("Ruhe am Rand"), |ui| {
                        stil::schieber(ui, &mut self.s.pano_pause, 0.0..=2.0, "s", 1);
                    });
                    stil::hinweis(ui, t("Panoramen fahren von links nach rechts und bleiben länger stehen. Tempo gilt bei 1080 Zeilen Höhe."));
                });
                stil::karte(ui, "uebergaenge", Sym::Uebergang, t("Übergänge"), false, |ui| {
                    stil::feld(ui, t("Art"), |ui| {
                        let opts: Vec<(Transition, &str)> = Transition::ALL.iter().map(|tr| (*tr, t(tr.label()))).collect();
                        stil::liste(ui, "uebergang", &mut self.s.transition, &opts);
                    });
                    stil::feld(ui, t("Dauer"), |ui| {
                        stil::schieber(ui, &mut self.s.trans_secs, 0.0..=3.0, "s", 1);
                    });
                    stil::hinweis(ui, t("Vor und nach Panoramen wird immer weich überblendet. Am Anfang und Ende blendet der Film aus Schwarz ein und aus."));
                });
                stil::karte_farbe(ui, "videos", Sym::Film, stil::violett(), t("Videos"), false, |ui| {
                    stil::feld(ui, t("längstens"), |ui| {
                        stil::schieber(ui, &mut self.s.video_max_secs, 0.0..=300.0, "s", 0);
                    });
                    stil::hinweis(ui, t("0 = ganze Länge. Der Ton der Videos wird weggelassen."));
                });
                stil::karte(ui, "musik", Sym::Musik, t("Musik"), false, |ui| {
                    if self.s.music.is_empty() {
                        stil::grau(ui, t("keine"));
                    }
                    let mut remove: Option<usize> = None;
                    let mut up: Option<usize> = None;
                    for (i, m) in self.s.music.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let name = m.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            stil::symbol(ui, Sym::Musik, 15.0, stil::akzent());
                            ui.add(egui::Label::new(format!("{}. {}", i + 1, name)).truncate());
                            stil::rechts(ui, |ui| {
                                if stil::leise(ui, true, Sym::Kreuz, "", t("Entfernen")).clicked() {
                                    remove = Some(i);
                                }
                                if i > 0 && stil::leise(ui, true, Sym::Pfeil, "", t("Eine Stelle nach oben")).clicked() {
                                    up = Some(i);
                                }
                            });
                        });
                    }
                    if let Some(i) = up {
                        self.s.music.swap(i, i - 1);
                    }
                    if let Some(i) = remove {
                        self.s.music.remove(i);
                    }
                    if stil::knopf(ui, Sym::Plus, t("Musik hinzufügen …")).clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .set_title(t("Musik wählen (mehrere möglich)"))
                            .add_filter(t("Musik"), &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus"])
                            .pick_files()
                        {
                            self.s.music.extend(files);
                        }
                    }
                    stil::hinweis(ui, t("Die Titel laufen der Reihe nach, werden bei Bedarf wiederholt, blenden ein und am Ende aus."));
                });
                stil::karte(ui, "abspann", Sym::Text, t("Vor- und Abspann"), false, |ui| {
                    stil::schalter(ui, &mut self.s.credits.intro, t("Vorspann mit dem Fotografen"));
                    stil::schalter(ui, &mut self.s.credits.outro, t("Abspann mit Dank"));
                    stil::feld(ui, t("Dauer"), |ui| {
                        stil::schieber(ui, &mut self.s.credits.card_secs, 3.0..=20.0, "s", 0);
                    });
                    let w = ui.available_width();
                    stil::feld(ui, t("Name"), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.s.credits.name).desired_width(w));
                    });
                    stil::feld(ui, t("Titelzeile (Vorspann)"), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.s.credits.intro_kicker).desired_width(w));
                    });
                    stil::feld(ui, t("Text (Vorspann)"), |ui| {
                        ui.add(egui::TextEdit::multiline(&mut self.s.credits.intro_text).desired_width(w).desired_rows(3));
                    });
                    stil::feld(ui, t("Titelzeile (Abspann)"), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.s.credits.outro_kicker).desired_width(w));
                    });
                    stil::feld(ui, t("Text (Abspann)"), |ui| {
                        ui.add(egui::TextEdit::multiline(&mut self.s.credits.outro_text).desired_width(w).desired_rows(3));
                    });
                    match self.s.credits.portrait.clone() {
                        Some(p) => {
                            let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.horizontal(|ui| {
                                stil::symbol(ui, Sym::Kamera, 15.0, stil::akzent());
                                ui.add(egui::Label::new(tf("Porträt: {name}", &[("name", &name)])).truncate());
                                stil::rechts(ui, |ui| {
                                    if stil::leise(ui, true, Sym::Kreuz, "", t("Porträt entfernen")).clicked() {
                                        self.s.credits.portrait = None;
                                    }
                                });
                            });
                        }
                        None => {
                            stil::grau(ui, t("Porträt: keines"));
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        if stil::knopf(ui, Sym::Kamera, t("Porträt wählen …")).clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .set_title(t("Porträt des Fotografen"))
                                .add_filter(t("Bilder"), &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff"])
                                .pick_file()
                            {
                                self.s.credits.portrait = Some(p);
                            }
                        }
                        if stil::knopf(ui, Sym::Text, t("Schrift wählen … (optional)")).clicked() {
                            if let Some(p) = rfd::FileDialog::new().set_title(t("Schriftdatei")).add_filter(t("Schrift"), &["ttf", "otf"]).pick_file() {
                                self.s.credits.font = Some(p);
                            }
                        }
                    });
                    if model::find_font(self.s.credits.font.as_deref()).is_none() {
                        ui.colored_label(stil::rot(), t("Keine Systemschrift gefunden – bitte eine Schrift wählen."));
                    }
                    stil::hinweis(ui, t("Layout wie die Webseite des Fotografen: links Titel, Name und Text, rechts das Porträt."));
                });
            });
        });

        // ---------------- Mitte: die Liste
        stil::mitte(ctx, |ui| {
            if self.items.is_empty() {
                stil::leer(ui, Sym::Bild, t("Noch keine Bilder"), t("Ordner oder Dateien hierher ziehen – oder oben hinzufügen. Videos (MP4, MOV …) kommen in die Liste wie Bilder."));
                return;
            }
            if self.show_view {
                egui::TopBottomPanel::bottom("ansicht")
                    .resizable(true)
                    .default_height(280.0)
                    .height_range(140.0..=900.0)
                    .frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::symmetric(4.0, 8.0)))
                    .show_inside(ui, |ui| self.view_ui(ui, ctx, &tl_all.slots, can_edit));
            } else {
                self.player = None;
            }
            let row_h = 34.0;
            let items = &self.items;
            let slots = &tl_all.slots;
            let fps = self.s.fps as f32;
            let sel = &self.sel;
            let dragging = self.drag.is_some();
            let area = ui.available_rect_before_wrap();
            let ptr = ctx.input(|i| i.pointer.hover_pos());
            let mods = ctx.input(|i| i.modifiers);
            let mut clicked: Option<usize> = None;
            let mut drag_started: Option<usize> = None;
            let mut rects: Vec<(usize, egui::Rect)> = Vec::new();
            let scroll_to = self.scroll_to.take();
            let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if let Some(i) = scroll_to {
                // Zeile ungefähr in die Mitte holen
                let y = (i as f32 * row_h - area.height() / 2.0).max(0.0);
                sa = sa.vertical_scroll_offset(y);
            }
            sa.show_rows(ui, row_h, items.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if dragging {
                        if let Some(p) = ptr {
                            if p.y < area.top() + 28.0 {
                                ui.scroll_with_delta(egui::vec2(0.0, 16.0));
                            } else if p.y > area.bottom() - 28.0 {
                                ui.scroll_with_delta(egui::vec2(0.0, -16.0));
                            }
                        }
                    }
                    for i in range {
                        let it = &items[i];
                        let sl = &slots[i];
                        let (tag, col) = match (it.kind, sl.pano) {
                            (Kind::Video, _) => (t("VIDEO"), stil::violett()),
                            (Kind::Intro | Kind::Outro, _) => (t("KARTE"), stil::violett()),
                            (_, true) => (t("PANORAMA"), stil::orange()),
                            _ if model::is_portrait(it) => (t("HOCH"), stil::gruen()),
                            _ => (t("BILD"), stil::blau()),
                        };
                        let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), egui::Sense::click_and_drag());
                        let on = sel.contains(&i);
                        if on {
                            ui.painter().rect_filled(rect.shrink2(egui::vec2(0.0, 1.0)), 8.0, stil::toenung());
                        } else if r.hovered() {
                            ui.painter().rect_filled(rect.shrink2(egui::vec2(0.0, 1.0)), 8.0, thema::hover());
                        }
                        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(10.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        row.spacing_mut().item_spacing.x = 12.0;
                        row.add_sized([30.0, row_h], egui::Label::new(egui::RichText::new(format!("{}", i + 1)).color(thema::muted())).selectable(false));
                        stil::marke(&mut row, tag, col, 72.0);
                        let hoch = if it.kind == Kind::Video && model::is_portrait(it) { format!(" · {}", t("hoch")) } else { String::new() };
                        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |row| {
                            row.label(egui::RichText::new(format!("{:.1} s", sl.frames as f32 / fps)).color(thema::muted()));
                            row.add_sized([96.0, row_h], egui::Label::new(egui::RichText::new(format!("{}×{}{hoch}", it.w, it.h)).color(thema::muted())).selectable(false));
                            row.with_layout(egui::Layout::left_to_right(egui::Align::Center), |row| {
                                row.add(egui::Label::new(egui::RichText::new(&it.name).color(thema::text())).truncate().selectable(false));
                            });
                        });
                        rects.push((i, rect));
                        if r.clicked() {
                            clicked = Some(i);
                        }
                        if can_edit && r.drag_started() {
                            drag_started = Some(i);
                        }
                    }
                });

            // Auswahl: Klick, Strg+Klick, Umschalt+Klick
            if let Some(i) = clicked {
                if mods.shift {
                    let a = self.anchor.unwrap_or(i);
                    let range = a.min(i)..=a.max(i);
                    if !(mods.ctrl || mods.command) {
                        self.sel.clear();
                    }
                    self.sel.extend(range);
                } else if mods.ctrl || mods.command {
                    if !self.sel.remove(&i) {
                        self.sel.insert(i);
                    }
                    self.anchor = Some(i);
                } else {
                    self.sel.clear();
                    self.sel.insert(i);
                    self.anchor = Some(i);
                }
            }

            // Umsortieren per Ziehen
            if let Some(i) = drag_started {
                if !self.sel.contains(&i) {
                    self.sel.clear();
                    self.sel.insert(i);
                    self.anchor = Some(i);
                }
                self.drag = Some(i);
            }
            if self.drag.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                // Einfügemarke: vor der Zeile unter dem Zeiger (obere Hälfte) oder dahinter
                let mut target: Option<(usize, f32)> = None;
                if let (Some(p), Some(first), Some(last)) = (ptr, rects.first(), rects.last()) {
                    if p.y <= first.1.top() {
                        target = Some((first.0, first.1.top()));
                    } else if p.y >= last.1.bottom() {
                        target = Some((last.0 + 1, last.1.bottom()));
                    } else if let Some((i, r)) = rects.iter().find(|(_, r)| p.y >= r.top() && p.y < r.bottom()) {
                        target = Some(if p.y < r.center().y { (*i, r.top()) } else { (*i + 1, r.bottom()) });
                    }
                }
                if let Some((_, y)) = target {
                    ui.painter().hline(area.x_range(), y, egui::Stroke::new(2.0, stil::akzent()));
                }
                if ctx.input(|i| i.pointer.any_released()) {
                    if let Some((before, _)) = target {
                        let sel = self.sel.clone();
                        self.sel = model::move_before(&mut self.items, &sel, before);
                        self.anchor = self.sel.iter().next().copied();
                    }
                    self.drag = None;
                } else {
                    ctx.request_repaint();
                }
            }
        });

        if self.confirm_portrait {
            let n = model::portrait_rows(&self.items).len();
            let mut yes = false;
            let mut no = false;
            egui::Window::new(t("Hochformat entfernen"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(tf("{n} Bilder und Videos im Hochformat aus der Liste nehmen?", &[("n", &n)]));
                    ui.small(t("Die Dateien selbst bleiben unberührt."));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if stil::gefahr(ui, true, Sym::Papierkorb, t("Entfernen")).clicked() {
                            yes = true;
                        }
                        if stil::knopf_text(ui, true, t("Abbrechen")).clicked() {
                            no = true;
                        }
                    });
                });
            if yes {
                self.player = None;
                self.remove_portrait();
            }
            if yes || no || n == 0 {
                self.confirm_portrait = false;
            }
        }
        if let Some(m) = pv_req {
            self.start_preview(m);
        }
        if self.preview.is_some() {
            self.show_preview(ctx);
        }
    }
}
