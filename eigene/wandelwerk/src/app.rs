//! Oberfläche (egui), passend zum Mockup: links die gemischte Dateiliste,
//! rechts die Ausgabe-Einstellungen für die gewählte Datei. Umwandeln stellt die
//! Dateien in eine Warteschlange (höchstens ein paar gleichzeitig, siehe warteschlange.rs);
//! Fortschritt wird gepollt. Ordner werden im Hintergrund durchsucht.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::convert::{self, sperren, Job, JobState, Papierkorb};
use crate::model::{
    self, Ablage, Aspect, AudioFmt, AudioTrack, Auswahl, Fit, ImageFmt, MediaKind, Quality, Target, VideoFmt,
};
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::thema;
use crate::warteschlange::Warteschlange;

/// Logofarbe für Schrift und Linien – je Design lesbar auf dem Hintergrund.
fn vio() -> egui::Color32 {
    stil::akzent()
}
/// Füllung von Flächen mit weisser Schrift darauf.
fn vio_fill() -> egui::Color32 {
    stil::farbe()
}
/// Hintergrund für Gewähltes (Datei-Zeile, Chip).
fn vio_soft() -> egui::Color32 {
    stil::toenung()
}
fn green() -> egui::Color32 {
    stil::gruen()
}
fn red() -> egui::Color32 {
    stil::rot()
}
/// Füllung der Typ-Symbole (weisser Buchstabe darauf) – in beiden Designs gleich.
fn kind_color(k: MediaKind) -> egui::Color32 {
    match k {
        MediaKind::Audio => egui::Color32::from_rgb(0x16, 0xa3, 0x4a),
        MediaKind::Image => egui::Color32::from_rgb(0x25, 0x63, 0xeb),
        MediaKind::Video => vio_fill(),
    }
}

/// Höhe des Inhalts einer Datei-Zeile (fest, damit die Liste auch mit Tausenden Dateien nur die
/// sichtbaren Zeilen zeichnen muss).
const ZEILE_INNEN: f32 = 40.0;
/// Ganze Zeilenhöhe: Inhalt plus Innenrand oben und unten.
const ZEILE: f32 = ZEILE_INNEN + 22.0;

struct Entry {
    input: PathBuf,
    name: String,
    /// Name des Ordners, in dem die Datei liegt (zur Unterscheidung gleicher Namen).
    ordner: String,
    kind: MediaKind,
    dims: Option<(u32, u32)>,
    target: Target,
    aspect: Aspect,
    fit: Fit,
    quality: Quality,
    job: Option<Arc<JobState>>,
    /// Tonspuren eines Videos (None, solange ffmpeg noch nachschaut oder fehlt).
    tracks: Arc<Mutex<Option<Vec<AudioTrack>>>>,
    /// Tonspuren schon abgefragt? (erst, wenn das Video gewählt wird)
    probed: bool,
    /// Gewählte Tonspur für «Nur Ton»; None = Standardspur der Datei.
    audio_track: Option<u32>,
    /// «Sprache verbessern» für Ton und Video.
    sprache: model::Sprache,
}

impl Entry {
    /// Tonspur, die beim Umwandeln benutzt wird (nur bei mehreren Spuren nötig).
    fn chosen_track(&self) -> Option<u32> {
        let tracks = self.tracks.lock().ok()?.clone()?;
        if tracks.len() < 2 {
            return None;
        }
        self.audio_track.or_else(|| tracks.iter().find(|t| t.default).map(|t| t.index)).or(Some(0))
    }

    fn zustand(&self) -> Zustand {
        let schon = || model::schon_im_ziel(&self.input, self.target);
        match &self.job {
            Some(j) if j.done.load(Ordering::SeqCst) => {
                if j.ok.load(Ordering::SeqCst) {
                    Zustand::Fertig
                } else if schon() {
                    Zustand::SchonImZiel
                } else if j.abgebrochen.load(Ordering::SeqCst) {
                    Zustand::Angehalten
                } else {
                    Zustand::Fehler
                }
            }
            Some(j) if j.running.load(Ordering::SeqCst) => Zustand::Laeuft,
            Some(j) if j.abgebrochen.load(Ordering::SeqCst) => {
                if schon() {
                    Zustand::SchonImZiel
                } else {
                    Zustand::Angehalten
                }
            }
            Some(_) => Zustand::Wartet,
            None if schon() => Zustand::SchonImZiel,
            None => Zustand::Offen,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Zustand {
    Offen,
    /// Datei hat schon die Endung des Zielformats: wird nicht umgewandelt, nie gelöscht.
    SchonImZiel,
    Wartet,
    Laeuft,
    Fertig,
    Fehler,
    Angehalten,
}

impl Zustand {
    /// Noch nicht (erfolgreich) umgewandelt und nicht unterwegs: darf neu eingereiht werden.
    fn offen(self) -> bool {
        matches!(self, Zustand::Offen | Zustand::Fehler | Zustand::Angehalten)
    }
    /// Unterwegs oder fertig: Einstellungen gelten dafür nicht mehr.
    fn gesperrt(self) -> bool {
        matches!(self, Zustand::Wartet | Zustand::Laeuft | Zustand::Fertig)
    }
}

/// Ergebnis einer Suche (im Hintergrund), mit Bildgrössen.
struct Gefunden {
    dateien: Vec<(PathBuf, MediaKind, Option<(u32, u32)>)>,
    ordner_uebersprungen: usize,
    abgeschnitten: bool,
}

impl Gefunden {
    fn zaehlen(&self) -> [usize; 3] {
        let mut n = [0usize; 3];
        for (_, k, _) in &self.dateien {
            n[model::kind_index(*k)] += 1;
        }
        n
    }
}

/// Laufende Suche; `ordner` = aus einem Ordner (dann wird bei gemischten Arten gefragt).
struct Suche {
    ordner: bool,
    ergebnis: Arc<Mutex<Option<Gefunden>>>,
}

/// Gefundene Dateien aus Ordnern, die noch auf die Wahl «nur Ton / … / alles» warten.
struct Angebot {
    gefunden: Gefunden,
    wahl: Auswahl,
}

pub struct App {
    entries: Vec<Entry>,
    sel: usize,
    out_dir: String,
    ffmpeg: Option<String>,
    ctx: egui::Context,
    /// Schon aufgenommene Dateien (keine doppelten Einträge).
    bekannt: HashSet<PathBuf>,
    /// Zuletzt gewähltes Zielformat je Art [Ton, Bild, Video] – gilt für neu hinzugefügte Dateien.
    zuletzt: [Option<Target>; 3],
    ablage: Ablage,
    /// «Originale nach dem Umwandeln löschen» (Papierkorb). Immer aus beim Start.
    originale_loeschen: bool,
    /// Rückfrage vor dem Start: so viele Originale kämen in den Papierkorb.
    frage_loeschen: Option<usize>,
    warteschlange: Warteschlange<(Job, Arc<JobState>)>,
    suchen: Vec<Suche>,
    angebot: Option<Angebot>,
    /// Kurze Meldung über der Liste (z. B. «120 Dateien hinzugefügt»).
    info: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0x2B, 0x8A, 0x6E]);
        stil::init(&cc.egui_ctx, [0x2B, 0x8A, 0x6E]);
        let ffmpeg = convert::find_ffmpeg();
        let kerne = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        let (ff, ctx) = (ffmpeg.clone(), cc.egui_ctx.clone());
        let warteschlange = Warteschlange::new(model::gleichzeitig(kerne), move |(job, st): (Job, Arc<JobState>)| {
            convert::run_job(&job, ff.as_deref(), &st);
            ctx.request_repaint();
        });
        App {
            entries: Vec::new(),
            sel: 0,
            out_dir: default_out_dir(),
            ffmpeg,
            ctx: cc.egui_ctx.clone(),
            bekannt: HashSet::new(),
            zuletzt: [None; 3],
            ablage: Ablage::default(),
            originale_loeschen: false,
            frage_loeschen: None,
            warteschlange,
            suchen: Vec::new(),
            angebot: None,
            info: None,
        }
    }

