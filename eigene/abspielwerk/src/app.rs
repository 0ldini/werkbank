//! Oberfläche: oben die Kopfleiste, in der Mitte die Anzeige (Video, Bild oder Musik mit
//! Albumbild), rechts die Wiedergabeliste, unten die Bedienung (Transport oder Bildleiste).
//!
//! Videos und Musik spielt libVLC (vlc.rs), Bilder lädt bild.rs im Hilfsthread.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use egui::{Align, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

use crate::bild::{self, Geladen, Zweck};
use crate::medien::{self, Art, Zufall};
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::thema;
use crate::vlc::{self, Meta, Player, Zustand};

const LOGO: [u8; 3] = [0xB5, 0x17, 0x9E];
/// So viele Bilder behält der Betrachter auf der Grafikkarte.
const BILD_CACHE: usize = 6;
/// Spulen mit ← / →
const SPRUNG_MS: i64 = 5_000;
const TEMPI: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

fn tex_opt() -> egui::TextureOptions {
    egui::TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: Some(egui::TextureFilter::Linear),
    }
}

/// Meldungen aus bild.rs/vlc.rs übersetzen (diese Dateien bleiben ohne Sprachmodul).
fn engine_msg(s: &str) -> String {
    let bekannt: [&'static str; 6] = [
        "Datei zu gross",
        "Bild ist leer",
        "Bild nicht lesbar",
        "Dateiname nicht lesbar",
        "Datei lässt sich nicht öffnen",
        "Wiedergabe startet nicht",
    ];
    match bekannt.iter().copied().find(|de| *de == s) {
        Some(de) => t(de).to_string(),
        None => s.to_string(),
    }
}

fn vlc_fehlt_text() -> &'static str {
    if cfg!(target_os = "linux") {
        t("VLC fehlt – bitte installieren, z. B. sudo apt install vlc")
    } else {
        t("libVLC wurde nicht gefunden. Videos und Musik lassen sich nicht abspielen; Bilder gehen weiter.")
    }
}

struct Eintrag {
    pfad: PathBuf,
    art: Art,
    name: String,
}

impl Eintrag {
    fn neu(pfad: PathBuf) -> Option<Eintrag> {
        let art = medien::art_von(&pfad)?;
        let name = pfad.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pfad.display().to_string());
        Some(Eintrag { pfad, art, name })
    }
}

