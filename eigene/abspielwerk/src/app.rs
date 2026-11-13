//! Oberfläche wie VLC: oben die Menüleiste (Medien, Wiedergabe, Audio, Video, Untertitel,
//! Werkzeuge, Ansicht, Hilfe), in der Mitte die Anzeige (Video, Bild oder Musik mit
//! Albumbild), rechts die Wiedergabeliste, unten die Bedienleiste (Suchleiste mit Zeit, darunter
//! die Knöpfe und die Lautstärke bis 125 %).
//!
//! Videos und Musik spielt libVLC (vlc.rs), Bilder lädt bild.rs im Hilfsthread. Die Leisten
//! stehen in `app/leisten.rs`, die Menüs in `app/menues.rs`, die Fenster (Effekte,
//! Medieninformationen, Einstellungen …) in `app/fenster.rs`.

mod fenster;
mod leisten;
mod menues;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use egui::{Color32, Pos2, Rect, Vec2};

use crate::befehl::{self, Aktion, Mods, Taste};
use crate::bild::{self, Geladen, Zweck};
use crate::medien::{self, Art, Zufall};
use crate::merken::{self, Positionen, Zuletzt};
use crate::sprache::{t, tf};
use crate::stil;
use crate::thema;
use crate::vlc::{self, meta, Player, Zustand};