    /// Nimmt eine Datei in die Liste auf; false, wenn sie schon drin ist.
    fn add_entry(&mut self, p: PathBuf, kind: MediaKind, dims: Option<(u32, u32)>) -> bool {
        if !self.bekannt.insert(p.clone()) {
            return false;
        }
        let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ordner = p
            .parent()
            .and_then(|d| d.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let target = self.zuletzt[model::kind_index(kind)].unwrap_or_else(|| model::default_target(kind));
        self.entries.push(Entry {
            input: p,
            name,
            ordner,
            kind,
            dims,
            target,
            aspect: Aspect::Original,
            fit: Fit::Crop,
            quality: Quality::Hoch,
            job: None,
            tracks: Arc::default(),
            probed: false,
            audio_track: None,
            sprache: model::Sprache::default(),
        });
        true
    }

    /// Fragt ffmpeg nach den Tonspuren eines Videos – erst, wenn es gewählt wird (sonst liefen
    /// bei einem grossen Ordner Tausende ffmpeg gleichzeitig).
    fn tonspuren_pruefen(&mut self, i: usize) {
        let Some(e) = self.entries.get_mut(i) else { return };
        if e.kind != MediaKind::Video || e.probed {
            return;
        }
        e.probed = true;
        if let Some(ff) = self.ffmpeg.clone() {
            let (tr, path, ctx) = (e.tracks.clone(), e.input.clone(), self.ctx.clone());
            let _ = std::thread::Builder::new().name("tonspuren".into()).spawn(move || {
                let found = convert::probe_audio(&ff, &path);
                *sperren(&tr) = found;
                ctx.request_repaint();
            });
        }
    }

    /// Durchsucht Ordner/Dateien im Hintergrund (Unterordner eingeschlossen).
    fn suche_starten(&mut self, wurzeln: Vec<PathBuf>, ordner: bool) {
        if wurzeln.is_empty() {
            return;
        }
        let slot: Arc<Mutex<Option<Gefunden>>> = Arc::default();
        let (s, ctx) = (slot.clone(), self.ctx.clone());
        let gestartet = std::thread::Builder::new()
            .name("suchen".into())
            .spawn(move || {
                let fund = model::sammeln(&wurzeln, model::MAX_TIEFE, model::MAX_DATEIEN);
                let dateien = fund
                    .dateien
                    .into_iter()
                    .map(|(p, k)| {
                        let dims = if k == MediaKind::Image { image::image_dimensions(&p).ok() } else { None };
                        (p, k, dims)
                    })
                    .collect();
                *sperren(&s) = Some(Gefunden {
                    dateien,
                    ordner_uebersprungen: fund.ordner_uebersprungen,
                    abgeschnitten: fund.abgeschnitten,
                });
                ctx.request_repaint();
            })
            .is_ok();
        if gestartet {
            self.suchen.push(Suche { ordner, ergebnis: slot });
        } else {
            self.info = Some(t("Die Suche konnte nicht gestartet werden.").to_string());
        }
    }

    /// Fertige Suchen übernehmen: Einzeldateien direkt, Ordner mit gemischten Arten als Angebot.
    fn suchen_abholen(&mut self) {
        let mut fertig: Vec<(bool, Gefunden)> = Vec::new();
        self.suchen.retain(|s| match sperren(&s.ergebnis).take() {
            Some(g) => {
                fertig.push((s.ordner, g));
                false
            }
            None => true,
        });
        for (ordner, g) in fertig {
            let arten = g.zaehlen().iter().filter(|&&n| n > 0).count();
            if ordner && (arten > 1 || self.angebot.is_some()) {
                match &mut self.angebot {
                    Some(a) => {
                        a.gefunden.dateien.extend(g.dateien);
                        a.gefunden.ordner_uebersprungen += g.ordner_uebersprungen;
                        a.gefunden.abgeschnitten |= g.abgeschnitten;
                        a.wahl = Auswahl::vorschlag(a.gefunden.zaehlen());
                    }
                    None => {
                        let wahl = Auswahl::vorschlag(g.zaehlen());
                        self.angebot = Some(Angebot { gefunden: g, wahl });
                    }
                }
            } else {
                self.aufnehmen(g, Auswahl::Alles, ordner);
            }
        }
    }

    /// Gefundene Dateien der gewählten Art in die Liste; bei Ordnern mit Meldung.
    fn aufnehmen(&mut self, g: Gefunden, wahl: Auswahl, melden: bool) {
        let (mut neu, mut doppelt) = (0usize, 0usize);
        for (p, k, dims) in g.dateien {
            if !wahl.passt(k) {
                continue;
            }
            if self.add_entry(p, k, dims) {
                neu += 1;
            } else {
                doppelt += 1;
            }
        }
        if !melden {
            return;
        }
        let mut m = if neu == 0 && doppelt == 0 {
            t("Keine unterstützten Dateien gefunden.").to_string()
        } else {
            tf("{n} Dateien hinzugefügt", &[("n", &neu)])
        };
        if doppelt > 0 {
            m.push_str(&tf(" · {n} waren schon in der Liste", &[("n", &doppelt)]));
        }
        if g.ordner_uebersprungen > 0 {
            m.push_str(&tf(" · {n} Ordner nicht lesbar", &[("n", &g.ordner_uebersprungen)]));
        }
        if g.abgeschnitten {
            m.push_str(t(" · nicht alle Dateien aufgenommen (zu viele oder zu tief)"));
        }
        self.info = Some(m);
    }

    /// Zielformat-Chip: gilt für alle gleichartigen Dateien, die noch nicht umgewandelt sind
    /// (ein Ton-Ziel auch für Videos), und für neu hinzugefügte Dateien.
    fn ziel_waehlen(&mut self, tgt: Target) {
        let sel = self.sel;
        for (i, e) in self.entries.iter_mut().enumerate() {
            let z = e.zustand();
            if z == Zustand::Wartet || z == Zustand::Laeuft {
                continue;
            }
            if model::ziel_passt(e.kind, tgt) && (i == sel || !z.gesperrt()) {
                e.target = tgt;
            }
        }
        self.merken(tgt);
    }

    fn merken(&mut self, tgt: Target) {
        for k in [MediaKind::Audio, MediaKind::Image, MediaKind::Video] {
            if model::ziel_passt(k, tgt) {
                self.zuletzt[model::kind_index(k)] = Some(tgt);
            }
        }
    }

    fn apply_to_all(&mut self) {
        let Some(s) = self.entries.get(self.sel) else { return };
        let (aspect, fit, quality, sprache) = (s.aspect, s.fit, s.quality, s.sprache);
        // Zielformat nur auf gleichartige Dateien übertragen.
        let tgt = s.target;
        for e in self.entries.iter_mut() {
            if matches!(e.zustand(), Zustand::Wartet | Zustand::Laeuft) {
                continue;
            }
            e.aspect = aspect;
            e.fit = fit;
            e.quality = quality;
            if e.kind != MediaKind::Image {
                e.sprache = sprache;
            }
            // «Nur Ton» gilt auch für Videos in der Liste.
            if model::ziel_passt(e.kind, tgt) {
                e.target = tgt;
            }
        }
        self.merken(tgt);
    }

    /// Wie viele Dateien «Umwandeln» einreihen würde.
    fn offene(&self) -> usize {
        self.entries.iter().filter(|e| e.zustand().offen()).count()
    }

    /// Hauptknopf: mit «Originale löschen» erst nachfragen.
    fn umwandeln_klick(&mut self) {
        let n = self.offene();
        if n == 0 {
            return;
        }
        if self.originale_loeschen {
            self.frage_loeschen = Some(n);
        } else {
            self.start_all();
        }
    }

    fn start_all(&mut self) {
        let out_dir = PathBuf::from(&self.out_dir);
        let (ablage, loeschen) = (self.ablage, self.originale_loeschen);
        for e in self.entries.iter_mut() {
            if !e.zustand().offen() {
                continue;
            }
            let job = Job {
                input: e.input.clone(),
                target: e.target,
                aspect: e.aspect,
                fit: e.fit,
                quality: e.quality,
                out_dir: model::ziel_ordner(&e.input, ablage, &out_dir),
                audio_track: if e.target.kind() == MediaKind::Audio { e.chosen_track() } else { None },
                sprache: e.sprache,
                original_in_papierkorb: loeschen,
            };
            let st = JobState::new();
            self.warteschlange.add((job, st.clone()));
            e.job = Some(st);
        }
    }

    /// Wartende Aufträge anhalten (laufende werden noch fertig).
    fn anhalten(&mut self) {
        for e in &self.entries {
            if let Some(j) = &e.job {
                if !j.running.load(Ordering::SeqCst) && !j.done.load(Ordering::SeqCst) {
                    j.abgebrochen.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    fn entfernen(&mut self, i: usize) {
        if i >= self.entries.len() {
            return;
        }
        let e = self.entries.remove(i);
        if let Some(j) = &e.job {
            j.abgebrochen.store(true, Ordering::SeqCst);
        }
        self.bekannt.remove(&e.input);
        if self.sel >= self.entries.len() {
            self.sel = self.entries.len().saturating_sub(1);
        }
    }

    fn leeren(&mut self) {
        self.anhalten();
        self.entries.clear();
        self.bekannt.clear();
        self.sel = 0;
        self.info = None;
    }
}

/// Zahlen für die Fussleiste.
#[derive(Default)]
struct Zahlen {
    offen: usize,
    wartet: usize,
    laeuft: usize,
    fertig: usize,
    fehler: usize,
    schon: usize,
    behalten: usize,
}

impl App {
    fn zahlen(&self) -> Zahlen {
        let mut z = Zahlen::default();
        for e in &self.entries {
            let s = e.zustand();
            match s {
                Zustand::Offen | Zustand::Angehalten => z.offen += 1,
                Zustand::SchonImZiel => z.schon += 1,
                Zustand::Wartet => z.wartet += 1,
                Zustand::Laeuft => z.laeuft += 1,
                Zustand::Fertig => z.fertig += 1,
                Zustand::Fehler => {
                    z.fehler += 1;
                    z.offen += 1;
                }
            }
            if s == Zustand::Fertig {
                if let Some(j) = &e.job {
                    if matches!(*sperren(&j.papierkorb), Papierkorb::Behalten(_)) {
                        z.behalten += 1;
                    }
                }
            }
        }
        z
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Hereingezogene Dateien und Ordner aufnehmen (Ordner mit allen Unterordnern).
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            let (ordner, dateien): (Vec<PathBuf>, Vec<PathBuf>) = dropped.into_iter().partition(|p| p.is_dir());
            self.suche_starten(dateien, false);
            self.suche_starten(ordner, true);
        }
        self.suchen_abholen();
        let zahlen = self.zahlen();
        if zahlen.wartet + zahlen.laeuft > 0 || !self.suchen.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }

        self.top_bar(ctx);
        self.status_bar(ctx, &zahlen);
        self.right_panel(ctx);
        self.left_panel(ctx, &zahlen);
        self.rueckfrage(ctx);
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        stil::kopfleiste(ctx, |ui| {
            stil::titel(ui, "Wandelwerk");
            stil::trenner(ui);
            if stil::knopf(ui, Sym::Plus, t("Hinzufügen")).clicked() {
                self.pick_files();
            }
            if stil::knopf(ui, Sym::Ordner, t("Ordner hinzufügen …"))
                .on_hover_text(t("Alle Ton-, Bild- und Videodateien aus dem Ordner und allen Unterordnern"))
                .clicked()
            {
                if let Some(dirs) = rfd::FileDialog::new().pick_folders() {
                    self.suche_starten(dirs, true);
                }
            }
            stil::rechts(ui, |ui| {
                if stil::gefahr(ui, !self.entries.is_empty(), Sym::Papierkorb, t("Leeren")).clicked() {
                    self.leeren();
                }
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context, z: &Zahlen) {
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                let (col, txt) = if self.ffmpeg.is_some() {
                    let txt = if convert::HAS_EMBEDDED_FFMPEG { t("ffmpeg eingebaut") } else { t("ffmpeg gefunden") };
                    (green(), txt)
                } else {
                    (red(), t("ffmpeg fehlt (nur Bilder möglich)"))
                };
                ui.painter().circle_filled(dot.center(), 4.0, col);
                stil::grau(ui, txt).on_hover_text(t("Bilder in Rust · Ton & Video über ffmpeg"));
                let n = self.entries.len();
                if n > 0 {
                    let mut s = format!("· {}", tf("{n} Dateien", &[("n", &n)]));
                    if z.fertig > 0 {
                        s.push_str(&tf(" · {n} fertig", &[("n", &z.fertig)]));
                    }
                    if z.laeuft + z.wartet > 0 {
                        s.push_str(&tf(" · {n} in Arbeit", &[("n", &(z.laeuft + z.wartet))]));
                    }
                    if z.fehler > 0 {
                        s.push_str(&tf(" · {n} mit Fehler", &[("n", &z.fehler)]));
                    }
                    if z.schon > 0 {
                        s.push_str(&tf(" · {n} schon im Zielformat", &[("n", &z.schon)]));
                    }
                    stil::grau(ui, &s);
                }
                stil::rechts(ui, |ui| {
                    let label = tf("{n} Dateien umwandeln", &[("n", &z.offen)]);
                    if stil::haupt(ui, z.offen > 0, Sym::Umwandeln, &label).clicked() {
                        self.umwandeln_klick();
                    }
                    if z.wartet > 0
                        && stil::knopf(ui, Sym::Pause, t("Anhalten"))
                            .on_hover_text(t("Wartende Dateien nicht mehr umwandeln (laufende werden fertig)"))
                            .clicked()
                    {
                        self.anhalten();
                    }
                });
            });
        });
    }

    /// Rückfrage vor dem Umwandeln, wenn Originale in den Papierkorb kommen.
    fn rueckfrage(&mut self, ctx: &egui::Context) {
        let Some(n) = self.frage_loeschen else { return };
        let (mut ja, mut nein) = (false, false);
        let wohin = match self.ablage {
            Ablage::NebenOriginal => t("Die neuen Dateien kommen jeweils neben das Original.").to_string(),
            Ablage::Ausgabeordner => tf("Die neuen Dateien kommen in den Ausgabeordner: {ordner}", &[("ordner", &self.out_dir)]),
        };
        egui::Window::new(t("Originale in den Papierkorb?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(egui::RichText::new(tf("{n} Originale kommen nach dem Umwandeln in den Papierkorb.", &[("n", &n)])).strong());
                ui.add_space(4.0);
                ui.label(t("Nur Dateien, die erfolgreich umgewandelt wurden. Dateien mit Fehler und Dateien im Zielformat bleiben, wo sie sind."));
                ui.add_space(4.0);
                ui.label(wohin);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if stil::gefahr(ui, true, Sym::Papierkorb, t("Umwandeln, Originale in den Papierkorb")).clicked() {
                        ja = true;
                    }
                    if stil::knopf_text(ui, true, t("Abbrechen")).clicked() {
                        nein = true;
                    }
                });
            });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            nein = true;
        }
        if ja {
            self.frage_loeschen = None;
            self.start_all();
        } else if nein {
            self.frage_loeschen = None;
        }
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        stil::seitenleiste(ctx, 380.0, |ui| {
            if !self.entries.is_empty() {
                if self.sel >= self.entries.len() {
                    self.sel = 0;
                }
                self.tonspuren_pruefen(self.sel);
                self.output_card(ui);
            }
            self.action_card(ui);
        });
    }