enum Engine {
    Laedt(Receiver<Result<vlc::Core, String>>),
    Bereit(Player),
    Fehlt(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Wdh {
    Aus,
    Alle,
    Eins,
}

struct Textur {
    pfad: PathBuf,
    tex: egui::TextureHandle,
    size: [usize; 2],
}

/// Was ein Klick in einer Leiste auslöst; ausgeführt nach dem Zeichnen.
enum Aktion {
    Oeffnen,
    OrdnerOeffnen,
    Zeigen(usize),
    Entfernen(usize),
    Zurueck,
    Weiter,
    PlayPause,
    Spulen(i64),
    Springen(i64),
    Vollbild,
    Diashow,
    Drehen(u8),
    Einpassen,
    Papierkorb,
    Leeren,
}

pub struct App {
    liste: Vec<Eintrag>,
    aktuell: Option<usize>,
    engine: Engine,
    /// Datei wartet darauf, dass libVLC fertig geladen ist.
    wartet: bool,
    video_tex: Option<egui::TextureHandle>,
    video_size: [usize; 2],
    bilder: VecDeque<Textur>,
    laedt: Vec<PathBuf>,
    bild_fehler: Option<(PathBuf, String)>,
    tx: Sender<Geladen>,
    rx: Receiver<Geladen>,
    cover: Option<Textur>,
    cover_pfad: Option<PathBuf>,
    titel: String,
    kuenstler: String,
    album: String,
    meta_seit: Instant,
    zoom: f32,
    pan: Vec2,
    drehung: u8,
    lautstaerke: f32,
    stumm: bool,
    tempo: f32,
    wdh: Wdh,
    zufall: bool,
    scrub: Option<i64>,
    ende_erledigt: bool,
    fehler_erledigt: bool,
    zeit: i64,
    laenge: i64,
    zustand: Zustand,
    diashow: bool,
    diashow_s: f32,
    diashow_seit: Instant,
    liste_sichtbar: bool,
    vollbild: bool,
    vollbild_seit: Instant,
    maus_seit: Instant,
    maus_pos: Option<Pos2>,
    meldung: Option<(String, Instant)>,
    frage_papierkorb: Option<usize>,
    ordner: Option<PathBuf>,
    rng: Zufall,
    verlauf: Vec<usize>,
    scroll_zu: bool,
    max_tex: usize,
    /// Massstab «Einpassen» des aktuellen Bildes (1.0 = Originalgrösse).
    basis: f32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>, kandidaten: Vec<vlc::Kandidat>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, LOGO);
        stil::init(&cc.egui_ctx, LOGO);

        let lies = |k: &str| cc.storage.and_then(|s| s.get_string(k));
        let lautstaerke = lies("lautstaerke").and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 100.0)).unwrap_or(80.0);
        let stumm = lies("stumm").is_some_and(|v| v == "1");
        let wdh = match lies("wiederholen").as_deref() {
            Some("alle") => Wdh::Alle,
            Some("eins") => Wdh::Eins,
            _ => Wdh::Aus,
        };
        let zufall = lies("zufall").is_some_and(|v| v == "1");
        let diashow_s = lies("diashow").and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(1.0, 120.0)).unwrap_or(5.0);
        let liste_sichtbar = lies("liste").is_none_or(|v| v != "0");
        let tempo = lies("tempo").and_then(|v| v.parse::<f32>().ok()).filter(|v| TEMPI.contains(v)).unwrap_or(1.0);
        let ordner = lies("ordner").map(PathBuf::from).filter(|p| p.is_dir());

        // libVLC im Hilfsthread laden: das Einlesen der Plugins dauert einen Moment.
        let (vtx, vrx) = channel();
        let ctx = cc.egui_ctx.clone();
        let engine = match std::thread::Builder::new().name("vlc".into()).spawn(move || {
            let r = vlc::laden(&kandidaten);
            let _ = vtx.send(r);
            ctx.request_repaint();
        }) {
            Ok(_) => Engine::Laedt(vrx),
            Err(e) => Engine::Fehlt(e.to_string()),
        };

        let (tx, rx) = channel();
        let jetzt = Instant::now();
        let mut app = App {
            liste: Vec::new(),
            aktuell: None,
            engine,
            wartet: false,
            video_tex: None,
            video_size: [0, 0],
            bilder: VecDeque::new(),
            laedt: Vec::new(),
            bild_fehler: None,
            tx,
            rx,
            cover: None,
            cover_pfad: None,
            titel: String::new(),
            kuenstler: String::new(),
            album: String::new(),
            meta_seit: jetzt,
            zoom: 1.0,
            pan: Vec2::ZERO,
            drehung: 0,
            lautstaerke,
            stumm,
            tempo,
            wdh,
            zufall,
            scrub: None,
            ende_erledigt: false,
            fehler_erledigt: false,
            zeit: 0,
            laenge: 0,
            zustand: Zustand::Nichts,
            diashow: false,
            diashow_s,
            diashow_seit: jetzt,
            liste_sichtbar,
            vollbild: false,
            vollbild_seit: jetzt,
            maus_seit: jetzt,
            maus_pos: None,
            meldung: None,
            frage_papierkorb: None,
            ordner,
            rng: Zufall::neu(),
            verlauf: Vec::new(),
            scroll_zu: false,
            max_tex: 8192,
            basis: 1.0,
        };
        if let Some(p) = start {
            app.oeffne_pfad(&cc.egui_ctx, p);
        }
        app
    }

    fn melden(&mut self, s: impl Into<String>) {
        self.meldung = Some((s.into(), Instant::now()));
    }

    fn player(&self) -> Option<&Player> {
        match &self.engine {
            Engine::Bereit(p) => Some(p),
            _ => None,
        }
    }

    fn aktuelle_art(&self) -> Option<Art> {
        self.aktuell.and_then(|i| self.liste.get(i)).map(|e| e.art)
    }

    // ------------------------------------------------------------------ Öffnen und Liste

    /// Datei (füllt die Liste mit ihrem Ordner) oder Ordner öffnen.
    fn oeffne_pfad(&mut self, ctx: &egui::Context, p: PathBuf) {
        if p.is_dir() {
            self.ordner_laden(ctx, &p, None);
            return;
        }
        if medien::art_von(&p).is_none() {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.melden(tf("{name}: Dateityp wird nicht unterstützt.", &[("name", &name)]));
            return;
        }
        match p.parent().filter(|d| !d.as_os_str().is_empty()) {
            Some(d) => {
                let d = d.to_path_buf();
                self.ordner_laden(ctx, &d, Some(&p));
            }
            None => {
                self.liste = Eintrag::neu(p).into_iter().collect();
                self.verlauf.clear();
                self.zeigen(ctx, 0);
            }
        }
    }

    fn ordner_laden(&mut self, ctx: &egui::Context, dir: &Path, wahl: Option<&Path>) {
        match medien::ordner_lesen(dir) {
            Ok(mut v) => {
                if let Some(w) = wahl {
                    if !v.iter().any(|p| p == w) {
                        v.insert(0, w.to_path_buf());
                    }
                }
                self.ordner = Some(dir.to_path_buf());
                self.liste = v.into_iter().filter_map(Eintrag::neu).collect();
                self.verlauf.clear();
                if self.liste.is_empty() {
                    self.anhalten();
                    self.aktuell = None;
                    self.melden(t("In diesem Ordner gibt es keine Videos, Musik oder Bilder."));
                    return;
                }
                let i = wahl.and_then(|w| self.liste.iter().position(|e| e.pfad == w)).unwrap_or(0);
                self.zeigen(ctx, i);
            }
            Err(e) => {
                let e = e.to_string();
                self.melden(tf("Ordner nicht lesbar: {e}", &[("e", &e)]));
            }
        }
    }

    /// Hineingezogene Dateien/Ordner an die Liste anhängen.
    fn hinzufuegen(&mut self, ctx: &egui::Context, pfade: Vec<PathBuf>) {
        let neu: Vec<Eintrag> = medien::aufloesen(&pfade).into_iter().filter_map(Eintrag::neu).collect();
        if neu.is_empty() {
            self.melden(t("Keine Videos, Musik oder Bilder dabei."));
            return;
        }
        let erste = self.liste.len();
        let platz = medien::MAX_EINTRAEGE.saturating_sub(self.liste.len());
        self.liste.extend(neu.into_iter().take(platz));
        if self.aktuell.is_none() && erste < self.liste.len() {
            self.zeigen(ctx, erste);
        }
    }

    fn dialog_dateien(&mut self, ctx: &egui::Context) {
        let mut alle: Vec<&str> = Vec::new();
        alle.extend_from_slice(medien::VIDEO);
        alle.extend_from_slice(medien::MUSIK);
        alle.extend_from_slice(medien::BILD);
        let mut d = rfd::FileDialog::new()
            .set_title(t("Dateien öffnen"))
            .add_filter(t("Videos, Musik und Bilder"), alle.as_slice())
            .add_filter(t("Videos"), medien::VIDEO)
            .add_filter(t("Musik"), medien::MUSIK)
            .add_filter(t("Bilder"), medien::BILD);
        if let Some(o) = &self.ordner {
            d = d.set_directory(o);
        }
        let Some(v) = d.pick_files() else { return };
        if v.len() == 1 {
            if let Some(p) = v.into_iter().next() {
                self.oeffne_pfad(ctx, p);
            }
        } else if !v.is_empty() {
            if let Some(d) = v.first().and_then(|p| p.parent()) {
                self.ordner = Some(d.to_path_buf());
            }
            self.liste = medien::aufloesen(&v).into_iter().filter_map(Eintrag::neu).collect();
            self.verlauf.clear();
            if !self.liste.is_empty() {
                self.zeigen(ctx, 0);
            }
        }
    }

    fn dialog_ordner(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(t("Ordner öffnen"));
        if let Some(o) = &self.ordner {
            d = d.set_directory(o);
        }
        if let Some(p) = d.pick_folder() {
            self.ordner_laden(ctx, &p, None);
        }
    }

    fn entfernen(&mut self, ctx: &egui::Context, i: usize) {
        if i >= self.liste.len() {
            return;
        }
        let war_aktuell = self.aktuell == Some(i);
        let pfad = self.liste.remove(i).pfad;
        self.bilder.retain(|b| b.pfad != pfad);
        self.verlauf.clear();
        match self.aktuell {
            Some(a) if a > i => self.aktuell = Some(a - 1),
            _ => {}
        }
        if war_aktuell {
            self.anhalten();
            self.aktuell = None;
            if !self.liste.is_empty() {
                self.zeigen(ctx, i.min(self.liste.len() - 1));
            } else {
                self.diashow = false;
                self.titel_setzen(ctx);
            }
        }
    }

    fn papierkorb(&mut self, ctx: &egui::Context, i: usize) {
        let Some(pfad) = self.liste.get(i).map(|e| e.pfad.clone()) else { return };
        if self.aktuell == Some(i) {
            // Unter Windows lässt sich eine geöffnete Datei nicht verschieben.
            self.anhalten();
        }
        match trash::delete(&pfad) {
            Ok(()) => self.entfernen(ctx, i),
            Err(e) => {
                let e = e.to_string();
                self.melden(tf("Nicht in den Papierkorb verschoben: {e}", &[("e", &e)]));
                if self.aktuell == Some(i) {
                    self.zeigen(ctx, i);
                }
            }
        }
    }

    fn anhalten(&mut self) {
        if let Some(p) = self.player() {
            if p.hat_medium() {
                p.stop();
            }
        }
        self.video_tex = None;
        self.zustand = Zustand::Nichts;
    }

    fn titel_setzen(&self, ctx: &egui::Context) {
        let t = match self.aktuell.and_then(|i| self.liste.get(i)) {
            Some(e) => format!("{} – Abspielwerk", e.name),
            None => "Abspielwerk".to_string(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(t));
    }

    // ------------------------------------------------------------------ Anzeigen

    fn zeigen(&mut self, ctx: &egui::Context, i: usize) {
        let Some(e) = self.liste.get(i) else { return };
        let (pfad, art) = (e.pfad.clone(), e.art);
        if let Some(a) = self.aktuell.filter(|a| *a != i) {
            self.verlauf.push(a);
            if self.verlauf.len() > 500 {
                self.verlauf.remove(0);
            }
        }
        self.aktuell = Some(i);
        self.scroll_zu = true;
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.drehung = 0;
        self.ende_erledigt = false;
        self.fehler_erledigt = false;
        self.zeit = 0;
        self.laenge = 0;
        self.scrub = None;
        self.titel.clear();
        self.kuenstler.clear();
        self.album.clear();
        self.cover = None;
        self.cover_pfad = None;
        self.video_tex = None;
        self.video_size = [0, 0];
        self.bild_fehler = None;
        self.wartet = false;
        self.titel_setzen(ctx);
        match art {
            Art::Bild => {
                self.anhalten();
                self.bild_anfordern(ctx, &pfad);
                self.diashow_seit = Instant::now();
            }
            Art::Video | Art::Musik => {
                self.diashow = false;
                let (tempo, vol, stumm) = (self.tempo, self.lautstaerke.round() as i32, self.stumm);
                let fehler = match &mut self.engine {
                    Engine::Bereit(p) => match p.oeffnen(&pfad) {
                        Ok(()) => {
                            p.tempo(tempo);
                            p.ton(vol, stumm);
                            None
                        }
                        Err(e) => Some(engine_msg(&e)),
                    },
                    Engine::Laedt(_) => {
                        self.wartet = true;
                        None
                    }
                    Engine::Fehlt(_) => Some(vlc_fehlt_text().to_string()),
                };
                self.meta_seit = Instant::now() - Duration::from_secs(5);
                if let Some(f) = fehler {
                    self.melden(f);
                }
            }
        }
    }

    fn bild_anfordern(&mut self, ctx: &egui::Context, pfad: &Path) {
        if self.bilder.iter().any(|b| b.pfad == pfad) || self.laedt.iter().any(|p| p == pfad) {
            return;
        }
        self.laedt.push(pfad.to_path_buf());
        bild::starten(pfad.to_path_buf(), Zweck::Bild, self.max_tex, self.tx.clone(), ctx.clone());
    }

    fn naechster(&mut self, auto: bool) -> Option<usize> {
        let n = self.liste.len();
        if n == 0 {
            return None;
        }
        let cur = self.aktuell;
        if auto && self.wdh == Wdh::Eins {
            return cur;
        }
        // Am Ende eines Stücks geht es nur mit Videos und Musik weiter; Bilder blättert man.
        let passt = |e: &Eintrag| !auto || e.art != Art::Bild;
        if self.zufall {
            let kand: Vec<usize> = (0..n).filter(|&i| Some(i) != cur && self.liste.get(i).is_some_and(passt)).collect();
            if kand.is_empty() {
                return if self.wdh == Wdh::Alle { cur } else { None };
            }
            let k = self.rng.bis(kand.len());
            return kand.get(k).copied();
        }
        let start = cur.map(|c| c + 1).unwrap_or(0);
        if let Some(i) = (start..n).find(|&i| self.liste.get(i).is_some_and(passt)) {
            return Some(i);
        }
        if !auto || self.wdh == Wdh::Alle {
            return (0..start.min(n)).find(|&i| self.liste.get(i).is_some_and(passt));
        }
        None
    }

    fn weiter(&mut self, ctx: &egui::Context, auto: bool) {
        match self.naechster(auto) {
            Some(i) => self.zeigen(ctx, i),
            None if auto => {
                // Ende der Liste: stehen bleiben.
            }
            None => {}
        }
    }

    fn zurueck(&mut self, ctx: &egui::Context) {
        let n = self.liste.len();
        if n == 0 {
            return;
        }
        if matches!(self.aktuelle_art(), Some(Art::Video | Art::Musik)) && self.zeit > 3_000 {
            if let Some(p) = self.player() {
                p.setze_zeit(0);
            }
            return;
        }
        if self.zufall {
            while let Some(v) = self.verlauf.pop() {
                if v < n {
                    self.zeigen(ctx, v);
                    // zeigen() legt das bisherige wieder in den Verlauf: entfernen.
                    self.verlauf.pop();
                    return;
                }
            }
        }
        let i = match self.aktuell {
            Some(0) | None => n - 1,
            Some(a) => a - 1,
        };
        self.zeigen(ctx, i.min(n - 1));
    }

    fn naechstes_bild(&self) -> Option<usize> {
        let n = self.liste.len();
        let start = self.aktuell.map(|a| a + 1).unwrap_or(0);
        (0..n).map(|k| (start + k) % n).find(|&i| self.liste.get(i).is_some_and(|e| e.art == Art::Bild))
    }

    fn play_pause(&mut self, ctx: &egui::Context) {
        match self.aktuelle_art() {
            Some(Art::Bild) => self.diashow_umschalten(),
            Some(_) => {
                if self.wartet {
                    return;
                }
                let z = self.zustand;
                let neu_starten = match self.player() {
                    Some(p) if p.hat_medium() => match z {
                        Zustand::Spielt | Zustand::Oeffnet | Zustand::Puffert => {
                            p.pause(true);
                            false
                        }
                        Zustand::Pause => {
                            p.pause(false);
                            false
                        }
                        _ => {
                            p.play();
                            false
                        }
                    },
                    Some(_) => true,
                    None => {
                        self.melden(vlc_fehlt_text());
                        false
                    }
                };
                if neu_starten {
                    if let Some(i) = self.aktuell {
                        self.zeigen(ctx, i);
                    }
                }
            }
            None => {
                if !self.liste.is_empty() {
                    self.zeigen(ctx, 0);
                }
            }
        }
    }

    fn diashow_umschalten(&mut self) {
        if self.diashow {
            self.diashow = false;
        } else if self.liste.iter().any(|e| e.art == Art::Bild) {
            self.diashow = true;
            self.diashow_seit = Instant::now();
        }
    }

    fn vollbild_setzen(&mut self, ctx: &egui::Context, an: bool) {
        self.vollbild = an;
        self.vollbild_seit = Instant::now();
        self.maus_seit = Instant::now();
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(an));
    }

    fn ausfuehren(&mut self, ctx: &egui::Context, a: Aktion) {
        match a {
            Aktion::Oeffnen => self.dialog_dateien(ctx),
            Aktion::OrdnerOeffnen => self.dialog_ordner(ctx),
            Aktion::Zeigen(i) => {
                if self.aktuelle_art() == Some(Art::Bild) && self.liste.get(i).is_some_and(|e| e.art != Art::Bild) {
                    self.diashow = false;
                }
                self.zeigen(ctx, i)
            }
            Aktion::Entfernen(i) => self.entfernen(ctx, i),
            Aktion::Zurueck => self.zurueck(ctx),
            Aktion::Weiter => self.weiter(ctx, false),
            Aktion::PlayPause => self.play_pause(ctx),
            Aktion::Spulen(d) => {
                let ziel = (self.zeit + d).clamp(0, self.laenge.max(0));
                if let Some(p) = self.player() {
                    p.setze_zeit(ziel);
                }
                self.zeit = ziel;
            }
            Aktion::Springen(ms) => {
                if let Some(p) = self.player() {
                    p.setze_zeit(ms);
                }
                self.zeit = ms;
            }
            Aktion::Vollbild => {
                let an = !self.vollbild;
                self.vollbild_setzen(ctx, an);
            }
            Aktion::Diashow => self.diashow_umschalten(),
            Aktion::Drehen(d) => self.drehung = (self.drehung + d) % 4,
            Aktion::Einpassen => {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
            Aktion::Papierkorb => self.frage_papierkorb = self.aktuell,
            Aktion::Leeren => {
                self.anhalten();
                self.liste.clear();
                self.aktuell = None;
                self.diashow = false;
                self.verlauf.clear();
                self.bilder.clear();
                self.titel_setzen(ctx);
            }
        }
    }

    // ------------------------------------------------------------------ Abfragen pro Bild

    fn engine_pruefen(&mut self, ctx: &egui::Context) {
        let Engine::Laedt(rx) = &self.engine else { return };
        let neu = match rx.try_recv() {
            Ok(Ok(core)) => match Player::neu(Rc::new(core), ctx.clone()) {
                Ok(p) => Engine::Bereit(p),
                Err(e) => Engine::Fehlt(e),
            },
            Ok(Err(e)) => Engine::Fehlt(e),
            Err(TryRecvError::Disconnected) => Engine::Fehlt(String::new()),
            Err(TryRecvError::Empty) => return,
        };
        if let Engine::Fehlt(e) = &neu {
            if !e.is_empty() {
                eprintln!("libVLC: {e}");
            }
        }
        self.engine = neu;
        if self.wartet {
            self.wartet = false;
            if let Some(i) = self.aktuell {
                self.zeigen(ctx, i);
            }
        }
    }

    fn bilder_pruefen(&mut self, ctx: &egui::Context) {
        while let Ok(g) = self.rx.try_recv() {
            self.laedt.retain(|p| p != &g.pfad);
            match (g.zweck, g.erg) {
                (Zweck::Bild, Ok(img)) => {
                    let size = img.size;
                    let tex = ctx.load_texture(g.pfad.display().to_string(), img, tex_opt());
                    self.bilder.push_back(Textur { pfad: g.pfad, tex, size });
                    let aktuell = self.aktuell.and_then(|i| self.liste.get(i)).map(|e| e.pfad.clone());
                    while self.bilder.len() > BILD_CACHE {
                        match self.bilder.iter().position(|b| Some(&b.pfad) != aktuell.as_ref()) {
                            Some(k) => {
                                self.bilder.remove(k);
                            }
                            None => break,
                        }
                    }
                }
                (Zweck::Bild, Err(e)) => self.bild_fehler = Some((g.pfad, e)),
                (Zweck::Cover, Ok(img)) => {
                    if self.cover_pfad.as_ref() == Some(&g.pfad) {
                        let size = img.size;
                        let tex = ctx.load_texture("cover", img, tex_opt());
                        self.cover = Some(Textur { pfad: g.pfad, tex, size });
                    }
                }
                (Zweck::Cover, Err(_)) => {}
            }
        }
        // Nächstes Bild vorladen, damit das Blättern flüssig geht.
        if self.laedt.is_empty() && self.aktuelle_art() == Some(Art::Bild) {
            let aktuell_da = self.aktuell.and_then(|i| self.liste.get(i)).is_some_and(|e| self.bilder.iter().any(|b| b.pfad == e.pfad));
            if aktuell_da {
                let n = self.liste.len();
                let a = self.aktuell.unwrap_or(0);
                for k in [a + 1, a + n.saturating_sub(1)] {
                    let i = k % n.max(1);
                    if let Some(e) = self.liste.get(i).filter(|e| e.art == Art::Bild) {
                        let p = e.pfad.clone();
                        if !self.bilder.iter().any(|b| b.pfad == p) {
                            self.bild_anfordern(ctx, &p);
                            break;
                        }
                    }
                }
            }
        }
    }

    fn player_pruefen(&mut self, ctx: &egui::Context) {
        if !matches!(self.aktuelle_art(), Some(Art::Video | Art::Musik)) {
            return;
        }
        let musik = self.aktuelle_art() == Some(Art::Musik);
        let meta_jetzt = musik && self.meta_seit.elapsed() > Duration::from_millis(800);
        let (vol, stumm) = (self.lautstaerke.round() as i32, self.stumm);
        let Some(p) = self.player() else { return };
        if !p.hat_medium() {
            return;
        }
        let z = p.zustand();
        let zeit = p.zeit();
        let laenge = p.laenge();
        if matches!(z, Zustand::Spielt | Zustand::Pause) {
            p.ton(vol, stumm);
        }
        let bild = p.neues_bild();
        let meta = meta_jetzt.then(|| (p.meta(Meta::Titel), p.meta(Meta::Kuenstler), p.meta(Meta::Album), p.meta(Meta::Bild)));
        self.zustand = z;
        if self.scrub.is_none() {
            self.zeit = zeit;
        }
        self.laenge = laenge;
        if let Some(img) = bild {
            self.video_size = img.size;
            match &mut self.video_tex {
                Some(tx) => tx.set(img, tex_opt()),
                None => self.video_tex = Some(ctx.load_texture("video", img, tex_opt())),
            }
        }
        if let Some((ti, ku, al, art)) = meta {
            self.meta_seit = Instant::now();
            self.titel = ti.unwrap_or_default();
            self.kuenstler = ku.unwrap_or_default();
            self.album = al.unwrap_or_default();
            if let Some(pf) = art.as_deref().and_then(medien::url_zu_pfad).filter(|p| p.is_file()) {
                if self.cover_pfad.as_ref() != Some(&pf) {
                    self.cover_pfad = Some(pf.clone());
                    self.cover = None;
                    bild::starten(pf, Zweck::Cover, 1024, self.tx.clone(), ctx.clone());
                }
            }
        }
        match z {
            Zustand::Ende if !self.ende_erledigt => {
                self.ende_erledigt = true;
                self.weiter(ctx, true);
            }
            Zustand::Fehler if !self.fehler_erledigt => {
                self.fehler_erledigt = true;
                let name = self.aktuell.and_then(|i| self.liste.get(i)).map(|e| e.name.clone()).unwrap_or_default();
                self.melden(tf("{name} lässt sich nicht abspielen.", &[("name", &name)]));
            }
            Zustand::Spielt | Zustand::Oeffnet | Zustand::Puffert => ctx.request_repaint_after(Duration::from_millis(250)),
            _ => {}
        }
        if musik {
            ctx.request_repaint_after(Duration::from_millis(900));
        }
    }

    fn tasten(&mut self, ctx: &egui::Context) -> Vec<Aktion> {
        let mut a = Vec::new();
        if ctx.wants_keyboard_input() || self.frage_papierkorb.is_some() {
            return a;
        }
        let k = |key: egui::Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
        let bild = self.aktuelle_art() == Some(Art::Bild);
        if k(egui::Key::Space) {
            a.push(Aktion::PlayPause);
        }
        if k(egui::Key::ArrowLeft) {
            a.push(if bild { Aktion::Zurueck } else { Aktion::Spulen(-SPRUNG_MS) });
        }
        if k(egui::Key::ArrowRight) {
            a.push(if bild { Aktion::Weiter } else { Aktion::Spulen(SPRUNG_MS) });
        }
        if k(egui::Key::ArrowUp) {
            self.lautstaerke = (self.lautstaerke + 5.0).min(100.0);
        }
        if k(egui::Key::ArrowDown) {
            self.lautstaerke = (self.lautstaerke - 5.0).max(0.0);
        }
        if k(egui::Key::F) {
            a.push(Aktion::Vollbild);
        }
        if k(egui::Key::Escape) && self.vollbild {
            self.vollbild_setzen(ctx, false);
        }
        if k(egui::Key::M) {
            self.stumm = !self.stumm;
        }
        if k(egui::Key::N) {
            a.push(Aktion::Weiter);
        }
        if k(egui::Key::P) {
            a.push(Aktion::Zurueck);
        }
        if k(egui::Key::Delete) && self.aktuell.is_some() {
            a.push(Aktion::Papierkorb);
        }
        a
    }

    // ------------------------------------------------------------------ Leisten

    fn kopf(&mut self, ui: &mut Ui) -> Option<Aktion> {
        let mut a = None;
        stil::titel(ui, "Abspielwerk");
        if stil::knopf(ui, Sym::Datei, t("Öffnen …")).clicked() {
            a = Some(Aktion::Oeffnen);
        }
        if stil::knopf(ui, Sym::OrdnerAuf, t("Ordner öffnen …")).clicked() {
            a = Some(Aktion::OrdnerOeffnen);
        }
        stil::trenner(ui);
        let hinweis = if self.liste_sichtbar { t("Wiedergabeliste ausblenden") } else { t("Wiedergabeliste einblenden") };
        if stil::leise(ui, true, Sym::Ebenen, t("Wiedergabeliste"), hinweis).clicked() {
            self.liste_sichtbar = !self.liste_sichtbar;
        }
        let bild = self.aktuelle_art() == Some(Art::Bild);
        let hat_bilder = self.liste.iter().any(|e| e.art == Art::Bild);
        let hat_aktuell = self.aktuell.is_some();
        let hat_liste = !self.liste.is_empty();
        let info = match &self.engine {
            Engine::Bereit(p) => format!("libVLC {}", p.version()),
            Engine::Laedt(_) => t("libVLC wird geladen …").to_string(),
            Engine::Fehlt(_) => vlc_fehlt_text().to_string(),
        };
        let diashow = self.diashow;
        let diashow_s = &mut self.diashow_s;
        stil::rechts(ui, |ui| {
            stil::menue(ui, true, Sym::Mehr, t("Mehr"), |ui| {
                let (s, txt) = if diashow { (Sym::Stopp, t("Diashow beenden")) } else { (Sym::Play, t("Diashow starten")) };
                if stil::eintrag(ui, hat_bilder, s, txt).clicked() {
                    a = Some(Aktion::Diashow);
                }
                ui.add_space(2.0);
                stil::feld(ui, t("Diashow: Sekunden pro Bild"), |ui| {
                    stil::wahl(ui, diashow_s, &[(2.0, "2"), (3.0, "3"), (5.0, "5"), (10.0, "10"), (20.0, "20")]);
                });
                ui.separator();
                if stil::eintrag(ui, bild, Sym::Rueckgaengig, t("Nach links drehen")).clicked() {
                    a = Some(Aktion::Drehen(3));
                }
                if stil::eintrag(ui, bild, Sym::Aktualisieren, t("Nach rechts drehen")).clicked() {
                    a = Some(Aktion::Drehen(1));
                }
                if stil::eintrag(ui, bild, Sym::Raster, t("Einpassen")).clicked() {
                    a = Some(Aktion::Einpassen);
                }
                ui.separator();
                if stil::eintrag(ui, hat_aktuell, Sym::Papierkorb, t("In Papierkorb …")).clicked() {
                    a = Some(Aktion::Papierkorb);
                }
                if stil::eintrag(ui, hat_liste, Sym::Kreuz, t("Liste leeren")).clicked() {
                    a = Some(Aktion::Leeren);
                }
                ui.separator();
                stil::hinweis(ui, &info);
            });
        });
        a
    }

    fn seite(&mut self, ui: &mut Ui) -> Option<Aktion> {
        let mut a = None;
        ui.horizontal(|ui| {
            stil::symbol(ui, Sym::Ebenen, 18.0, stil::akzent());
            ui.label(RichText::new(t("Wiedergabeliste")).size(14.5).strong());
            let n = self.liste.len();
            stil::rechts(ui, |ui| {
                stil::grau(ui, &n.to_string());
            });
        });
        if self.liste.is_empty() {
            stil::hinweis(ui, t("Dateien hineinziehen oder einen Ordner öffnen."));
            return a;
        }
        let w = ui.available_width();
        let alt = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, e) in self.liste.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), Sense::click());
            let an = self.aktuell == Some(i);
            if an && self.scroll_zu {
                resp.scroll_to_me(Some(Align::Center));
            }
            if ui.is_rect_visible(rect) {
                if an {
                    ui.painter().rect_filled(rect, 7.0, stil::toenung());
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 7.0, thema::hover());
                }
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(Align::Center)));
                c.spacing_mut().item_spacing.x = 8.0;
                let (txt, farbe) = match e.art {
                    Art::Video => (t("VIDEO"), stil::blau()),
                    Art::Musik => (t("MUSIK"), stil::violett()),
                    Art::Bild => (t("BILD"), stil::gruen()),
                };
                stil::marke(&mut c, txt, farbe, 52.0);
                let text = RichText::new(&e.name).size(13.0).color(if an { thema::text() } else { thema::muted() });
                c.add(egui::Label::new(if an { text.strong() } else { text }).truncate().selectable(false));
            }
            if resp.clicked() {
                a = Some(Aktion::Zeigen(i));
            }
            let resp = if resp.hovered() { resp.on_hover_text(e.pfad.display().to_string()) } else { resp };
            resp.context_menu(|ui| {
                if stil::eintrag(ui, true, Sym::Play, t("Anzeigen")).clicked() {
                    a = Some(Aktion::Zeigen(i));
                    ui.close_menu();
                }
                if stil::eintrag(ui, true, Sym::Kreuz, t("Aus der Liste nehmen")).clicked() {
                    a = Some(Aktion::Entfernen(i));
                    ui.close_menu();
                }
            });
        }
        ui.spacing_mut().item_spacing.y = alt;
        self.scroll_zu = false;
        a
    }

    fn suchleiste(&mut self, ui: &mut Ui, breite: f32) -> Option<i64> {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(breite.max(60.0), 22.0), Sense::click_and_drag());
        let len = self.laenge;
        let pos = self.scrub.unwrap_or(self.zeit);
        let frac = if len > 0 { (pos as f64 / len as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let p = ui.painter();
        let bahn = Rect::from_center_size(rect.center(), egui::vec2(rect.width(), 5.0));
        p.rect_filled(bahn, 3.0, thema::border());
        let voll = Rect::from_min_max(bahn.min, Pos2::new(bahn.left() + bahn.width() * frac, bahn.max.y));
        p.rect_filled(voll, 3.0, stil::farbe());
        let aktiv = resp.hovered() || resp.dragged();
        if len > 0 {
            p.circle(Pos2::new(voll.right(), bahn.center().y), if aktiv { 7.5 } else { 6.0 }, stil::farbe(), Stroke::new(2.0, thema::panel()));
        }
        if len <= 0 {
            return None;
        }
        let zeit_bei = |x: f32| (((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0) as f64 * len as f64) as i64;
        let mut ziel = None;
        if resp.dragged() {
            if let Some(pp) = resp.interact_pointer_pos() {
                self.scrub = Some(zeit_bei(pp.x));
            }
        }
        if resp.drag_stopped() {
            ziel = self.scrub.take();
        } else if resp.clicked() {
            ziel = resp.interact_pointer_pos().map(|pp| zeit_bei(pp.x));
        }
        if let Some(h) = resp.hover_pos() {
            let txt = medien::zeit_text(zeit_bei(h.x));
            resp.on_hover_text_at_pointer(txt);
        }
        ziel
    }

    /// Klappmenü, das sich nach oben öffnet (in der Fussleiste).
    fn menue_oben(ui: &mut Ui, enabled: bool, s: Sym, text: &str, add: impl FnOnce(&mut Ui)) {
        let resp = stil::knopf_an(ui, enabled, s, text);
        let id = resp.id.with("oben");
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        egui::popup_above_or_below_widget(ui, id, &resp, egui::AboveOrBelow::Above, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(220.0);
            add(ui);
        });
    }

    fn fuss(&mut self, ui: &mut Ui) -> Option<Aktion> {
        if self.aktuelle_art() == Some(Art::Bild) {
            return self.fuss_bild(ui);
        }
        let mut a = None;
        let medium = matches!(self.aktuelle_art(), Some(Art::Video | Art::Musik));
        // Zeile 1: Suchleiste und Zeit
        ui.horizontal(|ui| {
            let pos = self.scrub.unwrap_or(self.zeit);
            let txt = format!("{} / {}", medien::zeit_text(pos), medien::zeit_text(self.laenge));
            let w = ui.available_width() - 150.0;
            if let Some(z) = self.suchleiste(ui, w) {
                a = Some(Aktion::Springen(z));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(txt).monospace().size(13.0).color(thema::muted()));
        });
        ui.add_space(2.0);
        // Zeile 2: Knöpfe
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if stil::leise(ui, !self.liste.is_empty(), Sym::Anfang, "", t("Zurück (P)")).clicked() {
                a = Some(Aktion::Zurueck);
            }
            let spielt = matches!(self.zustand, Zustand::Spielt | Zustand::Oeffnet | Zustand::Puffert) && medium;
            if stil::haupt(ui, !self.liste.is_empty(), if spielt { Sym::Pause } else { Sym::Play }, "").on_hover_text(t("Abspielen / Pause (Leertaste)")).clicked() {
                a = Some(Aktion::PlayPause);
            }
            if stil::leise(ui, !self.liste.is_empty(), Sym::Ende, "", t("Weiter (N)")).clicked() {
                a = Some(Aktion::Weiter);
            }
            stil::trenner(ui);
            let stumm_hinweis = if self.stumm { t("Ton einschalten (M)") } else { t("Stumm (M)") };
            if stil::leise(ui, true, if self.stumm { Sym::Kreuz } else { Sym::Musik }, "", stumm_hinweis).clicked() {
                self.stumm = !self.stumm;
            }
            ui.allocate_ui(egui::vec2(170.0, 30.0), |ui| {
                stil::schieber(ui, &mut self.lautstaerke, 0.0..=100.0, "%", 0);
            });
            stil::trenner(ui);
            let player = match &self.engine {
                Engine::Bereit(p) if medium && p.hat_medium() => Some(p),
                _ => None,
            };
            Self::menue_oben(ui, player.is_some(), Sym::Musik, t("Tonspur"), |ui| {
                if let Some(p) = player {
                    let aktiv = p.tonspur();
                    let spuren = p.tonspuren();
                    if spuren.is_empty() {
                        stil::hinweis(ui, t("Keine Tonspur"));
                    }
                    for (id, name) in spuren {
                        let name = if id < 0 { t("Aus").to_string() } else if name.is_empty() { tf("Spur {n}", &[("n", &id)]) } else { name };
                        if stil::eintrag(ui, true, if id == aktiv { Sym::Haken } else { Sym::Musik }, &name).clicked() {
                            p.setze_tonspur(id);
                        }
                    }
                }
            });
            Self::menue_oben(ui, player.is_some(), Sym::Text, t("Untertitel"), |ui| {
                if let Some(p) = player {
                    let aktiv = p.untertitel();
                    let spuren = p.untertitel_liste();
                    if spuren.is_empty() {
                        stil::hinweis(ui, t("Keine Untertitel"));
                    }
                    for (id, name) in spuren {
                        let name = if id < 0 { t("Aus").to_string() } else if name.is_empty() { tf("Spur {n}", &[("n", &id)]) } else { name };
                        if stil::eintrag(ui, true, if id == aktiv { Sym::Haken } else { Sym::Text }, &name).clicked() {
                            p.setze_untertitel(id);
                        }
                    }
                }
            });
            let tempo_txt = format!("{}×", self.tempo);
            let mut tempo = self.tempo;
            Self::menue_oben(ui, true, Sym::Uhr, &tempo_txt, |ui| {
                stil::hinweis(ui, t("Wiedergabe-Geschwindigkeit"));
                for r in TEMPI {
                    if stil::eintrag(ui, true, if r == tempo { Sym::Haken } else { Sym::Uhr }, &format!("{r}×")).clicked() {
                        tempo = r;
                    }
                }
            });
            if tempo != self.tempo {
                self.tempo = tempo;
                if let Some(p) = player {
                    p.tempo(tempo);
                }
            }
            let (wdh_sym, wdh_txt) = match (self.zufall, self.wdh) {
                (true, _) => (Sym::Mischen, t("Zufall")),
                (false, Wdh::Alle) => (Sym::Aktualisieren, t("Alle wiederholen")),
                (false, Wdh::Eins) => (Sym::Aktualisieren, t("Eines wiederholen")),
                (false, Wdh::Aus) => (Sym::Pfeil, t("Der Reihe nach")),
            };
            let (wdh, zufall) = (&mut self.wdh, &mut self.zufall);
            Self::menue_oben(ui, true, wdh_sym, wdh_txt, |ui| {
                stil::hinweis(ui, t("Wiederholen"));
                for (w, txt) in [(Wdh::Aus, t("Nicht wiederholen")), (Wdh::Alle, t("Ganze Liste wiederholen")), (Wdh::Eins, t("Aktuelles Stück wiederholen"))] {
                    if stil::eintrag(ui, true, if *wdh == w { Sym::Haken } else { Sym::Aktualisieren }, txt).clicked() {
                        *wdh = w;
                    }
                }
                ui.separator();
                if stil::eintrag(ui, true, if *zufall { Sym::Haken } else { Sym::Mischen }, t("Zufällige Reihenfolge")).clicked() {
                    *zufall = !*zufall;
                }
            });
            stil::rechts(ui, |ui| {
                let h = if self.vollbild { t("Vollbild beenden (Esc)") } else { t("Vollbild (F)") };
                if stil::leise(ui, true, Sym::Panorama, "", h).clicked() {
                    a = Some(Aktion::Vollbild);
                }
                if let Some(e) = self.aktuell.and_then(|i| self.liste.get(i)) {
                    ui.add(egui::Label::new(RichText::new(&e.name).size(13.0).color(thema::muted())).truncate().selectable(false));
                }
            });
        });
        a
    }

    fn fuss_bild(&mut self, ui: &mut Ui) -> Option<Aktion> {
        let mut a = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if stil::leise(ui, true, Sym::Anfang, "", t("Vorheriges (←)")).clicked() {
                a = Some(Aktion::Zurueck);
            }
            let h = if self.diashow { t("Diashow anhalten (Leertaste)") } else { t("Diashow starten (Leertaste)") };
            if stil::haupt(ui, true, if self.diashow { Sym::Pause } else { Sym::Play }, "").on_hover_text(h).clicked() {
                a = Some(Aktion::Diashow);
            }
            if stil::leise(ui, true, Sym::Ende, "", t("Nächstes (→)")).clicked() {
                a = Some(Aktion::Weiter);
            }
            stil::trenner(ui);
            if stil::leise(ui, true, Sym::Rueckgaengig, "", t("Nach links drehen")).clicked() {
                a = Some(Aktion::Drehen(3));
            }
            if stil::leise(ui, true, Sym::Aktualisieren, "", t("Nach rechts drehen")).clicked() {
                a = Some(Aktion::Drehen(1));
            }
            if stil::knopf_text(ui, self.zoom != 1.0 || self.pan != Vec2::ZERO, t("Einpassen")).clicked() {
                a = Some(Aktion::Einpassen);
            }
            stil::grau(ui, &format!("{:.0} %", self.zoom_prozent()));
            stil::trenner(ui);
            if self.diashow {
                stil::grau(ui, &tf("Diashow: {s} s pro Bild", &[("s", &self.diashow_s)]));
            }
            stil::rechts(ui, |ui| {
                let h = if self.vollbild { t("Vollbild beenden (Esc)") } else { t("Vollbild (F)") };
                if stil::leise(ui, true, Sym::Panorama, "", h).clicked() {
                    a = Some(Aktion::Vollbild);
                }
                if stil::leise(ui, true, Sym::Papierkorb, "", t("In Papierkorb … (Entf)")).clicked() {
                    a = Some(Aktion::Papierkorb);
                }
                stil::trenner(ui);
                if let Some(e) = self.aktuell.and_then(|i| self.liste.get(i)) {
                    let groesse = self.bilder.iter().find(|b| b.pfad == e.pfad).map(|b| format!("  ·  {} × {}", b.size[0], b.size[1])).unwrap_or_default();
                    ui.add(egui::Label::new(RichText::new(format!("{}{}", e.name, groesse)).size(13.0).color(thema::muted())).truncate().selectable(false));
                }
            });
        });
        a
    }

    /// Zoom in Prozent der Originalgrösse (für die Anzeige), ungefähr aus dem letzten Bild.
    fn zoom_prozent(&self) -> f32 {
        (self.basis * self.zoom * 100.0).clamp(0.0, 100_000.0)
    }

    // ------------------------------------------------------------------ Anzeige

    fn anzeige(&mut self, ui: &mut Ui) -> Option<Aktion> {
        let mut a = None;
        let Some((pfad, art, name)) = self.aktuell.and_then(|i| self.liste.get(i)).map(|e| (e.pfad.clone(), e.art, e.name.clone())) else {
            stil::leer(ui, Sym::Film, t("Videos, Musik und Bilder"), t("Dateien hineinziehen oder oben öffnen."));
            ui.vertical_centered(|ui| {
                ui.add_space(10.0);
                match &self.engine {
                    Engine::Fehlt(_) => {
                        ui.label(RichText::new(vlc_fehlt_text()).color(stil::orange()));
                    }
                    Engine::Laedt(_) => {
                        stil::grau(ui, t("libVLC wird geladen …"));
                    }
                    Engine::Bereit(_) => {}
                }
            });
            let r = ui.max_rect();
            self.meldung_malen(ui, r);
            return a;
        };
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        if resp.double_clicked() {
            a = Some(Aktion::Vollbild);
        }
        let rund = if self.vollbild { 0.0 } else { 10.0 };
        match art {
            Art::Video => {
                ui.painter().rect_filled(rect, rund, Color32::BLACK);
                match &self.video_tex {
                    Some(tex) => {
                        let r = einpassen(rect, self.video_size, f32::INFINITY);
                        ui.painter().image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        let txt = match &self.engine {
                            Engine::Fehlt(_) => vlc_fehlt_text().to_string(),
                            _ if matches!(self.zustand, Zustand::Fehler) => tf("{name} lässt sich nicht abspielen.", &[("name", &name)]),
                            _ if matches!(self.zustand, Zustand::Spielt) => String::new(),
                            _ => t("Wird geöffnet …").to_string(),
                        };
                        mittig_text(ui, rect, &txt, 15.0, Color32::from_gray(200));
                    }
                }
            }
            Art::Musik => self.musik_malen(ui, rect, &name),
            Art::Bild => self.bild_malen(ui, rect, &resp, &pfad, &name),
        }
        if !ui.ctx().input(|i| i.raw.hovered_files.is_empty()) {
            ui.painter().rect(rect.shrink(4.0), 12.0, stil::mix(stil::farbe(), thema::panel(), 0.12), Stroke::new(2.0, stil::farbe()));
            mittig_text(ui, rect, t("Loslassen, um zur Wiedergabeliste hinzuzufügen"), 17.0, thema::text());
        }
        self.meldung_malen(ui, rect);
        a
    }

    fn musik_malen(&mut self, ui: &mut Ui, rect: Rect, name: &str) {
        let p = ui.painter();
        let seite = (rect.height() * 0.55).min(rect.width() * 0.6).clamp(80.0, 440.0);
        let block_h = seite + 96.0;
        let oben = rect.center().y - block_h / 2.0;
        let cover_r = Rect::from_min_size(Pos2::new(rect.center().x - seite / 2.0, oben), Vec2::splat(seite));
        if let Some(c) = &self.cover {
            p.image(c.tex.id(), einpassen(cover_r, c.size, f32::INFINITY), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        } else if let Some(tex) = &self.video_tex {
            // Manche Dateien liefern das Albumbild als Videobild.
            p.image(tex.id(), einpassen(cover_r, self.video_size, f32::INFINITY), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        } else {
            p.rect_filled(cover_r, 18.0, stil::toenung());
            stil::sym(p, Rect::from_center_size(cover_r.center(), Vec2::splat(seite * 0.42)), Sym::Musik, stil::akzent());
        }
        let titel = if self.titel.is_empty() { name.to_string() } else { self.titel.clone() };
        let breite = rect.width() - 40.0;
        let mut y = cover_r.bottom() + 18.0;
        for (text, gr, farbe, stark) in [(titel, 20.0, thema::text(), true), (self.kuenstler.clone(), 15.0, thema::text(), false), (self.album.clone(), 13.5, thema::muted(), false)] {
            if text.is_empty() {
                continue;
            }
            let font = egui::FontId::proportional(gr);
            let mut job = egui::text::LayoutJob::simple_singleline(text, font, farbe);
            job.wrap = egui::text::TextWrapping::truncate_at_width(breite.max(40.0));
            let g = ui.fonts(|f| f.layout_job(job));
            let pos = Pos2::new(rect.center().x - g.size().x / 2.0, y);
            if stark {
                // etwas kräftiger: doppelt mit Versatz zeichnen
                ui.painter().galley(pos + egui::vec2(0.4, 0.0), g.clone(), farbe);
            }
            ui.painter().galley(pos, g.clone(), farbe);
            y += g.size().y + 6.0;
        }
    }

    fn bild_malen(&mut self, ui: &mut Ui, rect: Rect, resp: &egui::Response, pfad: &Path, name: &str) {
        let Some(b) = self.bilder.iter().find(|b| b.pfad == pfad) else {
            match &self.bild_fehler {
                Some((p, e)) if p == pfad => {
                    let e = engine_msg(e);
                    mittig_text(ui, rect, &tf("{name} lässt sich nicht anzeigen: {e}", &[("name", &name), ("e", &e)]), 15.0, stil::rot());
                }
                _ => mittig_text(ui, rect, t("Bild wird geladen …"), 15.0, thema::muted()),
            }
            return;
        };
        let (tex, size) = (b.tex.id(), b.size);
        let gedreht = self.drehung % 2 == 1;
        let (w, h) = if gedreht { (size[1] as f32, size[0] as f32) } else { (size[0] as f32, size[1] as f32) };
        if w < 1.0 || h < 1.0 {
            return;
        }
        // Einpassen, kleine Bilder aber nicht über die Originalgrösse vergrössern.
        let basis = (rect.width() / w).min(rect.height() / h).min(1.0);
        self.basis = basis;
        // Zoom mit dem Mausrad um den Mauszeiger, Verschieben durch Ziehen.
        if resp.hovered() {
            let (scroll, zd) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let f = (scroll / 300.0).exp() * zd;
            if (f - 1.0).abs() > 1e-4 && f.is_finite() {
                let neu = (self.zoom * f).clamp(0.05, 40.0);
                let f = neu / self.zoom;
                if let Some(ptr) = resp.hover_pos() {
                    let rel = ptr - rect.center();
                    self.pan = (self.pan - rel) * f + rel;
                } else {
                    self.pan *= f;
                }
                self.zoom = neu;
            }
        }
        if resp.dragged() {
            self.pan += resp.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if resp.hovered() && self.zoom > 1.0 {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let s = basis * self.zoom;
        let r = Rect::from_center_size(rect.center() + self.pan, egui::vec2(w * s, h * s));
        let p = ui.painter().with_clip_rect(rect);
        let uv = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
        let ecken = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
        let k = (self.drehung % 4) as usize;
        let mut m = egui::Mesh::with_texture(tex);
        for (i, e) in ecken.iter().enumerate() {
            m.vertices.push(egui::epaint::Vertex { pos: *e, uv: uv[(i + 4 - k) % 4], color: Color32::WHITE });
        }
        m.indices = vec![0, 1, 2, 0, 2, 3];
        p.add(egui::Shape::mesh(m));
    }

    fn meldung_malen(&mut self, ui: &mut Ui, rect: Rect) {
        let Some((text, seit)) = &self.meldung else { return };
        let alter = seit.elapsed();
        if alter > Duration::from_secs(7) {
            self.meldung = None;
            return;
        }
        ui.ctx().request_repaint_after(Duration::from_secs(7) - alter);
        let breite = (rect.width() - 60.0).clamp(120.0, 640.0);
        let g = ui.painter().layout(text.clone(), egui::FontId::proportional(14.0), thema::text(), breite);
        let box_r = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 20.0 + g.size().y / 2.0 + 10.0), g.size() + egui::vec2(28.0, 20.0));
        ui.painter().rect(box_r, 10.0, thema::panel(), Stroke::new(1.0, stil::mix(stil::orange(), thema::border(), 0.6)));
        ui.painter().galley(box_r.min + egui::vec2(14.0, 10.0), g, thema::text());
    }

    fn leisten_zeigen(&self) -> bool {
        if !self.vollbild {
            return true;
        }
        let spielt = matches!(self.zustand, Zustand::Spielt) || self.diashow;
        !spielt || self.maus_seit.elapsed() < Duration::from_millis(2500)
    }
}

/// Grösse `size` in `rect` einpassen (Seitenverhältnis bleibt), höchstens Massstab `max`.
fn einpassen(rect: Rect, size: [usize; 2], max: f32) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let s = (rect.width() / w).min(rect.height() / h).min(max);
    Rect::from_center_size(rect.center(), egui::vec2(w * s, h * s))
}