const LOGO: [u8; 3] = [0xB5, 0x17, 0x9E];
/// So viele Bilder behält der Betrachter auf der Grafikkarte.
const BILD_CACHE: usize = 6;
/// Lautstärke wie in VLC: bis 125 %.
const MAX_LAUTSTAERKE: f32 = 125.0;
/// Schritt für Lautstärke-Tasten und Mausrad
const LAUT_SCHRITT: f32 = 5.0;
/// Bedienung im Vollbild ausblenden nach …
const VOLLBILD_AUSBLENDEN: Duration = Duration::from_millis(3000);
/// So lange steht die Frage «Fortsetzen ab …?»
const FRAGE_DAUER: Duration = Duration::from_secs(10);
/// Gleichzeitig im Hintergrund gelesene Medien (Dauer/Album für die Liste)
const PROBEN: usize = 3;

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
    let bekannt: [&'static str; 8] = [
        "Datei zu gross",
        "Bild ist leer",
        "Bild nicht lesbar",
        "Dateiname nicht lesbar",
        "Datei lässt sich nicht öffnen",
        "Wiedergabe startet nicht",
        "Netzwerkstreams gehen mit diesem libVLC nicht",
        "Adresse nicht lesbar",
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

/// Tastenkürzel für die Anzeige: «Strg+O» → «Ctrl+O» (Englisch) bzw. «⌘O» (macOS).
fn kurz_text(k: &str) -> String {
    if k.is_empty() {
        return String::new();
    }
    let en = crate::sprache::is_en();
    let mac = cfg!(target_os = "macos");
    let mut s = k.to_string();
    let ersatz: [(&str, &str, &str); 5] = [("Strg+", "Ctrl+", "⌘"), ("Umschalt+", "Shift+", "⇧"), ("Alt+", "Alt+", "⌥"), ("Leertaste", "Space", "Space"), ("Entf", "Del", "⌫")];
    for (de, e, m) in ersatz {
        s = s.replace(de, if mac { m } else if en { e } else { de });
    }
    if en {
        s = s.replace("Doppelklick", "double-click").replace("Mausrad", "mouse wheel");
    }
    s
}

pub(crate) struct Eintrag {
    pfad: PathBuf,
    /// Netzwerkstream statt Datei
    url: Option<String>,
    art: Art,
    name: String,
    /// aus den Angaben der Datei (leer = Dateiname)
    titel: String,
    album: String,
    dauer: Option<i64>,
    geprueft: bool,
}

impl Eintrag {
    fn neu(pfad: PathBuf) -> Option<Eintrag> {
        let art = medien::art_von(&pfad)?;
        let name = pfad.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pfad.display().to_string());
        Some(Eintrag { pfad, url: None, art, name, titel: String::new(), album: String::new(), dauer: None, geprueft: art == Art::Bild })
    }

    fn adresse(url: &str) -> Eintrag {
        let url = url.trim().to_string();
        let ohne = url.split(['?', '#']).next().unwrap_or("");
        let art = if medien::art_von(Path::new(ohne)) == Some(Art::Musik) { Art::Musik } else { Art::Video };
        let name = ohne.rsplit('/').find(|s| !s.is_empty()).map(|s| merken::url_dekodieren(s)).filter(|s| !s.contains("://") && !s.is_empty()).unwrap_or_else(|| url.clone());
        Eintrag { pfad: PathBuf::from(&url), url: Some(url), art, name, titel: String::new(), album: String::new(), dauer: None, geprueft: true }
    }

    /// Schlüssel für «Fortsetzen» und «Zuletzt geöffnet».
    fn schluessel(&self) -> String {
        self.url.clone().unwrap_or_else(|| self.pfad.to_string_lossy().into_owned())
    }

    fn anzeige(&self) -> &str {
        if self.titel.is_empty() {
            &self.name
        } else {
            &self.titel
        }
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

#[derive(Clone, Copy, PartialEq)]
enum Fortsetzen {
    Fragen,
    Immer,
    Nie,
}

struct Textur {
    pfad: PathBuf,
    tex: egui::TextureHandle,
    size: [usize; 2],
}

/// Audio-Equalizer (Werte in dB).
#[derive(Clone, PartialEq)]
struct Equalizer {
    an: bool,
    vorlage: Option<usize>,
    vor: f32,
    baender: Vec<f32>,
}

impl Equalizer {
    fn to_text(&self) -> String {
        let b: Vec<String> = self.baender.iter().map(|v| format!("{v:.1}")).collect();
        format!("{};{};{:.1};{}", self.an as u8, self.vorlage.map(|v| v as i64).unwrap_or(-1), self.vor, b.join(","))
    }

    fn from_text(s: &str) -> Equalizer {
        let mut e = Equalizer { an: false, vorlage: None, vor: 0.0, baender: Vec::new() };
        let mut it = s.split(';');
        e.an = it.next() == Some("1");
        e.vorlage = it.next().and_then(|v| v.parse::<usize>().ok()).filter(|v| *v < 64);
        e.vor = it.next().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(-20.0, 20.0)).unwrap_or(0.0);
        e.baender = it.next().unwrap_or("").split(',').filter_map(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(-20.0, 20.0)).take(32).collect();
        e
    }
}

/// Video-Anpassungen: Helligkeit, Kontrast, Sättigung, Farbton, Gamma.
#[derive(Clone, PartialEq)]
struct Anpassung {
    an: bool,
    werte: [f32; 5],
}

const ANPASSUNG_STD: [f32; 5] = [1.0, 1.0, 1.0, 0.0, 1.0];

impl Anpassung {
    fn to_text(&self) -> String {
        let w: Vec<String> = self.werte.iter().map(|v| format!("{v:.3}")).collect();
        format!("{};{}", self.an as u8, w.join(","))
    }

    fn from_text(s: &str) -> Anpassung {
        let (an, rest) = s.split_once(';').unwrap_or(("0", ""));
        let mut werte = ANPASSUNG_STD;
        for (k, v) in rest.split(',').enumerate().take(5) {
            if let (Some(w), Ok(x)) = (werte.get_mut(k), v.parse::<f32>()) {
                if x.is_finite() {
                    *w = x;
                }
            }
        }
        Anpassung { an: an == "1", werte }
    }
}

/// Offene Fenster
#[derive(Default)]
struct Fenster {
    effekte: bool,
    effekte_reiter: u8,
    info: bool,
    info_reiter: u8,
    einstellungen: bool,
    tasten: bool,
    ueber: bool,
    netzwerk: bool,
    zeit: bool,
}

/// Für «Medieninformationen» gelesene Angaben (jede Sekunde neu).
#[derive(Default)]
struct Info {
    meta: Vec<(&'static str, String)>,
    spuren: Vec<vlc::SpurInfo>,
    statistik: Option<vlc::Statistik>,
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
    /// Kurze Anzeige oben rechts im Bild (wie das OSD von VLC)
    osd: Option<(String, Instant)>,
    frage_papierkorb: Option<usize>,
    ordner: Option<PathBuf>,
    rng: Zufall,
    verlauf: Vec<usize>,
    scroll_zu: bool,
    max_tex: usize,
    /// Massstab «Einpassen» des aktuellen Bildes (1.0 = Originalgrösse).
    basis: f32,
    // --- wie VLC
    rest_zeit: bool,
    seiten: usize,
    zuschnitt: usize,
    deinterlace: usize,
    sar: Option<(u32, u32)>,
    sar_seit: Instant,
    eq: Equalizer,
    eq_vorlagen: Vec<String>,
    eq_frequenzen: Vec<f32>,
    anpassung: Anpassung,
    /// Effekte nach dem Start eines Mediums noch einmal setzen
    effekte_neu: bool,
    spu_verzoegerung: i64,
    fenster: Fenster,
    netz_url: String,
    zeit_eingabe: String,
    positionen: Positionen,
    fortsetzen: Fortsetzen,
    frage_fortsetzen: Option<(i64, Instant)>,
    auto_fortsetzen: Option<i64>,
    zuletzt: Zuletzt,
    minimal: bool,
    oben: bool,
    filter: String,
    auswahl: Option<usize>,
    liste_fokus: bool,
    ziehen: Option<usize>,
    ziehen_vor: Option<usize>,
    proben: Vec<vlc::Probe>,
    probe_ab: usize,
    snap_tx: Sender<Result<PathBuf, String>>,
    snap_rx: Receiver<Result<PathBuf, String>>,
    snap_ordner: Option<PathBuf>,
    /// Bedienung im Vollbild (für «Maus darüber»)
    overlay: Option<Rect>,
    info: Info,
    info_seit: Instant,
    laut_rest: f32,
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
        let lautstaerke = lies("lautstaerke").and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, MAX_LAUTSTAERKE)).unwrap_or(80.0);
        let stumm = lies("stumm").is_some_and(|v| v == "1");
        let wdh = match lies("wiederholen").as_deref() {
            Some("alle") => Wdh::Alle,
            Some("eins") => Wdh::Eins,
            _ => Wdh::Aus,
        };
        let zufall = lies("zufall").is_some_and(|v| v == "1");
        let diashow_s = lies("diashow").and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(1.0, 120.0)).unwrap_or(5.0);
        let liste_sichtbar = lies("liste").is_none_or(|v| v != "0");
        let tempo = lies("tempo").and_then(|v| v.parse::<f32>().ok()).filter(|v| befehl::TEMPI.contains(v)).unwrap_or(1.0);
        let ordner = lies("ordner").map(PathBuf::from).filter(|p| p.is_dir());
        let rest_zeit = lies("restzeit").is_some_and(|v| v == "1");
        let fortsetzen = match lies("fortsetzen").as_deref() {
            Some("immer") => Fortsetzen::Immer,
            Some("nie") => Fortsetzen::Nie,
            _ => Fortsetzen::Fragen,
        };
        let positionen = lies("positionen").map(|s| Positionen::from_text(&s)).unwrap_or_default();
        let zuletzt = lies("zuletzt").map(|s| Zuletzt::from_text(&s)).unwrap_or_default();
        let eq = lies("equalizer").map(|s| Equalizer::from_text(&s)).unwrap_or(Equalizer { an: false, vorlage: None, vor: 0.0, baender: Vec::new() });
        let anpassung = lies("anpassung").map(|s| Anpassung::from_text(&s)).unwrap_or(Anpassung { an: false, werte: ANPASSUNG_STD });
        let snap_ordner = lies("fotoordner").map(PathBuf::from).filter(|p| p.is_dir());

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
        let (snap_tx, snap_rx) = channel();
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
            osd: None,
            frage_papierkorb: None,
            ordner,
            rng: Zufall::neu(),
            verlauf: Vec::new(),
            scroll_zu: false,
            max_tex: 8192,
            basis: 1.0,
            rest_zeit,
            seiten: 0,
            zuschnitt: 0,
            deinterlace: 0,
            sar: None,
            sar_seit: jetzt,
            eq,
            eq_vorlagen: Vec::new(),
            eq_frequenzen: Vec::new(),
            anpassung,
            effekte_neu: false,
            spu_verzoegerung: 0,
            fenster: Fenster::default(),
            netz_url: String::new(),
            zeit_eingabe: String::new(),
            positionen,
            fortsetzen,
            frage_fortsetzen: None,
            auto_fortsetzen: None,
            zuletzt,
            minimal: false,
            oben: false,
            filter: String::new(),
            auswahl: None,
            liste_fokus: false,
            ziehen: None,
            ziehen_vor: None,
            proben: Vec::new(),
            probe_ab: 0,
            snap_tx,
            snap_rx,
            snap_ordner,
            overlay: None,
            info: Info::default(),
            info_seit: jetzt,
            laut_rest: 0.0,
        };
        if let Some(p) = start {
            app.oeffne_pfad(&cc.egui_ctx, p);
        }
        app
    }

    fn melden(&mut self, s: impl Into<String>) {
        self.meldung = Some((s.into(), Instant::now()));
    }

    /// Kurze Anzeige im Bild (Lautstärke, Tempo, Seitenverhältnis …).
    fn osd(&mut self, s: impl Into<String>) {
        self.osd = Some((s.into(), Instant::now()));
    }

    fn player(&self) -> Option<&Player> {
        match &self.engine {
            Engine::Bereit(p) => Some(p),
            _ => None,
        }
    }

    /// Player mit geladenem Medium (Video oder Musik wird gezeigt).
    fn medium_player(&self) -> Option<&Player> {
        match (&self.engine, self.aktuelle_art()) {
            (Engine::Bereit(p), Some(Art::Video | Art::Musik)) if p.hat_medium() => Some(p),
            _ => None,
        }
    }

    fn aktuelle_art(&self) -> Option<Art> {
        self.aktuell.and_then(|i| self.liste.get(i)).map(|e| e.art)
    }

    fn liste_geaendert(&mut self) {
        self.verlauf.clear();
        self.probe_ab = 0;
        self.auswahl = None;
        self.ziehen = None;
    }

    // ------------------------------------------------------------------ Öffnen und Liste

    /// Datei (füllt die Liste mit ihrem Ordner), Wiedergabeliste (M3U) oder Ordner öffnen.
    fn oeffne_pfad(&mut self, ctx: &egui::Context, p: PathBuf) {
        if p.is_dir() {
            self.ordner_laden(ctx, &p, None);
            return;
        }
        if ist_liste(&p) {
            self.liste_laden(ctx, &p);
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
                self.ordner_laden(ctx, &d, Some(p.as_path()));
            }
            None => {
                self.anhalten();
                self.liste = Eintrag::neu(p).into_iter().collect();
                self.aktuell = None;
                self.liste_geaendert();
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
                self.anhalten();
                self.ordner = Some(dir.to_path_buf());
                self.liste = v.into_iter().filter_map(Eintrag::neu).collect();
                self.aktuell = None;
                self.liste_geaendert();
                if self.liste.is_empty() {
                    self.melden(t("In diesem Ordner gibt es keine Videos, Musik oder Bilder."));
                    self.titel_setzen(ctx);
                    return;
                }
                let i = wahl.and_then(|w| self.liste.iter().position(|e| e.pfad == w)).unwrap_or(0);
                self.zeigen(ctx, i);
            }
            Err(e) => {
                self.melden(tf("Ordner nicht lesbar: {e}", &[("e", &e)]));
            }
        }
    }

    /// M3U/M3U8-Wiedergabeliste laden (ersetzt die Liste).
    fn liste_laden(&mut self, ctx: &egui::Context, p: &Path) {
        let text = match std::fs::metadata(p) {
            Ok(m) if m.len() > 32 * 1024 * 1024 => {
                self.melden(t("Datei zu gross"));
                return;
            }
            Ok(_) => match std::fs::read(p) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(e) => {
                    self.melden(tf("Wiedergabeliste nicht lesbar: {e}", &[("e", &e)]));
                    return;
                }
            },
            Err(e) => {
                self.melden(tf("Wiedergabeliste nicht lesbar: {e}", &[("e", &e)]));
                return;
            }
        };
        let basis = p.parent().map(Path::to_path_buf).unwrap_or_default();
        let neu: Vec<Eintrag> = merken::m3u_lesen(&text, &basis)
            .into_iter()
            .filter_map(|s| if merken::ist_url(&s) { Some(Eintrag::adresse(&s)) } else { Eintrag::neu(PathBuf::from(s)) })
            .take(medien::MAX_EINTRAEGE)
            .collect();
        if neu.is_empty() {
            self.melden(t("Keine Videos, Musik oder Bilder dabei."));
            return;
        }
        self.anhalten();
        self.liste = neu;
        self.aktuell = None;
        self.liste_geaendert();
        self.zeigen(ctx, 0);
    }

    /// Hineingezogene Dateien/Ordner an die Liste anhängen.
    fn hinzufuegen(&mut self, ctx: &egui::Context, pfade: Vec<PathBuf>) {
        if pfade.len() == 1 && pfade.first().is_some_and(|p| ist_liste(p)) {
            if let Some(p) = pfade.into_iter().next() {
                self.liste_laden(ctx, &p);
            }
            return;
        }
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

    fn dialog_dateien(&mut self, ctx: &egui::Context, mehrere: bool) {
        let mut alle: Vec<&str> = Vec::new();
        alle.extend_from_slice(medien::VIDEO);
        alle.extend_from_slice(medien::MUSIK);
        alle.extend_from_slice(medien::BILD);
        alle.extend_from_slice(LISTEN);
        let mut d = rfd::FileDialog::new()
            .set_title(if mehrere { t("Mehrere Dateien öffnen") } else { t("Datei öffnen") })
            .add_filter(t("Videos, Musik und Bilder"), alle.as_slice())
            .add_filter(t("Videos"), medien::VIDEO)
            .add_filter(t("Musik"), medien::MUSIK)
            .add_filter(t("Bilder"), medien::BILD)
            .add_filter(t("Wiedergabelisten"), LISTEN);
        if let Some(o) = &self.ordner {
            d = d.set_directory(o);
        }
        if !mehrere {
            if let Some(p) = d.pick_file() {
                self.oeffne_pfad(ctx, p);
            }
            return;
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
            let neu: Vec<Eintrag> = medien::aufloesen(&v).into_iter().filter_map(Eintrag::neu).collect();
            if neu.is_empty() {
                self.melden(t("Keine Videos, Musik oder Bilder dabei."));
                return;
            }
            self.anhalten();
            self.liste = neu;
            self.aktuell = None;
            self.liste_geaendert();
            self.zeigen(ctx, 0);
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

    /// Netzwerkstream an die Liste hängen und abspielen.
    fn adresse_oeffnen(&mut self, ctx: &egui::Context, url: &str) -> bool {
        let url = url.trim();
        if url.starts_with("file://") {
            if let Some(p) = merken::file_url_zu_pfad(url) {
                self.oeffne_pfad(ctx, PathBuf::from(p));
                return true;
            }
        }
        if !merken::ist_url(url) || url.contains(['\n', '\r', '\t']) {
            self.melden(t("Bitte eine Adresse wie https://… oder rtsp://… eingeben."));
            return false;
        }
        if self.liste.len() >= medien::MAX_EINTRAEGE {
            return false;
        }
        self.liste.push(Eintrag::adresse(url));
        let i = self.liste.len() - 1;
        self.zeigen(ctx, i);
        true
    }

    fn zuletzt_oeffnen(&mut self, ctx: &egui::Context, i: usize) {
        let Some(s) = self.zuletzt.v.get(i).cloned() else { return };
        if merken::ist_url(&s) {
            self.adresse_oeffnen(ctx, &s);
            return;
        }
        let p = PathBuf::from(&s);
        if p.exists() {
            self.oeffne_pfad(ctx, p);
        } else {
            self.melden(tf("{name} gibt es nicht mehr.", &[("name", &s)]));
            self.zuletzt.v.retain(|x| *x != s);
        }
    }

    fn liste_speichern(&mut self) {
        if self.liste.is_empty() {
            self.melden(t("Die Wiedergabeliste ist leer."));
            return;
        }
        let mut d = rfd::FileDialog::new()
            .set_title(t("Wiedergabeliste speichern"))
            .add_filter("M3U", &["m3u8", "m3u"])
            .add_filter("XSPF", &["xspf"])
            .set_file_name(t("Wiedergabeliste.m3u8"));
        if let Some(o) = &self.ordner {
            d = d.set_directory(o);
        }
        let Some(ziel) = d.save_file() else { return };
        let orte: Vec<String> = self.liste.iter().map(Eintrag::schluessel).collect();
        let v: Vec<merken::ListenEintrag> = self.liste.iter().zip(orte.iter()).map(|(e, ort)| merken::ListenEintrag { ort, titel: e.anzeige(), dauer_ms: e.dauer }).collect();
        let xspf = ziel.extension().is_some_and(|e| e.eq_ignore_ascii_case("xspf"));
        let text = if xspf { merken::xspf_schreiben(&v, t("Wiedergabeliste")) } else { merken::m3u_schreiben(&v) };
        match std::fs::write(&ziel, text) {
            Ok(()) => {
                let p = ziel.display().to_string();
                self.melden(tf("Wiedergabeliste gespeichert: {p}", &[("p", &p)]));
            }
            Err(e) => self.melden(tf("Nicht gespeichert: {e}", &[("e", &e)])),
        }
    }

    fn entfernen(&mut self, ctx: &egui::Context, i: usize) {
        if i >= self.liste.len() {
            return;
        }
        let war_aktuell = self.aktuell == Some(i);
        if war_aktuell {
            self.position_merken();
        }
        let pfad = self.liste.remove(i).pfad;
        self.bilder.retain(|b| b.pfad != pfad);
        self.verlauf.clear();
        self.probe_ab = 0;
        if let Some(a) = self.aktuell.filter(|a| *a > i) {
            self.aktuell = Some(a - 1);
        }
        self.auswahl = if self.liste.is_empty() { None } else { Some(i.min(self.liste.len() - 1)) };
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
        let Some(e) = self.liste.get(i) else { return };
        if e.url.is_some() {
            return;
        }
        let pfad = e.pfad.clone();
        if self.aktuell == Some(i) {
            // Unter Windows lässt sich eine geöffnete Datei nicht verschieben.
            self.anhalten();
        }
        match trash::delete(&pfad) {
            Ok(()) => self.entfernen(ctx, i),
            Err(e) => {
                self.melden(tf("Nicht in den Papierkorb verschoben: {e}", &[("e", &e)]));
                if self.aktuell == Some(i) {
                    self.zeigen(ctx, i);
                }
            }
        }
    }

    /// Position des laufenden Videos für «Fortsetzen» merken (oder vergessen).
    fn position_merken(&mut self) {
        let Some(e) = self.aktuell.and_then(|i| self.liste.get(i)) else { return };
        if e.art == Art::Bild || self.laenge <= 0 {
            return;
        }
        // Musik nur bei langen Stücken (Hörbücher, Sendungen), Videos ab 1 min.
        let min = if e.art == Art::Musik { 20 * 60_000 } else { 60_000 };
        let key = e.schluessel();
        let zeit = self.scrub.unwrap_or(self.zeit);
        if matches!(self.zustand, Zustand::Ende) {
            self.positionen.vergessen(&key);
        } else if matches!(self.zustand, Zustand::Spielt | Zustand::Pause | Zustand::Puffert) {
            self.positionen.merken(&key, zeit, self.laenge, min);
        }
    }

    fn anhalten(&mut self) {
        self.position_merken();
        if let Some(p) = self.player() {
            if p.hat_medium() {
                p.stop();
            }
        }
        self.video_tex = None;
        self.zustand = Zustand::Nichts;
        self.frage_fortsetzen = None;
        self.auto_fortsetzen = None;
    }

    fn titel_setzen(&self, ctx: &egui::Context) {
        let t = match self.aktuell.and_then(|i| self.liste.get(i)) {
            Some(e) => format!("{} – Abspielwerk", e.anzeige()),
            None => "Abspielwerk".to_string(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(t));
    }

    // ------------------------------------------------------------------ Anzeigen

    fn zeigen(&mut self, ctx: &egui::Context, i: usize) {
        let Some(e) = self.liste.get(i) else { return };
        let (pfad, url, art, key) = (e.pfad.clone(), e.url.clone(), e.art, e.schluessel());
        self.position_merken();
        if let Some(a) = self.aktuell.filter(|a| *a != i) {
            self.verlauf.push(a);
            if self.verlauf.len() > 500 {
                self.verlauf.remove(0);
            }
        }
        self.aktuell = Some(i);
        self.auswahl = Some(i);
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
        self.sar = None;
        self.spu_verzoegerung = 0;
        self.frage_fortsetzen = None;
        self.auto_fortsetzen = None;
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
                    Engine::Bereit(p) => {
                        let r = match &url {
                            Some(u) => p.oeffnen_adresse(u),
                            None => p.oeffnen(&pfad),
                        };
                        match r {
                            Ok(()) => {
                                p.tempo(tempo);
                                p.ton(vol, stumm);
                                None
                            }
                            Err(e) => Some(engine_msg(&e)),
                        }
                    }
                    Engine::Laedt(_) => {
                        self.wartet = true;
                        None
                    }
                    Engine::Fehlt(_) => Some(vlc_fehlt_text().to_string()),
                };
                self.meta_seit = Instant::now().checked_sub(Duration::from_secs(5)).unwrap_or_else(Instant::now);
                self.effekte_neu = true;
                self.zustand = Zustand::Oeffnet;
                match fehler {
                    Some(f) => self.melden(f),
                    None if !self.wartet => {
                        self.zuletzt.hinzufuegen(&key);
                        if let Some(ms) = self.positionen.holen(&key) {
                            match self.fortsetzen {
                                Fortsetzen::Fragen => self.frage_fortsetzen = Some((ms, Instant::now())),
                                Fortsetzen::Immer => self.auto_fortsetzen = Some(ms),
                                Fortsetzen::Nie => {}
                            }
                        }
                    }
                    None => {}
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
        // Auch bei «Eines wiederholen» (gleicher Eintrag) neu starten: libVLC steht am Ende.
        if let Some(i) = self.naechster(auto) {
            self.zeigen(ctx, i);
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
                        Zustand::Ende | Zustand::Fehler => true,
                        _ => {
                            p.play();
                            self.effekte_neu = true;
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
                    let i = self.auswahl.filter(|i| *i < self.liste.len()).unwrap_or(0);
                    self.zeigen(ctx, i);
                }
            }
        }
    }

    fn stopp(&mut self) {
        if self.aktuelle_art() == Some(Art::Bild) {
            self.diashow = false;
            return;
        }
        self.anhalten();
        self.zustand = Zustand::Gestoppt;
        self.zeit = 0;
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

    fn lautstaerke_aendern(&mut self, d: f32) {
        self.lautstaerke = (self.lautstaerke + d).clamp(0.0, MAX_LAUTSTAERKE).round();
        if d > 0.0 {
            self.stumm = false;
        }
        let v = self.lautstaerke;
        self.osd(tf("Lautstärke {v} %", &[("v", &v)]));
    }

    fn tempo_setzen(&mut self, r: f32) {
        if !(r.is_finite() && r > 0.0) {
            return;
        }
        self.tempo = r;
        if let Some(p) = self.medium_player() {
            p.tempo(r);
        }
        let s = befehl::tempo_text(r);
        self.osd(tf("Geschwindigkeit {s}", &[("s", &s)]));
    }

    /// Spuren-Wechsel (B, V): nächste Spur der Liste.
    fn spur_wechseln(&mut self, ton: bool) {
        let Some(p) = self.medium_player() else { return };
        let (spuren, aktiv) = if ton { (p.tonspuren(), p.tonspur()) } else { (p.untertitel_liste(), p.untertitel()) };
        if spuren.is_empty() {
            let s = if ton { t("Keine Tonspur") } else { t("Keine Untertitel") };
            self.osd(s);
            return;
        }
        let pos = spuren.iter().position(|(id, _)| *id == aktiv).unwrap_or(0);
        let Some((id, name)) = spuren.get(befehl::weiter_index(pos, spuren.len())).cloned() else { return };
        if ton {
            p.setze_tonspur(id);
        } else {
            p.setze_untertitel(id);
        }
        let name = spur_name(id, &name);
        let s = if ton { tf("Tonspur: {name}", &[("name", &name)]) } else { tf("Untertitel: {name}", &[("name", &name)]) };
        self.osd(s);
    }

    fn effekte_anwenden(&self) {
        let Some(p) = self.player() else { return };
        if p.kann_equalizer() {
            if self.eq.an && !self.eq.baender.is_empty() {
                p.equalizer(Some((self.eq.vor, self.eq.baender.as_slice())));
            } else {
                p.equalizer(None);
            }
        }
        if p.kann_anpassen() {
            p.anpassen(self.anpassung.an.then_some(self.anpassung.werte));
        }
        if p.kann_deinterlace() {
            p.deinterlace(befehl::DEINTERLACE.get(self.deinterlace).copied().filter(|m| !m.is_empty()));
        }
    }

    /// Bildschirmfoto des aktuellen Videobilds als PNG (im Hintergrund schreiben).
    fn schnappschuss(&mut self, ctx: &egui::Context) {
        let Some(foto) = self.medium_player().filter(|_| self.aktuelle_art() == Some(Art::Video)).and_then(|p| p.foto()) else {
            self.melden(t("Kein Videobild für ein Bildschirmfoto."));
            return;
        };
        let ordner = self.snap_ordner.clone().filter(|p| p.is_dir()).or_else(standard_bilder_ordner).unwrap_or_else(|| PathBuf::from("."));
        use chrono::{Datelike, Timelike};
        let n = chrono::Local::now();
        let name = merken::schnappschuss_name(n.year(), n.month(), n.day(), n.hour(), n.minute(), n.second());
        let ziel = merken::freier_pfad(&ordner, &name, &|p: &Path| p.exists());
        let tx = self.snap_tx.clone();
        let ctx = ctx.clone();
        let r = std::thread::Builder::new().name("foto".into()).spawn(move || {
            let (w, h) = (u32::try_from(foto.breite).unwrap_or(0), u32::try_from(foto.hoehe).unwrap_or(0));
            let erg = std::panic::catch_unwind(|| image::save_buffer_with_format(&ziel, &foto.rgb, w, h, image::ColorType::Rgb8, image::ImageFormat::Png))
                .map_err(|_| "PNG".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()))
                .map(|()| ziel);
            let _ = tx.send(erg);
            ctx.request_repaint();
        });
        if let Err(e) = r {
            self.melden(tf("Nicht gespeichert: {e}", &[("e", &e)]));
        }
    }

    fn ausfuehren(&mut self, ctx: &egui::Context, a: Aktion) {
        let bild = self.aktuelle_art() == Some(Art::Bild);
        match a {
            Aktion::Oeffnen => self.dialog_dateien(ctx, false),
            Aktion::MehrereOeffnen => self.dialog_dateien(ctx, true),
            Aktion::OrdnerOeffnen => self.dialog_ordner(ctx),
            Aktion::NetzwerkDialog => self.fenster.netzwerk = true,
            Aktion::ZuletztOeffnen(i) => self.zuletzt_oeffnen(ctx, i),
            Aktion::ZuletztLeeren => self.zuletzt.v.clear(),
            Aktion::ListeSpeichern => self.liste_speichern(),
            Aktion::Beenden => {
                self.position_merken();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Aktion::Zeigen(i) => {
                if bild && self.liste.get(i).is_some_and(|e| e.art != Art::Bild) {
                    self.diashow = false;
                }
                self.zeigen(ctx, i)
            }
            Aktion::Entfernen(i) => self.entfernen(ctx, i),
            Aktion::Leeren => {
                self.anhalten();
                self.liste.clear();
                self.aktuell = None;
                self.diashow = false;
                self.liste_geaendert();
                self.bilder.clear();
                self.titel_setzen(ctx);
            }
            Aktion::Verschieben(von, vor) => {
                if let Some(nach) = befehl::verschieben(&mut self.liste, von, vor) {
                    self.aktuell = self.aktuell.map(|i| befehl::index_nach(i, von, nach));
                    self.auswahl = Some(nach);
                    self.verlauf.clear();
                    self.probe_ab = 0;
                }
            }
            Aktion::PlayPause => self.play_pause(ctx),
            Aktion::Stopp => self.stopp(),
            Aktion::Zurueck => self.zurueck(ctx),
            Aktion::Weiter => self.weiter(ctx, false),
            Aktion::Spulen(d) => {
                if bild || self.medium_player().is_none() {
                    return;
                }
                let basis = self.zeit;
                let ziel = if self.laenge > 0 { basis.saturating_add(d).clamp(0, self.laenge) } else { basis.saturating_add(d).max(0) };
                if let Some(p) = self.player() {
                    p.setze_zeit(ziel);
                }
                self.zeit = ziel;
                let s = format!("{} / {}", medien::zeit_text(ziel), medien::zeit_text(self.laenge));
                self.osd(s);
            }
            Aktion::Springen(ms) => {
                if let Some(p) = self.medium_player() {
                    p.setze_zeit(ms);
                }
                self.zeit = ms.max(0);
            }
            Aktion::ZeitDialog => {
                if self.medium_player().is_some() {
                    self.zeit_eingabe = medien::zeit_text(self.zeit);
                    self.fenster.zeit = true;
                }
            }
            Aktion::Schneller => self.tempo_setzen(befehl::schneller(self.tempo)),
            Aktion::Langsamer => self.tempo_setzen(befehl::langsamer(self.tempo)),
            Aktion::TempoNormal => self.tempo_setzen(1.0),
            Aktion::Tempo(r) => self.tempo_setzen(r),
            Aktion::BildFuerBild => {
                if let Some(p) = self.medium_player().filter(|p| p.kann_bild_fuer_bild()) {
                    p.bild_fuer_bild();
                    self.osd(t("Bild für Bild"));
                }
            }
            Aktion::ZeitZeigen => {
                if self.medium_player().is_some() {
                    let s = format!("{} / {}", medien::zeit_text(self.zeit), medien::zeit_text(self.laenge));
                    self.osd(s);
                }
            }
            Aktion::Titel(i) => {
                if let Some(p) = self.medium_player() {
                    p.setze_titel(i);
                }
            }
            Aktion::Kapitel(i) => {
                if let Some(p) = self.medium_player() {
                    p.setze_kapitel(i);
                }
            }
            Aktion::WdhWechseln => {
                self.wdh = match self.wdh {
                    Wdh::Aus => Wdh::Alle,
                    Wdh::Alle => Wdh::Eins,
                    Wdh::Eins => Wdh::Aus,
                };
                let s = wdh_text(self.wdh);
                self.osd(s);
            }
            Aktion::Wdh(w) => {
                self.wdh = match w {
                    1 => Wdh::Alle,
                    2 => Wdh::Eins,
                    _ => Wdh::Aus,
                };
            }
            Aktion::ZufallUmschalten => {
                self.zufall = !self.zufall;
                let s = if self.zufall { t("Zufall: an") } else { t("Zufall: aus") };
                self.osd(s);
            }
            Aktion::Lauter => self.lautstaerke_aendern(LAUT_SCHRITT),
            Aktion::Leiser => self.lautstaerke_aendern(-LAUT_SCHRITT),
            Aktion::Stumm => {
                self.stumm = !self.stumm;
                let s = if self.stumm { t("Stumm") } else { t("Ton an") };
                self.osd(s);
            }
            Aktion::TonspurWechseln => self.spur_wechseln(true),
            Aktion::Tonspur(id) => {
                if let Some(p) = self.medium_player() {
                    p.setze_tonspur(id);
                }
            }
            Aktion::Audiogeraet(id) => {
                if let Some(p) = self.player() {
                    p.setze_audiogeraet(&id);
                }
            }
            Aktion::StereoModus(m) => {
                if let Some(p) = self.medium_player() {
                    p.setze_stereo(m);
                }
            }
            Aktion::Vollbild => {
                let an = !self.vollbild;
                self.vollbild_setzen(ctx, an);
            }
            Aktion::VollbildAus => {
                if self.vollbild {
                    self.vollbild_setzen(ctx, false);
                }
            }
            Aktion::Seitenverhaeltnis(i) => {
                self.seiten = i.min(befehl::SEITEN.len() - 1);
                let s = seiten_name(self.seiten);
                self.osd(tf("Seitenverhältnis: {s}", &[("s", &s)]));
            }
            Aktion::SeitenWechseln => self.ausfuehren(ctx, Aktion::Seitenverhaeltnis(befehl::weiter_index(self.seiten, befehl::SEITEN.len()))),
            Aktion::Zuschnitt(i) => {
                self.zuschnitt = i.min(befehl::ZUSCHNITTE.len() - 1);
                let s = zuschnitt_name(self.zuschnitt);
                self.osd(tf("Zuschneiden: {s}", &[("s", &s)]));
            }
            Aktion::ZuschnittWechseln => self.ausfuehren(ctx, Aktion::Zuschnitt(befehl::weiter_index(self.zuschnitt, befehl::ZUSCHNITTE.len()))),
            Aktion::Deinterlace(i) => {
                if self.player().is_some_and(|p| p.kann_deinterlace()) {
                    self.deinterlace = i.min(befehl::DEINTERLACE.len() - 1);
                    self.effekte_anwenden();
                    let s = deinterlace_name(self.deinterlace);
                    self.osd(tf("Deinterlace: {s}", &[("s", &s)]));
                }
            }
            Aktion::DeinterlaceWechseln => self.ausfuehren(ctx, Aktion::Deinterlace(befehl::weiter_index(self.deinterlace, befehl::DEINTERLACE.len()))),
            Aktion::Schnappschuss => self.schnappschuss(ctx),
            Aktion::Untertitel(id) => {
                if let Some(p) = self.medium_player() {
                    p.setze_untertitel(id);
                }
            }
            Aktion::UntertitelWechseln => self.spur_wechseln(false),
            Aktion::UntertitelDatei => self.untertitel_datei(),
            Aktion::UntertitelVerzoegerung(d) => {
                if let Some(p) = self.medium_player().filter(|p| p.kann_untertitel_verzoegerung()) {
                    let neu = self.spu_verzoegerung.saturating_add(d).clamp(-600_000, 600_000);
                    p.untertitel_verzoegerung(neu);
                    self.spu_verzoegerung = neu;
                    self.osd(tf("Untertitel-Verzögerung: {ms} ms", &[("ms", &neu)]));
                }
            }
            Aktion::UntertitelVerzoegerungNull => {
                if let Some(p) = self.medium_player().filter(|p| p.kann_untertitel_verzoegerung()) {
                    p.untertitel_verzoegerung(0);
                    self.spu_verzoegerung = 0;
                    self.osd(tf("Untertitel-Verzögerung: {ms} ms", &[("ms", &0)]));
                }
            }
            Aktion::Effekte => self.fenster.effekte = !self.fenster.effekte,
            Aktion::Medieninfo => {
                self.fenster.info = !self.fenster.info;
                self.info_seit = Instant::now().checked_sub(Duration::from_secs(5)).unwrap_or_else(Instant::now);
            }
            Aktion::Einstellungen => self.fenster.einstellungen = !self.fenster.einstellungen,
            Aktion::ListeUmschalten => self.liste_sichtbar = !self.liste_sichtbar,
            Aktion::Minimal => {
                self.minimal = !self.minimal;
                if self.minimal {
                    self.melden(t("Minimale Ansicht – zurück mit Strg+H oder Rechtsklick ins Bild."));
                }
            }
            Aktion::ImmerOben => {
                self.oben = !self.oben;
                let l = if self.oben { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(l));
            }
            Aktion::Tastenkuerzel => self.fenster.tasten = !self.fenster.tasten,
            Aktion::Ueber => self.fenster.ueber = !self.fenster.ueber,
            Aktion::Diashow => self.diashow_umschalten(),
            Aktion::Drehen(d) => {
                if bild {
                    self.drehung = (self.drehung + d) % 4
                }
            }
            Aktion::Einpassen => {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
            Aktion::Papierkorb => {
                if self.aktuell.and_then(|i| self.liste.get(i)).is_some_and(|e| e.url.is_none()) {
                    self.frage_papierkorb = self.aktuell;
                }
            }
            Aktion::EntfTaste => {
                if self.liste_fokus && self.liste_sichtbar {
                    if let Some(i) = self.auswahl.filter(|i| *i < self.liste.len()) {
                        self.entfernen(ctx, i);
                    }
                } else if bild {
                    self.ausfuehren(ctx, Aktion::Papierkorb);
                }
            }
        }
    }

    fn untertitel_datei(&mut self) {
        let Some(p) = self.medium_player().filter(|p| p.kann_untertitel_datei()) else { return };
        let mut d = rfd::FileDialog::new().set_title(t("Untertiteldatei hinzufügen")).add_filter(t("Untertitel"), &["srt", "ass", "ssa", "sub", "vtt", "idx", "smi", "txt"]);
        if let Some(dir) = self.aktuell.and_then(|i| self.liste.get(i)).filter(|e| e.url.is_none()).and_then(|e| e.pfad.parent().map(Path::to_path_buf)) {
            d = d.set_directory(dir);
        }
        let Some(datei) = d.pick_file() else { return };
        let uri = merken::pfad_zu_url(&datei.to_string_lossy());
        if p.untertitel_datei(&uri) {
            let name = datei.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.osd(tf("Untertitel: {name}", &[("name", &name)]));
        } else {
            self.melden(t("Die Untertiteldatei liess sich nicht laden."));
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
        if let Some(p) = self.player() {
            let vorlagen = p.equalizer_vorlagen();
            let freq = p.equalizer_baender();
            self.eq_vorlagen = vorlagen;
            if self.eq.baender.len() != freq.len() {
                self.eq.baender = vec![0.0; freq.len()];
            }
            self.eq_frequenzen = freq;
            self.effekte_anwenden();
        }
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
        // Fertige Bildschirmfotos
        while let Ok(r) = self.snap_rx.try_recv() {
            match r {
                Ok(p) => {
                    let p = p.display().to_string();
                    self.melden(tf("Bildschirmfoto gespeichert: {p}", &[("p", &p)]));
                }
                Err(e) => self.melden(tf("Nicht gespeichert: {e}", &[("e", &e)])),
            }
        }
    }

    /// Dauer, Titel und Album der Listeneinträge im Hintergrund lesen.
    fn proben_pruefen(&mut self, ctx: &egui::Context) {
        let mut fertig = Vec::new();
        let mut k = 0;
        while k < self.proben.len() {
            match self.proben.get(k).and_then(|p| p.fertig()) {
                Some(ok) => {
                    let p = self.proben.remove(k);
                    fertig.push((ok, p));
                }
                None => k += 1,
            }
        }
        for (ok, p) in fertig {
            let (dauer, titel, album) = if ok { (p.dauer(), p.meta(meta::TITEL), p.meta(meta::ALBUM)) } else { (None, None, None) };
            if let Some(e) = self.liste.iter_mut().find(|e| e.url.is_none() && e.pfad == p.pfad) {
                if e.dauer.is_none() {
                    e.dauer = dauer;
                }
                if let Some(ti) = titel.filter(|ti| *ti != e.name) {
                    if e.titel.is_empty() {
                        e.titel = ti;
                    }
                }
                if let Some(al) = album {
                    if e.album.is_empty() {
                        e.album = al;
                    }
                }
            }
        }
        let Engine::Bereit(player) = &self.engine else { return };
        while self.proben.len() < PROBEN {
            let Some(i) = (self.probe_ab..self.liste.len()).find(|&i| self.liste.get(i).is_some_and(|e| !e.geprueft)) else {
                self.probe_ab = self.liste.len();
                break;
            };
            self.probe_ab = i + 1;
            let Some(e) = self.liste.get_mut(i) else { break };
            e.geprueft = true;
            if let Some(p) = player.probe(&e.pfad) {
                self.proben.push(p);
            }
        }
        if !self.proben.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    fn player_pruefen(&mut self, ctx: &egui::Context) {
        if !matches!(self.aktuelle_art(), Some(Art::Video | Art::Musik)) {
            return;
        }
        let musik = self.aktuelle_art() == Some(Art::Musik);
        let meta_jetzt = self.meta_seit.elapsed() > Duration::from_millis(if musik { 800 } else { 2000 });
        let sar_jetzt = !musik && self.sar.is_none() && self.sar_seit.elapsed() > Duration::from_millis(700);
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
        let meta = meta_jetzt.then(|| (p.meta(meta::TITEL), p.meta(meta::KUENSTLER), p.meta(meta::ALBUM), p.meta(meta::BILD), p.meta(meta::LAEUFT)));
        let sar = if sar_jetzt && z == Zustand::Spielt { p.sar() } else { None };
        // Effekte nach dem Start setzen; «Fortsetzen» erst, wenn libVLC die Länge kennt.
        let effekte = self.effekte_neu && z == Zustand::Spielt;
        if let Some(ms) = self.auto_fortsetzen.filter(|_| z == Zustand::Spielt && laenge > 0) {
            p.setze_zeit(ms);
            self.auto_fortsetzen = None;
            let s = medien::zeit_text(ms);
            self.osd(tf("Fortgesetzt ab {t}", &[("t", &s)]));
        }
        if sar_jetzt {
            self.sar_seit = Instant::now();
        }
        if sar.is_some() {
            self.sar = sar;
        }
        if effekte {
            self.effekte_neu = false;
            self.effekte_anwenden();
        }
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
        if let Some(e) = self.aktuell.and_then(|i| self.liste.get_mut(i)) {
            if laenge > 0 {
                e.dauer = Some(laenge);
            }
        }
        if let Some((ti, ku, al, art, laeuft)) = meta {
            self.meta_seit = Instant::now();
            // Radiosender melden das laufende Stück unter «Läuft gerade».
            self.titel = laeuft.or(ti).unwrap_or_default();
            self.kuenstler = ku.unwrap_or_default();
            self.album = al.unwrap_or_default();
            if let Some(e) = self.aktuell.and_then(|i| self.liste.get_mut(i)) {
                if !self.titel.is_empty() && self.titel != e.name && e.titel != self.titel {
                    e.titel = self.titel.clone();
                    let t = format!("{} – Abspielwerk", e.titel);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title(t));
                }
                if !self.album.is_empty() {
                    e.album = self.album.clone();
                }
            }
            if musik {
                if let Some(pf) = art.as_deref().and_then(medien::url_zu_pfad).filter(|p| p.is_file()) {
                    if self.cover_pfad.as_ref() != Some(&pf) {
                        self.cover_pfad = Some(pf.clone());
                        self.cover = None;
                        bild::starten(pf, Zweck::Cover, 1024, self.tx.clone(), ctx.clone());
                    }
                }
            }
        }
        match z {
            Zustand::Ende if !self.ende_erledigt => {
                self.ende_erledigt = true;
                if let Some(e) = self.aktuell.and_then(|i| self.liste.get(i)) {
                    let key = e.schluessel();
                    self.positionen.vergessen(&key);
                }
                self.weiter(ctx, true);
            }
            Zustand::Fehler if !self.fehler_erledigt => {
                self.fehler_erledigt = true;
                let name = self.aktuell.and_then(|i| self.liste.get(i)).map(|e| e.anzeige().to_string()).unwrap_or_default();
                self.melden(tf("{name} lässt sich nicht abspielen.", &[("name", &name)]));
            }
            Zustand::Spielt | Zustand::Oeffnet | Zustand::Puffert => ctx.request_repaint_after(Duration::from_millis(250)),
            _ => {}
        }
        if musik {
            ctx.request_repaint_after(Duration::from_millis(900));
        }
    }

    /// Tasten wie in VLC (befehl::aus_taste); gehandhabte Tasten werden verbraucht, damit
    /// z. B. die Leertaste nicht zusätzlich einen Knopf auslöst.
    fn tasten(&mut self, ctx: &egui::Context) -> Vec<Aktion> {
        let mut a = Vec::new();
        if ctx.wants_keyboard_input() || self.frage_papierkorb.is_some() {
            return a;
        }
        let bild = self.aktuelle_art() == Some(Art::Bild);
        let evs: Vec<(egui::Key, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        let mut verbraucht = Vec::new();
        for (key, m) in evs {
            let Some(tk) = taste_von(key) else { continue };
            let mods = Mods { shift: m.shift, strg: m.command, alt: m.alt };
            if let Some(akt) = befehl::aus_taste(tk, mods, bild) {
                a.push(akt);
                verbraucht.push(key);
            }
        }
        if !verbraucht.is_empty() {
            ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key, pressed: true, .. } if verbraucht.contains(key))));
        }
        a
    }

    fn leisten_zeigen(&self) -> bool {
        if !self.vollbild {
            return true;
        }
        let spielt = matches!(self.zustand, Zustand::Spielt) || self.diashow;
        let ueber = match (self.overlay, self.maus_pos) {
            (Some(r), Some(p)) => r.expand(8.0).contains(p),
            _ => false,
        };
        !spielt || ueber || self.maus_seit.elapsed() < VOLLBILD_AUSBLENDEN
    }
}

/// Wiedergabelisten, die sich öffnen lassen
const LISTEN: &[&str] = &["m3u", "m3u8"];

fn ist_liste(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| LISTEN.iter().any(|l| e.eq_ignore_ascii_case(l)))
}

fn standard_bilder_ordner() -> Option<PathBuf> {
    let home = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") }.map(PathBuf::from);
    let ud = if cfg!(target_os = "linux") {
        let conf = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home.as_ref().map(|h| h.join(".config")));
        conf.and_then(|c| std::fs::read_to_string(c.join("user-dirs.dirs")).ok())
    } else {
        None
    };
    merken::bilder_ordner(home.as_deref(), ud.as_deref(), &|p: &Path| p.is_dir())
}