    fn output_card(&mut self, ui: &mut egui::Ui) {
        stil::karte(ui, "ausgabe", Sym::Umwandeln, t("Ausgabe"), true, |ui| {
            let kind = self.entries[self.sel].kind;
            let name = self.entries[self.sel].name.clone();
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(name).weak()).truncate());
                stil::rechts(ui, |ui| {
                    if stil::leise(ui, true, Sym::Kopieren, t("Für alle übernehmen"), "").clicked() {
                        self.apply_to_all();
                    }
                });
            });

            // Zielformat je nach Typ – gilt für alle gleichartigen Dateien in der Liste.
            ui.add_space(10.0);
            sub(ui, match kind { MediaKind::Audio=>t("Zielformat (Ton)"), MediaKind::Image=>t("Zielformat (Bild)"), MediaKind::Video=>t("Zielformat (Video)") });
            let cur = self.entries[self.sel].target;
            let mut gewaehlt: Option<Target> = None;
            ui.horizontal_wrapped(|ui| match kind {
                MediaKind::Audio => {
                    for f in AudioFmt::all() {
                        if chip(ui, cur == Target::Audio(f), f.label()).clicked() { gewaehlt = Some(Target::Audio(f)); }
                    }
                }
                MediaKind::Image => {
                    for f in ImageFmt::all() {
                        if chip(ui, cur == Target::Image(f), f.label()).clicked() { gewaehlt = Some(Target::Image(f)); }
                    }
                }
                MediaKind::Video => {
                    for f in VideoFmt::all() {
                        if chip(ui, cur == Target::Video(f), f.label()).clicked() { gewaehlt = Some(Target::Video(f)); }
                    }
                }
            });
            stil::hinweis(ui, match kind {
                MediaKind::Audio => t("Gilt für alle Tondateien und Videos mit «Nur Ton» in der Liste."),
                MediaKind::Image => t("Gilt für alle Bilder in der Liste."),
                MediaKind::Video => t("Gilt für alle Videos in der Liste."),
            });

            // Video: nur die Tonspur speichern (z. B. MP3 aus einem MKV-Film).
            if kind == MediaKind::Video {
                ui.add_space(12.0);
                sub(ui, t("Nur Ton speichern"));
                ui.horizontal_wrapped(|ui| {
                    for f in AudioFmt::all() {
                        if chip(ui, cur == Target::Audio(f), f.label()).clicked() { gewaehlt = Some(Target::Audio(f)); }
                    }
                });
            }
            if let Some(g) = gewaehlt {
                self.ziel_waehlen(g);
            }
            if kind == MediaKind::Video {
                let e = &mut self.entries[self.sel];
                if e.target.kind() == MediaKind::Audio {
                    let tracks = e.tracks.lock().ok().and_then(|g| g.clone());
                    match tracks {
                        Some(tl) if tl.is_empty() => {
                            ui.add_space(6.0);
                            ui.colored_label(red(), t("Dieses Video hat keine Tonspur."));
                        }
                        Some(tl) if tl.len() > 1 => {
                            ui.add_space(10.0);
                            sub(ui, t("Tonspur"));
                            let chosen = e.chosen_track();
                            for tr in &tl {
                                if chip(ui, chosen == Some(tr.index), &track_label(tr)).clicked() {
                                    e.audio_track = Some(tr.index);
                                }
                            }
                        }
                        Some(tl) => {
                            if let Some(first) = tl.first() {
                                ui.add_space(4.0);
                                ui.weak(tf("Tonspur: {spur}", &[("spur", &track_label(first))]));
                            }
                        }
                        None => {}
                    }
                }
            }

            // Sprache verbessern: für Ton und Videos mit Ton (nicht GIF, nicht Bilder)
            let tgt = self.entries[self.sel].target;
            if tgt.kind() == MediaKind::Audio || (tgt.kind() == MediaKind::Video && tgt != Target::Video(VideoFmt::Gif)) {
                ui.add_space(12.0);
                sub(ui, t("Sprache verbessern"));
                let e = &mut self.entries[self.sel];
                if e.sprache.sprecher && tgt.kind() == MediaKind::Audio {
                    // «Zwei Sprecher angleichen» gleicht die Lautstärke immer mit an.
                    ui.add_enabled_ui(false, |ui| {
                        let mut an = true;
                        stil::schalter(ui, &mut an, t("Stimmen gleich laut machen"));
                    });
                } else {
                    stil::schalter(ui, &mut e.sprache.angleichen, t("Stimmen gleich laut machen"));
                }
                if tgt.kind() == MediaKind::Audio {
                    stil::schalter(ui, &mut e.sprache.sprecher, t("Zwei Sprecher angleichen (Klang)"));
                    stil::hinweis(ui, t("Erkennt zwei Stimmen und gleicht die dumpfere im Klang an. Ausgabe in Mono."));
                    ui.add_space(4.0);
                }
                stil::schalter(ui, &mut e.sprache.brummen, t("Brummen und Rumpeln entfernen"));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(t("Hintergrundgeräusche entfernen")).size(13.5));
                ui.horizontal_wrapped(|ui| {
                    for r in model::Rauschen::all() {
                        if chip(ui, e.sprache.rauschen == r, t(r.label())).clicked() { e.sprache.rauschen = r; }
                    }
                });
                if e.sprache.rauschen == model::Rauschen::Stark {
                    ui.weak(t("Stark kann Stimmen etwas blechern klingen lassen."));
                }
            }

            // Seitenverhältnis + Anpassung nur für Bild/Video
            let kind = self.entries[self.sel].target.kind();
            if kind != MediaKind::Audio {
                ui.add_space(12.0);
                sub(ui, t("Seitenverhältnis"));
                let e = &mut self.entries[self.sel];
                ui.horizontal_wrapped(|ui| {
                    for a in Aspect::all() {
                        if chip(ui, e.aspect == a, a.label()).clicked() { e.aspect = a; }
                    }
                });

                // Vorschau
                let (asp, fit, dims) = {
                    let e = &self.entries[self.sel];
                    (e.aspect, e.fit, e.dims)
                };
                ui.add_space(8.0);
                let (prect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 118.0), egui::Sense::hover());
                draw_preview(ui, prect, dims, asp, fit);

                ui.add_space(10.0);
                sub(ui, t("Anpassung"));
                let e = &mut self.entries[self.sel];
                ui.horizontal_wrapped(|ui| {
                    for f in [Fit::Crop, Fit::Pad, Fit::Stretch] {
                        if chip(ui, e.fit == f, t(f.label())).clicked() { e.fit = f; }
                    }
                });
            }

            // Qualität (nicht bei verlustfreien Formaten wie WAV/FLAC/PNG/BMP/TIFF – trotzdem anzeigen, schadet nicht)
            ui.add_space(12.0);
            sub(ui, t("Qualität"));
            let e = &mut self.entries[self.sel];
            ui.horizontal_wrapped(|ui| {
                for q in [Quality::Hoch, Quality::Mittel, Quality::Klein] {
                    if chip(ui, e.quality == q, t(q.label())).clicked() { e.quality = q; }
                }
            });
        });
    }

    fn action_card(&mut self, ui: &mut egui::Ui) {
        stil::karte(ui, "zielordner", Sym::Ordner, t("Zielordner"), true, |ui| {
            stil::wahl(
                ui,
                &mut self.ablage,
                &[(Ablage::Ausgabeordner, t("In den Ausgabeordner")), (Ablage::NebenOriginal, t("Neben das Original"))],
            );
            ui.add_space(8.0);
            match self.ablage {
                Ablage::Ausgabeordner => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(short_dir(&self.out_dir))).truncate());
                        stil::rechts(ui, |ui| {
                            if stil::knopf(ui, Sym::OrdnerAuf, t("Wählen …")).clicked() {
                                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                                    self.out_dir = d.display().to_string();
                                }
                            }
                        });
                    });
                }
                Ablage::NebenOriginal => {
                    stil::hinweis(ui, t("Jede neue Datei kommt in den Ordner ihres Originals."));
                }
            }
            ui.add_space(8.0);
            stil::schalter(ui, &mut self.originale_loeschen, t("Originale nach dem Umwandeln löschen"));
            if self.originale_loeschen {
                stil::hinweis(ui, t("Originale kommen in den Papierkorb – nur nach erfolgreicher Umwandlung. Vor dem Start wird nachgefragt."));
            }
        });
    }

    /// Meldungen über der Liste: Suche läuft, Auswahl nach Art, Ergebnis, behaltene Originale.
    fn meldungen(&mut self, ui: &mut egui::Ui, z: &Zahlen) {
        if !self.suchen.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t("Ordner werden durchsucht …"));
            });
            ui.add_space(8.0);
        }
        let mut aufnehmen: Option<Auswahl> = None;
        let mut verwerfen = false;
        if let Some(a) = &mut self.angebot {
            let n = a.gefunden.zaehlen();
            banner(ui, vio(), |ui| {
                ui.label(
                    egui::RichText::new(tf(
                        "Gefunden: {ton} Ton · {bild} Bilder · {video} Videos",
                        &[("ton", &n[0]), ("bild", &n[1]), ("video", &n[2])],
                    ))
                    .strong(),
                );
                ui.add_space(2.0);
                ui.label(t("Welche Dateien aufnehmen?"));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let wahlen = [
                        (Auswahl::Ton, tf("nur Ton ({n})", &[("n", &n[0])]), n[0]),
                        (Auswahl::Bilder, tf("nur Bilder ({n})", &[("n", &n[1])]), n[1]),
                        (Auswahl::Videos, tf("nur Videos ({n})", &[("n", &n[2])]), n[2]),
                        (Auswahl::Alles, tf("alles ({n})", &[("n", &(n[0] + n[1] + n[2]))]), 1),
                    ];
                    for (w, label, anzahl) in wahlen {
                        if anzahl > 0 && chip(ui, a.wahl == w, &label).clicked() {
                            a.wahl = w;
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let k = a.gefunden.dateien.iter().filter(|(_, k, _)| a.wahl.passt(*k)).count();
                    if stil::knopf(ui, Sym::Plus, &tf("{n} Dateien hinzufügen", &[("n", &k)])).clicked() {
                        aufnehmen = Some(a.wahl);
                    }
                    if stil::knopf_text(ui, true, t("Abbrechen")).clicked() {
                        verwerfen = true;
                    }
                });
            });
        }
        if let Some(w) = aufnehmen {
            if let Some(a) = self.angebot.take() {
                self.aufnehmen(a.gefunden, w, true);
            }
        } else if verwerfen {
            self.angebot = None;
        }
        let mut zu = false;
        if let Some(m) = &self.info {
            banner(ui, thema::border(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(m.as_str());
                    stil::rechts(ui, |ui| {
                        if stil::leise(ui, true, Sym::Kreuz, "", t("Schliessen")).clicked() {
                            zu = true;
                        }
                    });
                });
            });
        }
        if zu {
            self.info = None;
        }
        if z.behalten > 0 {
            banner(ui, stil::orange(), |ui| {
                ui.colored_label(
                    stil::orange(),
                    tf("{n} Originale behalten: Papierkorb nicht möglich. Details: Maus auf «Original behalten».", &[("n", &z.behalten)]),
                );
            });
        }
    }

    fn left_panel(&mut self, ctx: &egui::Context, z: &Zahlen) {
        stil::mitte(ctx, |ui| {
            self.meldungen(ui, z);
            if self.entries.is_empty() {
                if self.suchen.is_empty() && self.angebot.is_none() {
                    stil::leer(ui, Sym::Umwandeln, t("Noch keine Dateien"), t("Dateien oder Ordner hierher ziehen – Ton, Bild oder Video gemischt."));
                }
                return;
            }
            let mut remove: Option<usize> = None;
            let hoehe = (ui.available_height() - 24.0).max(ZEILE);
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(hoehe).show_rows(
                ui,
                ZEILE,
                self.entries.len(),
                |ui, bereich| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    for i in bereich {
                        self.file_row(ui, i, &mut remove);
                    }
                },
            );
            if let Some(i) = remove {
                self.entfernen(i);
            }
            ui.add_space(4.0);
            stil::hinweis(ui, t("Dateien oder Ordner hierher ziehen – Ton, Bild oder Video gemischt."));
        });
    }

    fn file_row(&mut self, ui: &mut egui::Ui, i: usize, remove: &mut Option<usize>) {
        let Some(e) = self.entries.get(i) else { return };
        let sel = i == self.sel;
        let fill = if sel { vio_soft() } else { thema::panel() };
        let stroke = if sel { vio() } else { thema::border() };
        let resp = egui::Frame::none()
            .fill(fill)
            .rounding(11.0)
            .stroke(egui::Stroke::new(1.0, stroke))
            .inner_margin(egui::Margin::symmetric(13.0, 11.0))
            .show(ui, |ui| {
                ui.set_height(ZEILE_INNEN);
                ui.horizontal(|ui| {
                    // Typ-Icon
                    let (ic, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                    ui.painter().rect_filled(ic, 9.0, kind_color(e.kind));
                    let s = match e.kind { MediaKind::Audio => Sym::Musik, MediaKind::Image => Sym::Bild, MediaKind::Video => Sym::Film };
                    stil::sym(ui.painter(), egui::Rect::from_center_size(ic.center(), egui::vec2(20.0, 20.0)), s, egui::Color32::WHITE);
                    ui.add_space(4.0);
                    // Rechts zuerst (Entfernen, Status), damit lange Namen davor abgeschnitten werden.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if stil::leise(ui, true, Sym::Kreuz, "", t("Entfernen")).clicked() {
                            *remove = Some(i);
                        }
                        ui.add_space(6.0);
                        status_badge(ui, e);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(e.name.as_str()).strong().size(13.5)).truncate());
                            let laeuft = e.job.as_ref().is_some_and(|j| j.running.load(Ordering::SeqCst));
                            match &e.job {
                                Some(j) if laeuft => {
                                    ui.add(egui::ProgressBar::new(j.percent()).desired_height(8.0).fill(vio_fill()));
                                }
                                _ => {
                                    ui.add(egui::Label::new(egui::RichText::new(source_line(e)).weak()).truncate());
                                }
                            }
                        });
                    });
                });
            })
            .response
            .interact(egui::Sense::click());
        if resp.clicked() {
            self.sel = i;
        }
    }

    fn pick_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter(
                t("Medien"),
                &[
                    "mp3", "wav", "flac", "ogg", "oga", "m4a", "aac", "opus", "wma", "jpg", "jpeg",
                    "png", "bmp", "gif", "tif", "tiff", "webp", "mp4", "mov", "mkv", "webm",
                    "avi", "wmv", "m4v", "flv", "ts", "mpg", "mpeg",
                ],
            )
            .pick_files()
        {
            self.suche_starten(files, false);
        }
    }
}