fn mittig_text(ui: &Ui, rect: Rect, text: &str, gr: f32, farbe: Color32) {
    if text.is_empty() {
        return;
    }
    let g = ui.painter().layout(text.to_string(), egui::FontId::proportional(gr), farbe, (rect.width() - 40.0).max(60.0));
    ui.painter().galley(rect.center() - g.size() / 2.0, g, farbe);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.max_tex = ctx.input(|i| i.max_texture_side).clamp(1024, 16384);
        self.engine_pruefen(ctx);
        self.bilder_pruefen(ctx);
        self.player_pruefen(ctx);

        // Hineingezogene Dateien
        let fallen: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !fallen.is_empty() {
            self.hinzufuegen(ctx, fallen);
        }

        // Vollbild: Zustand des Fensters übernehmen (z. B. vom System beendet).
        if let Some(fs) = ctx.input(|i| i.viewport().fullscreen) {
            if fs != self.vollbild && self.vollbild_seit.elapsed() > Duration::from_millis(1200) {
                self.vollbild = fs;
            }
        }
        let pos = ctx.input(|i| i.pointer.latest_pos());
        if pos != self.maus_pos {
            self.maus_pos = pos;
            self.maus_seit = Instant::now();
        }

        let mut aktionen = self.tasten(ctx);

        // Diashow
        if self.diashow {
            if self.aktuelle_art() == Some(Art::Bild) {
                let dauer = Duration::from_secs_f32(self.diashow_s.clamp(1.0, 120.0));
                let el = self.diashow_seit.elapsed();
                if el >= dauer {
                    if let Some(i) = self.naechstes_bild() {
                        self.zeigen(ctx, i);
                    }
                    self.diashow_seit = Instant::now();
                } else {
                    ctx.request_repaint_after(dauer - el);
                }
            } else if let Some(i) = self.naechstes_bild() {
                self.zeigen(ctx, i);
            } else {
                self.diashow = false;
            }
        }

        let zeigen = self.leisten_zeigen();
        if self.vollbild && !zeigen {
            // Leisten ausgeblendet; bei Mausbewegung wieder zeigen.
        } else if self.vollbild {
            ctx.request_repaint_after(Duration::from_millis(2600));
        }
        if !self.vollbild {
            let mut a = None;
            stil::kopfleiste(ctx, |ui| a = self.kopf(ui));
            aktionen.extend(a);
        }
        if zeigen {
            let mut a = None;
            stil::fussleiste(ctx, |ui| a = self.fuss(ui));
            aktionen.extend(a);
        }
        if self.liste_sichtbar && !self.vollbild {
            let mut a = None;
            stil::seitenleiste(ctx, 300.0, |ui| a = self.seite(ui));
            aktionen.extend(a);
        }
        let mut a = None;
        let schwarz = self.vollbild || self.aktuelle_art() == Some(Art::Video);
        if schwarz && self.vollbild {
            egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::BLACK)).show(ctx, |ui| a = self.anzeige(ui));
        } else {
            stil::mitte(ctx, |ui| a = self.anzeige(ui));
        }
        aktionen.extend(a);

        // Rückfrage «In Papierkorb»
        if let Some(i) = self.frage_papierkorb {
            match self.liste.get(i) {
                None => self.frage_papierkorb = None,
                Some(e) => {
                    let name = e.name.clone();
                    let (mut ja, mut nein) = (false, false);
                    egui::Window::new(t("In Papierkorb verschieben?"))
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ctx, |ui| {
                            ui.label(tf("«{name}» in den Papierkorb verschieben?", &[("name", &name)]));
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if stil::gefahr(ui, true, Sym::Papierkorb, t("In Papierkorb")).clicked() {
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
                    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                        ja = true;
                    }
                    if ja {
                        self.frage_papierkorb = None;
                        self.papierkorb(ctx, i);
                    } else if nein {
                        self.frage_papierkorb = None;
                    }
                }
            }
        }

        for a in aktionen {
            self.ausfuehren(ctx, a);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("lautstaerke", format!("{:.0}", self.lautstaerke));
        storage.set_string("stumm", if self.stumm { "1" } else { "0" }.to_string());
        let w = match self.wdh {
            Wdh::Aus => "aus",
            Wdh::Alle => "alle",
            Wdh::Eins => "eins",
        };
        storage.set_string("wiederholen", w.to_string());
        storage.set_string("zufall", if self.zufall { "1" } else { "0" }.to_string());
        storage.set_string("diashow", format!("{}", self.diashow_s));
        storage.set_string("liste", if self.liste_sichtbar { "1" } else { "0" }.to_string());
        storage.set_string("tempo", format!("{}", self.tempo));
        if let Some(o) = &self.ordner {
            storage.set_string("ordner", o.display().to_string());
        }
    }
}