fn taste_von(k: egui::Key) -> Option<Taste> {
    use egui::Key;
    Some(match k {
        Key::Space => Taste::Leer,
        Key::ArrowLeft => Taste::Links,
        Key::ArrowRight => Taste::Rechts,
        Key::ArrowUp => Taste::Hoch,
        Key::ArrowDown => Taste::Runter,
        Key::Escape => Taste::Esc,
        Key::Delete => Taste::Entf,
        Key::Plus => Taste::Plus,
        Key::Minus => Taste::Minus,
        Key::Equals => Taste::Gleich,
        Key::F1 => Taste::F1,
        _ => {
            let n = k.name();
            let mut it = n.chars();
            let c = it.next()?;
            if it.next().is_some() || !c.is_ascii_alphabetic() {
                return None;
            }
            Taste::Buchstabe(c.to_ascii_uppercase())
        }
    })
}

fn spur_name(id: i32, name: &str) -> String {
    if id < 0 {
        t("Aus").to_string()
    } else if name.is_empty() {
        tf("Spur {n}", &[("n", &id)])
    } else {
        name.to_string()
    }
}

fn wdh_text(w: Wdh) -> &'static str {
    match w {
        Wdh::Aus => t("Nicht wiederholen"),
        Wdh::Alle => t("Ganze Liste wiederholen"),
        Wdh::Eins => t("Aktuelles Stück wiederholen"),
    }
}