/// Umrahmter Hinweiskasten über der Liste.
fn banner(ui: &mut egui::Ui, rand: egui::Color32, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(thema::panel())
        .rounding(11.0)
        .stroke(egui::Stroke::new(1.0, rand))
        .inner_margin(egui::Margin::symmetric(13.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(8.0);
}

// ---------- Bausteine ----------

fn sub(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).color(thema::muted()));
    ui.add_space(6.0);
}

fn chip(ui: &mut egui::Ui, sel: bool, label: &str) -> egui::Response {
    let col = vio();
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .size(12.5)
                .color(if sel { col } else { thema::text() }),
        )
        .fill(if sel { vio_soft() } else { thema::panel() })
        .stroke(egui::Stroke::new(1.0, if sel { col } else { thema::border() })),
    )
}

fn status_badge(ui: &mut egui::Ui, e: &Entry) {
    let badge = format!(
        "{}{}",
        if e.kind == MediaKind::Video && e.target.kind() == MediaKind::Audio {
            tf("{format} (Ton)", &[("format", &e.target.label())])
        } else {
            e.target.label().to_string()
        },
        if e.target.kind() != MediaKind::Audio && e.aspect != Aspect::Original {
            format!(" · {}", e.aspect.label())
        } else {
            String::new()
        }
    );
    let msg = |j: &JobState| sperren(&j.msg).clone();
    match (e.zustand(), &e.job) {
        (Zustand::Fertig, Some(j)) => {
            let m = msg(j);
            let pk = sperren(&j.papierkorb).clone();
            let r = match pk {
                Papierkorb::Behalten(grund) => ui
                    .colored_label(stil::orange(), t("fertig · Original behalten"))
                    .on_hover_text(grund),
                Papierkorb::Verschoben => ui
                    .colored_label(green(), t("fertig · Original im Papierkorb")),
                Papierkorb::Nein => ui.colored_label(green(), t("fertig")),
            };
            let out = sperren(&j.out).clone();
            let hover = [m, out].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
            if !hover.is_empty() {
                r.on_hover_text(hover);
            }
        }
        (Zustand::Fehler, Some(j)) => {
            ui.colored_label(red(), t("Fehler")).on_hover_text(msg(j));
        }
        (Zustand::Laeuft, Some(j)) => {
            let m = msg(j);
            let pct = (j.percent() * 100.0) as u32;
            if m.is_empty() {
                ui.weak(format!("{pct} %"));
            } else {
                ui.weak(format!("{m} {pct} %"));
            }
        }
        (Zustand::Wartet, _) => {
            ui.weak(t("wartet"));
        }
        (Zustand::Angehalten, _) => {
            ui.weak(t("angehalten"));
        }
        (Zustand::SchonImZiel, _) => {
            ui.weak(t("schon im Zielformat"))
                .on_hover_text(t("Hat schon die Endung des Zielformats: wird nicht umgewandelt und nicht gelöscht."));
        }
        _ => {
            ui.label(egui::RichText::new(badge).color(vio()).strong().size(12.0));
        }
    }
}

fn source_line(e: &Entry) -> String {
    let from = e
        .input
        .extension()
        .map(|x| x.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    let s = match e.dims {
        Some((w, h)) => tf("von {from} · {w}×{h}", &[("from", &from), ("w", &w), ("h", &h)]),
        None => tf("von {from}", &[("from", &from)]),
    };
    if e.ordner.is_empty() {
        s
    } else {
        format!("{s} · {}", e.ordner)
    }
}

/// Deutsche Sprachnamen aus `model::language_name` (zum Übersetzen mit `t`).
const LANG_NAMES: &[&str] = &[
    "Deutsch", "Schweizerdeutsch", "Englisch", "Französisch", "Italienisch", "Spanisch",
    "Portugiesisch", "Niederländisch", "Russisch", "Japanisch", "Koreanisch", "Chinesisch",
    "Türkisch", "Polnisch",
];

/// Wie `AudioTrack::label`, aber in der gewählten Sprache
/// (z. B. «Deutsch · AC3 · 5.1 · Kommentar» bzw. «German · AC3 · 5.1 · Kommentar»).
fn track_label(tr: &AudioTrack) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(match tr.lang.as_deref() {
        Some(code) => {
            let de = model::language_name(code);
            match LANG_NAMES.iter().copied().find(|n| *n == de) {
                Some(n) => t(n).to_string(),
                None => de,
            }
        }
        None => tf("Spur {n}", &[("n", &(tr.index + 1))]),
    });
    if !tr.codec.is_empty() {
        parts.push(tr.codec.clone());
    }
    if !tr.channels.is_empty() {
        parts.push(tr.channels.clone());
    }
    if let Some(title) = &tr.title {
        parts.push(title.clone());
    }
    parts.join(" · ")
}