fn seiten_name(i: usize) -> String {
    match befehl::SEITEN.get(i) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => t("Standard").to_string(),
    }
}

fn zuschnitt_name(i: usize) -> String {
    match befehl::ZUSCHNITTE.get(i) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => t("Standard").to_string(),
    }
}

fn deinterlace_name(i: usize) -> String {
    match befehl::DEINTERLACE.get(i) {
        Some(s) if !s.is_empty() => {
            let mut c = s.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
        _ => t("Aus").to_string(),
    }
}

/// Grösse `size` in `rect` einpassen (Seitenverhältnis bleibt), höchstens Massstab `max`.
fn einpassen(rect: Rect, size: [usize; 2], max: f32) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let s = (rect.width() / w).min(rect.height() / h).min(max);
    Rect::from_center_size(rect.center(), egui::vec2(w * s, h * s))
}

/// Rechteck mit Seitenverhältnis `r` (Breite/Höhe) in `rect` einpassen.
fn einpassen_verhaeltnis(rect: Rect, r: f64) -> Rect {
    let r = if r.is_finite() && r > 0.01 { r as f32 } else { 16.0 / 9.0 };
    let (mut w, mut h) = (rect.width(), rect.width() / r);
    if h > rect.height() {
        h = rect.height();
        w = h * r;
    }
    Rect::from_center_size(rect.center(), egui::vec2(w.max(1.0), h.max(1.0)))
}