/// Zeichnet eine kleine Seitenverhältnis-Vorschau (Crop/Pad/Stretch).
fn draw_preview(ui: &mut egui::Ui, area: egui::Rect, dims: Option<(u32, u32)>, aspect: Aspect, fit: Fit) {
    let p = ui.painter();
    p.rect_filled(area, 10.0, thema::bg());
    let sr = match dims {
        Some((w, h)) if h > 0 => w as f32 / h as f32,
        _ => 16.0 / 9.0,
    };
    let inner = area.shrink(16.0);
    let src = fit_rect(inner, sr);

    match aspect.ratio() {
        None => {
            p.rect_filled(src, 4.0, vio_fill());
        }
        Some(tr) => {
            let tr = tr as f32;
            match fit {
                Fit::Crop => {
                    p.rect_filled(src, 4.0, vio_fill());
                    let crop = fit_rect_inside(src, tr);
                    // Ränder abdunkeln
                    let dim = egui::Color32::from_black_alpha(110);
                    for r in around(src, crop) {
                        p.rect_filled(r, 0.0, dim);
                    }
                    p.rect_stroke(crop, 2.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                }
                Fit::Pad => {
                    let canvas = fit_rect(inner, tr);
                    // Leere Ränder: dunkel, im dunklen Design etwas heller, damit sie sich abheben.
                    p.rect_filled(canvas, 4.0, thema::pick(egui::Color32::from_gray(40), egui::Color32::from_gray(64)));
                    let inside = fit_rect_inside(canvas, sr);
                    p.rect_filled(inside, 2.0, vio_fill());
                }
                Fit::Stretch => {
                    let stretched = fit_rect(inner, tr);
                    p.rect_filled(stretched, 4.0, vio_fill());
                }
            }
        }
    }
}

fn fit_rect(area: egui::Rect, ratio: f32) -> egui::Rect {
    let aw = area.width();
    let ah = area.height();
    let (w, h) = if aw / ah > ratio {
        (ah * ratio, ah)
    } else {
        (aw, aw / ratio)
    };
    egui::Rect::from_center_size(area.center(), egui::vec2(w, h))
}

fn fit_rect_inside(outer: egui::Rect, ratio: f32) -> egui::Rect {
    let ow = outer.width();
    let oh = outer.height();
    let (w, h) = if ow / oh > ratio {
        (oh * ratio, oh)
    } else {
        (ow, ow / ratio)
    };
    egui::Rect::from_center_size(outer.center(), egui::vec2(w, h))
}

/// Die (bis zu vier) Randflächen zwischen `outer` und dem zentrierten `inner`.
fn around(outer: egui::Rect, inner: egui::Rect) -> Vec<egui::Rect> {
    let mut v = Vec::new();
    if inner.top() > outer.top() {
        v.push(egui::Rect::from_min_max(outer.left_top(), egui::pos2(outer.right(), inner.top())));
    }
    if inner.bottom() < outer.bottom() {
        v.push(egui::Rect::from_min_max(egui::pos2(outer.left(), inner.bottom()), outer.right_bottom()));
    }
    if inner.left() > outer.left() {
        v.push(egui::Rect::from_min_max(egui::pos2(outer.left(), inner.top()), egui::pos2(inner.left(), inner.bottom())));
    }
    if inner.right() < outer.right() {
        v.push(egui::Rect::from_min_max(egui::pos2(inner.right(), inner.top()), egui::pos2(outer.right(), inner.bottom())));
    }
    v
}

fn default_out_dir() -> String {
    if let Ok(up) = std::env::var("USERPROFILE") {
        format!("{}\\Downloads\\Umgewandelt", up)
    } else if let Ok(home) = std::env::var("HOME") {
        format!("{}/Umgewandelt", home)
    } else {
        "Umgewandelt".into()
    }
}

fn short_dir(dir: &str) -> String {
    let parts: Vec<&str> = dir.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => "—".into(),
        1 => parts[0].to_string(),
        n => format!("{}\\{}", parts[n - 2], parts[n - 1]),
    }
}