fn mittig_text(ui: &egui::Ui, rect: Rect, text: &str, gr: f32, farbe: Color32) {
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
        self.proben_pruefen(ctx);

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
        if self.vollbild && zeigen {
            // Nach dem Ausblenden einmal neu zeichnen.
            ctx.request_repaint_after(VOLLBILD_AUSBLENDEN + Duration::from_millis(100));
        }
        if !self.vollbild && !self.minimal {
            self.menueleiste(ctx, &mut aktionen);
        }
        if !self.vollbild {
            self.overlay = None;
            let mut a = Vec::new();
            stil::fussleiste(ctx, |ui| self.fuss(ui, &mut a));
            aktionen.extend(a);
        }
        if self.liste_sichtbar && !self.vollbild && !self.minimal {
            self.liste_panel(ctx, &mut aktionen);
        }
        let mut a = Vec::new();
        if self.vollbild {
            egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::BLACK)).show(ctx, |ui| self.anzeige(ui, &mut a));
        } else {
            stil::mitte(ctx, |ui| self.anzeige(ui, &mut a));
        }
        aktionen.extend(a);
        if self.vollbild {
            if zeigen {
                self.vollbild_bedienung(ctx, &mut aktionen);
            } else {
                self.overlay = None;
                // Mauszeiger im Vollbild ausblenden, solange die Bedienung weg ist.
                ctx.set_cursor_icon(egui::CursorIcon::None);
            }
        }

        self.fenster_zeigen(ctx, &mut aktionen);

        for a in aktionen {
            self.ausfuehren(ctx, a);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.position_merken();
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
        storage.set_string("restzeit", if self.rest_zeit { "1" } else { "0" }.to_string());
        let f = match self.fortsetzen {
            Fortsetzen::Fragen => "fragen",
            Fortsetzen::Immer => "immer",
            Fortsetzen::Nie => "nie",
        };
        storage.set_string("fortsetzen", f.to_string());
        storage.set_string("positionen", self.positionen.to_text());
        storage.set_string("zuletzt", self.zuletzt.to_text());
        storage.set_string("equalizer", self.eq.to_text());
        storage.set_string("anpassung", self.anpassung.to_text());
        storage.set_string("fotoordner", self.snap_ordner.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}
