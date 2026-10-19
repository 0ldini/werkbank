//! Oberfläche (egui): links die Sicherungsaufträge, rechts der gewählte Auftrag mit den
//! Reitern Einstellungen, Zeitplan, Protokoll und Wiederherstellen. Alles Langsame
//! (Lesen, Kopieren, Prüfen, Ziel suchen) läuft in Hintergrund-Threads.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::aufgabe;
use crate::auftrag::{
    self, Art, Auftrag, Ausloeser, Daten, Entfernte, Ergebnis, Fehler, FehlerArt, Protokoll, Quelle, Taetigkeit,
    VersionForm, Zeitplan, AUSSCHLUSS_VORSCHLAEGE, WOCHENTAGE,
};
use crate::lauf;
use crate::motor::{self, Fortschritt, Stand, StandEintrag, Vorschau};
use crate::plan;
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::store;
use crate::thema;

// ------------------------------------------------------------------ Farben

fn akzent() -> egui::Color32 {
    stil::akzent()
}
fn gruen() -> egui::Color32 {
    stil::gruen()
}
fn orange() -> egui::Color32 {
    stil::orange()
}
fn rot() -> egui::Color32 {
    stil::rot()
}

fn ergebnis_farbe(e: Ergebnis) -> egui::Color32 {
    match e {
        Ergebnis::Ok => gruen(),
        Ergebnis::MitFehlern | Ergebnis::Abgebrochen => orange(),
        Ergebnis::Fehlgeschlagen => rot(),
    }
}

fn karte() -> egui::Frame {
    egui::Frame::none()
        .fill(thema::panel())
        .stroke(egui::Stroke::new(1.0, thema::border()))
        .rounding(12.0)
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
}

fn grau(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).color(thema::muted()).small());
}

// ------------------------------------------------------------------ Zustand

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Einstellungen,
    Zeitplan,
    Protokoll,
    Wiederherstellen,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ton {
    Gut,
    Warnung,
    Fehler,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobArt {
    Sichern,
    Pruefen,
    Vorschau,
    Staende,
    Inhalt,
    Wiederherstellen,
}

enum JobErgebnis {
    Protokoll(Box<Protokoll>, Option<store::StoreFehler>),
    Vorschau(Box<Vorschau>),
    Staende(Result<Vec<Stand>, Fehler>),
    Inhalt(Stand, Result<Vec<StandEintrag>, Fehler>),
    Absturz,
}

struct Job {
    art: JobArt,
    auftrag_id: u64,
    name: String,
    fs: Arc<Fortschritt>,
    ergebnis: Arc<Mutex<Option<JobErgebnis>>>,
    start: Instant,
}

/// Eintrag im Wiederherstellen-Baum.
#[derive(Clone)]
struct Knoten {
    rel: String,
    name: String,
    ordner: bool,
    groesse: u64,
    zeit: i64,
}

#[derive(Default)]
struct Wh {
    auftrag_id: Option<u64>,
    staende: Vec<Stand>,
    gewaehlt: Option<Stand>,
    inhalt: Vec<StandEintrag>,
    kinder: HashMap<String, Vec<Knoten>>,
    pfad: String,
    auswahl: BTreeSet<String>,
    filter: String,
    fehler: Option<String>,
    geladen: bool,
}

impl Wh {
    fn zuruecksetzen(&mut self, id: Option<u64>) {
        *self = Wh { auftrag_id: id, ..Default::default() };
    }

    fn index_bauen(&mut self) {
        let mut kinder: HashMap<String, Vec<Knoten>> = HashMap::new();
        let mut ordner: HashSet<String> = HashSet::new();
        let eltern = |rel: &str| -> (String, String) {
            match rel.rfind('/') {
                Some(p) => (rel[..p].to_string(), rel[p + 1..].to_string()),
                None => (String::new(), rel.to_string()),
            }
        };
        for e in &self.inhalt {
            // Alle Elternordner sind Ordner, auch wenn sie nicht eigens aufgeführt sind.
            let mut p = e.rel.as_str();
            while let Some(i) = p.rfind('/') {
                p = &p[..i];
                ordner.insert(p.to_string());
            }
            if e.ordner {
                ordner.insert(e.rel.clone());
            } else {
                let (el, name) = eltern(&e.rel);
                kinder.entry(el).or_default().push(Knoten { rel: e.rel.clone(), name, ordner: false, groesse: e.groesse, zeit: e.zeit });
            }
        }
        for o in &ordner {
            let (el, name) = eltern(o);
            kinder.entry(el).or_default().push(Knoten { rel: o.clone(), name, ordner: true, groesse: 0, zeit: 0 });
        }
        for v in kinder.values_mut() {
            v.sort_by(|a, b| b.ordner.cmp(&a.ordner).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        }
        self.kinder = kinder;
    }

    /// Ist `rel` selbst oder über einen Ordner darüber gewählt? (Some(true) = über Ordner)
    fn gewaehlt(&self, rel: &str) -> Option<bool> {
        if self.auswahl.contains(rel) {
            return Some(false);
        }
        let mut p = rel;
        while let Some(i) = p.rfind('/') {
            p = &p[..i];
            if self.auswahl.contains(p) {
                return Some(true);
            }
        }
        None
    }

    fn umschalten(&mut self, rel: &str) {
        if self.auswahl.remove(rel) {
            return;
        }
        let pre = format!("{rel}/");
        self.auswahl.retain(|s| !s.starts_with(&pre));
        self.auswahl.insert(rel.to_string());
    }
}

pub struct App {
    daten: Daten,
    sel: Option<u64>,
    entwurf: Option<Auftrag>,
    ausschluss_text: String,
    neuer_pfad: String,
    tab: Tab,
    meldung: Option<(String, Ton)>,
    job: Option<Job>,
    vorschau: Option<(u64, Vorschau)>,
    vorschau_liste: u8,
    wechsel: Option<Option<u64>>,
    /// Nach dem Speichern/Verwerfen im Wechsel-Dialog einen neuen Auftrag anlegen.
    neuer_nach_wechsel: bool,
    loeschen_frage: Option<u64>,
    schliessen_frage: bool,
    schliessen_nach_job: bool,
    darf_schliessen: bool,
    wh: Wh,
    // Anschluss: Ziele im Hintergrund prüfen.
    anschluss_pruefung: Option<Arc<Mutex<Option<Vec<(u64, bool)>>>>>,
    anschluss_zuletzt: Option<Instant>,
    anschluss_versucht: HashSet<(u64, String)>,
    anschluss_warteschlange: Vec<u64>,
    zuletzt_geladen: Instant,
    ctx: egui::Context,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0x3D, 0x5A, 0x80]);
        stil::init(&cc.egui_ctx, [0x3D, 0x5A, 0x80]);
        let mut app = App {
            daten: Daten::default(),
            sel: None,
            entwurf: None,
            ausschluss_text: String::new(),
            neuer_pfad: String::new(),
            tab: Tab::Einstellungen,
            meldung: None,
            job: None,
            vorschau: None,
            vorschau_liste: 0,
            wechsel: None,
            neuer_nach_wechsel: false,
            loeschen_frage: None,
            schliessen_frage: false,
            schliessen_nach_job: false,
            darf_schliessen: false,
            wh: Wh::default(),
            anschluss_pruefung: None,
            anschluss_zuletzt: None,
            anschluss_versucht: HashSet::new(),
            anschluss_warteschlange: Vec::new(),
            zuletzt_geladen: Instant::now(),
            ctx: cc.egui_ctx.clone(),
        };
        match store::laden() {
            Ok((d, hinweis)) => {
                app.daten = d;
                if let Some(store::Hinweis::Defekt { aus_bak, .. }) = hinweis {
                    let text = if aus_bak {
                        t("Die Datendatei war beschädigt – die letzte gute Fassung wurde geladen.")
                    } else {
                        t("Die Datendatei war beschädigt und wird beim nächsten Speichern beiseitegelegt.")
                    };
                    app.melden(text, Ton::Warnung);
                }
            }
            Err(e) => app.melden(&format!("{} {}", t(e.text), e.detail), Ton::Fehler),
        }
        let erster = app.daten.auftraege.first().map(|a| a.id);
        app.waehlen(erster);
        app
    }

    fn melden(&mut self, text: &str, ton: Ton) {
        self.meldung = Some((text.to_string(), ton));
    }

    fn store_fehler(&mut self, e: &store::StoreFehler) {
        let s = if e.detail.is_empty() { t(e.text).to_string() } else { format!("{}: {}", t(e.text), e.detail) };
        self.melden(&s, Ton::Fehler);
    }

    fn gespeichert(&self) -> Option<&Auftrag> {
        self.sel.and_then(|id| self.daten.auftrag(id))
    }

    fn dirty(&self) -> bool {
        match (&self.entwurf, self.gespeichert()) {
            (Some(e), Some(a)) => e != a,
            _ => false,
        }
    }

    fn waehlen(&mut self, id: Option<u64>) {
        self.sel = id;
        self.entwurf = self.gespeichert().cloned();
        self.ausschluss_text = self.entwurf.as_ref().map(|e| e.ausschluesse.join("\n")).unwrap_or_default();
        self.neuer_pfad.clear();
        self.vorschau = None;
        if self.wh.auftrag_id != id {
            self.wh.zuruecksetzen(id);
        }
    }

    fn neu_laden(&mut self) {
        let war_dirty = self.dirty();
        match store::laden() {
            Ok((d, _)) => self.daten = d,
            Err(e) => {
                self.store_fehler(&e);
                return;
            }
        }
        self.zuletzt_geladen = Instant::now();
        if self.sel.is_some() && self.gespeichert().is_none() {
            let erster = self.daten.auftraege.first().map(|a| a.id);
            self.waehlen(erster);
        } else if !war_dirty {
            self.entwurf = self.gespeichert().cloned();
            self.ausschluss_text = self.entwurf.as_ref().map(|e| e.ausschluesse.join("\n")).unwrap_or_default();
        }
    }

    // -------------------------------------------------------------- Speichern

    fn speichern(&mut self) -> bool {
        let Some(mut e) = self.entwurf.clone() else { return false };
        e.name = e.name.trim().to_string();
        e.ziel = e.ziel.trim().to_string();
        let andere: Vec<&str> = self.daten.auftraege.iter().filter(|a| a.id != e.id).map(|a| a.name.as_str()).collect();
        if let Err(m) = auftrag::name_pruefen(&e.name, &andere) {
            self.melden(t(m), Ton::Fehler);
            return false;
        }
        let alt = self.daten.auftrag(e.id).cloned();
        let mut hinweis: Option<String> = None;
        if let Some(alt) = &alt {
            if !alt.gleicher_kern(&e) {
                e.vorschau_gesehen = false;
            }
            // Eingetragene Windows-Aufgabe nachführen.
            if cfg!(windows) && alt.aufgabe_eingerichtet && !alt.zeitplan_gleich(&e) {
                let _ = aufgabe::entfernen(&alt.name);
                if alt.zeitplan == Zeitplan::BeimAnschliessen && e.zeitplan != Zeitplan::BeimAnschliessen {
                    let _ = aufgabe::anschluss_aufraeumen(&self.daten.auftraege, e.id);
                }
                if e.zeitplan == Zeitplan::Keiner {
                    e.aufgabe_eingerichtet = false;
                } else {
                    match aufgabe::einrichten(&e) {
                        Ok(()) => e.aufgabe_eingerichtet = true,
                        Err(x) => {
                            e.aufgabe_eingerichtet = false;
                            hinweis = Some(x);
                        }
                    }
                }
            }
        }
        let e2 = e.clone();
        match store::aendern(move |d| {
            if let Some(a) = d.auftraege.iter_mut().find(|a| a.id == e2.id) {
                *a = e2;
            } else {
                d.auftraege.push(e2);
            }
        }) {
            Ok((d, _, _)) => {
                self.daten = d;
                self.entwurf = Some(e);
                match hinweis {
                    Some(x) => self.melden(&format!("{} {x}", t("Gespeichert, aber die Aufgabenplanung meldet:")), Ton::Warnung),
                    None => self.melden(t("Gespeichert."), Ton::Gut),
                }
                true
            }
            Err(err) => {
                self.store_fehler(&err);
                false
            }
        }
    }

    fn neuer_auftrag(&mut self) {
        let r = store::aendern(|d| {
            let id = d.neue_id();
            let namen: Vec<String> = d.auftraege.iter().map(|a| a.name.to_lowercase()).collect();
            let basis = t("Neuer Auftrag");
            let mut name = basis.to_string();
            let mut i = 2;
            while namen.contains(&name.to_lowercase()) {
                name = format!("{basis} ({i})");
                i += 1;
            }
            d.auftraege.push(Auftrag { id, name, ..Default::default() });
            id
        });
        match r {
            Ok((d, id, _)) => {
                self.daten = d;
                self.waehlen(Some(id));
                self.tab = Tab::Einstellungen;
            }
            Err(e) => self.store_fehler(&e),
        }
    }

    fn auftrag_loeschen(&mut self, id: u64) {
        if let Some(a) = self.daten.auftrag(id).cloned() {
            if cfg!(windows) && a.aufgabe_eingerichtet {
                let _ = aufgabe::entfernen(&a.name);
                if a.zeitplan == Zeitplan::BeimAnschliessen {
                    let _ = aufgabe::anschluss_aufraeumen(&self.daten.auftraege, id);
                }
            }
        }
        match store::aendern(|d| d.auftraege.retain(|a| a.id != id)) {
            Ok((d, _, _)) => {
                self.daten = d;
                let erster = self.daten.auftraege.first().map(|a| a.id);
                self.waehlen(erster);
                self.melden(t("Auftrag gelöscht. Die Sicherungen im Ziel bleiben erhalten."), Ton::Gut);
            }
            Err(e) => self.store_fehler(&e),
        }
    }

    fn vorschau_gesehen_setzen(&mut self, id: u64) {
        match store::aendern(|d| {
            if let Some(a) = d.auftraege.iter_mut().find(|a| a.id == id) {
                a.vorschau_gesehen = true;
            }
        }) {
            Ok((d, _, _)) => {
                let war_dirty = self.dirty();
                self.daten = d;
                if !war_dirty {
                    self.entwurf = self.gespeichert().cloned();
                } else if let Some(e) = self.entwurf.as_mut() {
                    e.vorschau_gesehen = true;
                }
            }
            Err(e) => self.store_fehler(&e),
        }
    }

    // -------------------------------------------------------------- Hintergrund

    fn job_starten(&mut self, art: JobArt, a: &Auftrag, f: impl FnOnce(&Fortschritt) -> JobErgebnis + Send + 'static) {
        let fs_ = Fortschritt::neu();
        let ergebnis = Arc::new(Mutex::new(None));
        let (fs2, erg2, ctx) = (fs_.clone(), ergebnis.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&*fs2))).unwrap_or(JobErgebnis::Absturz);
            match erg2.lock() {
                Ok(mut g) => *g = Some(r),
                Err(p) => *p.into_inner() = Some(r),
            }
            ctx.request_repaint();
        });
        self.job = Some(Job { art, auftrag_id: a.id, name: a.name.clone(), fs: fs_, ergebnis, start: Instant::now() });
    }

    fn sichern_starten(&mut self, a: Auftrag, ausl: Ausloeser) {
        if self.job.is_some() {
            return;
        }
        self.melden(&tf("Sichere «{name}» …", &[("name", &a.name)]), Ton::Gut);
        self.job_starten(JobArt::Sichern, &a.clone(), move |fs_| {
            let (p, e) = lauf::ausfuehren(&a, Taetigkeit::Sichern, ausl, fs_);
            JobErgebnis::Protokoll(Box::new(p), e)
        });
    }

    fn pruefen_starten(&mut self, a: Auftrag) {
        if self.job.is_some() {
            return;
        }
        self.job_starten(JobArt::Pruefen, &a.clone(), move |fs_| {
            let (p, e) = lauf::ausfuehren(&a, Taetigkeit::Pruefen, Ausloeser::Hand, fs_);
            JobErgebnis::Protokoll(Box::new(p), e)
        });
    }

    fn vorschau_starten(&mut self, a: Auftrag) {
        if self.job.is_some() {
            return;
        }
        self.vorschau = None;
        self.job_starten(JobArt::Vorschau, &a.clone(), move |fs_| {
            JobErgebnis::Vorschau(Box::new(motor::vorschau(&a, &lauf::umgebung(), fs_)))
        });
    }

    fn staende_laden(&mut self, a: Auftrag) {
        if self.job.is_some() {
            return;
        }
        self.wh.zuruecksetzen(Some(a.id));
        self.wh.geladen = true;
        self.job_starten(JobArt::Staende, &a.clone(), move |_| JobErgebnis::Staende(motor::staende(&a)));
    }

    fn inhalt_laden(&mut self, a: Auftrag, s: Stand) {
        if self.job.is_some() {
            return;
        }
        self.wh.gewaehlt = Some(s.clone());
        self.wh.inhalt.clear();
        self.wh.kinder.clear();
        self.wh.auswahl.clear();
        self.wh.pfad.clear();
        self.wh.fehler = None;
        self.job_starten(JobArt::Inhalt, &a.clone(), move |fs_| {
            let r = motor::stand_inhalt(&a, &s, fs_);
            JobErgebnis::Inhalt(s, r)
        });
    }

    fn wiederherstellen_starten(&mut self, a: Auftrag, s: Stand, auswahl: Vec<String>, ziel: PathBuf) {
        if self.job.is_some() {
            return;
        }
        self.job_starten(JobArt::Wiederherstellen, &a.clone(), move |fs_| {
            let (p, e) = lauf::wiederherstellen(&a, &s, &auswahl, &ziel, fs_);
            JobErgebnis::Protokoll(Box::new(p), e)
        });
    }

    fn job_abfragen(&mut self) {
        let fertig = match &self.job {
            Some(j) => match j.ergebnis.lock() {
                Ok(mut g) => g.take(),
                Err(p) => p.into_inner().take(),
            },
            None => return,
        };
        let Some(r) = fertig else { return };
        let Some(job) = self.job.take() else { return };
        match r {
            JobErgebnis::Absturz => self.melden(t("Interner Fehler – der Vorgang wurde abgebrochen."), Ton::Fehler),
            JobErgebnis::Vorschau(v) => {
                self.vorschau_liste = 0;
                self.vorschau = Some((job.auftrag_id, *v));
            }
            JobErgebnis::Staende(r) => match r {
                Ok(st) => {
                    self.wh.staende = st;
                    self.wh.fehler = None;
                    if let (Some(s), Some(a)) = (self.wh.staende.first().cloned(), self.gespeichert().cloned()) {
                        if a.id == job.auftrag_id {
                            self.inhalt_laden(a, s);
                        }
                    }
                }
                Err(f) => self.wh.fehler = Some(fehler_text(&f)),
            },
            JobErgebnis::Inhalt(s, r) => {
                if self.wh.gewaehlt.as_ref() == Some(&s) {
                    match r {
                        Ok(inh) => {
                            self.wh.inhalt = inh;
                            self.wh.index_bauen();
                        }
                        Err(f) => self.wh.fehler = Some(fehler_text(&f)),
                    }
                }
            }
            JobErgebnis::Protokoll(p, store_fehler) => {
                let text = protokoll_kurz(&p);
                let ton = match p.ergebnis {
                    Ergebnis::Ok => Ton::Gut,
                    Ergebnis::MitFehlern | Ergebnis::Abgebrochen => Ton::Warnung,
                    Ergebnis::Fehlgeschlagen => Ton::Fehler,
                };
                let erster_fehler = p.fehler.iter().find(|f| f.art != FehlerArt::Hinweis).map(fehler_text);
                match (ton, erster_fehler) {
                    (Ton::Gut, _) | (_, None) => self.melden(&text, ton),
                    (_, Some(f)) => self.melden(&format!("{text} – {f}"), ton),
                }
                if let Some(e) = store_fehler {
                    self.store_fehler(&e);
                }
                self.neu_laden();
                if p.taetigkeit == Taetigkeit::Sichern && job.art == JobArt::Sichern {
                    // Gesicherter Stand hat sich geändert: Wiederherstellen neu laden lassen.
                    if self.wh.auftrag_id == Some(p.auftrag_id) {
                        self.wh.zuruecksetzen(Some(p.auftrag_id));
                    }
                }
            }
        }
        if self.schliessen_nach_job {
            self.darf_schliessen = true;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Aufträge «beim Anschliessen»: alle 30 s im Hintergrund nachsehen, ob das Ziel da ist.
    fn anschluss_pruefen(&mut self) {
        let heute = lauf::heute();
        if let Some(p) = &self.anschluss_pruefung {
            let fertig = match p.lock() {
                Ok(mut g) => g.take(),
                Err(e) => e.into_inner().take(),
            };
            if let Some(liste) = fertig {
                self.anschluss_pruefung = None;
                for (id, da) in liste {
                    let Some(a) = self.daten.auftrag(id) else { continue };
                    let schluessel = (id, heute.clone());
                    if da
                        && a.vorschau_gesehen
                        && !self.daten.heute_gesichert(id, &heute)
                        && !self.anschluss_versucht.contains(&schluessel)
                        && !self.anschluss_warteschlange.contains(&id)
                    {
                        self.anschluss_versucht.insert(schluessel);
                        self.anschluss_warteschlange.push(id);
                    }
                }
            }
        }
        let faellig = self.anschluss_zuletzt.map(|z| z.elapsed() >= Duration::from_secs(30)).unwrap_or(true);
        if faellig && self.anschluss_pruefung.is_none() {
            self.anschluss_zuletzt = Some(Instant::now());
            let liste: Vec<Auftrag> =
                self.daten.auftraege.iter().filter(|a| a.zeitplan == Zeitplan::BeimAnschliessen).cloned().collect();
            if !liste.is_empty() {
                let slot = Arc::new(Mutex::new(None));
                let (s2, ctx) = (slot.clone(), self.ctx.clone());
                std::thread::spawn(move || {
                    let r: Vec<(u64, bool)> = liste.iter().map(|a| (a.id, motor::ziel_da(a))).collect();
                    if let Ok(mut g) = s2.lock() {
                        *g = Some(r);
                    }
                    ctx.request_repaint();
                });
                self.anschluss_pruefung = Some(slot);
            }
        }
        if self.job.is_none() && !self.anschluss_warteschlange.is_empty() {
            let id = self.anschluss_warteschlange.remove(0);
            if let Some(a) = self.daten.auftrag(id).cloned() {
                self.sichern_starten(a, Ausloeser::Anschluss);
            }
        }
    }
}

fn fehler_text(f: &Fehler) -> String {
    let mut s = t(f.art.text()).to_string();
    if !f.detail.is_empty() {
        s.push_str(": ");
        s.push_str(&motor_text(&f.detail));
    }
    if !f.pfad.is_empty() {
        s.push_str(" – ");
        s.push_str(&f.pfad);
    }
    s
}

/// Meldungen aus motor.rs/store (deutsch, fest) übersetzen; Systemmeldungen bleiben, wie sie sind.
fn motor_text(s: &str) -> String {
    const TEXTE: [&str; 29] = [
        "Kein Ziel gewählt",
        "Laufwerk nicht angeschlossen oder Ordner nicht vorhanden",
        "Das Ziel liegt in diesem Quellordner – bitte ein anderes Ziel wählen",
        "Dieser Quellordner liegt im Sicherungsordner",
        "Dateiname mit ungültigen Zeichen – übersprungen",
        "Ordner zu tief verschachtelt",
        "Ordner nicht gefunden – nichts davon gesichert, im Ziel nichts entfernt",
        "Quellordner ist leer – im Ziel wurde nichts entfernt",
        "Das Ziellaufwerk kann keine harten Verknüpfungen – es wird alles kopiert",
        "Wegen Fehlern wurden keine alten Versionen entfernt",
        "Keine Quellordner gewählt",
        "Nicht gefunden",
        "Ordner nicht gefunden",
        "Ungültiger Pfad in der Sicherung – übersprungen",
        "Daten im ZIP beschädigt (CRC)",
        "Noch keine Sicherung vorhanden",
        "Noch keine Version vorhanden",
        "Grösse stimmt nicht",
        "Inhalt weicht von der Quelle ab",
        "abgebrochen",
        "ungültiger Name im ZIP",
        "kein ZIP-Archiv (Ende nicht gefunden)",
        "ZIP-Ende beschädigt",
        "ZIP64-Zeiger fehlt",
        "ZIP64-Ende beschädigt",
        "ZIP-Inhaltsverzeichnis ausserhalb der Datei",
        "ZIP-Inhaltsverzeichnis beschädigt",
        "ZIP-Eintrag beschädigt",
        "ZIP-Eintrag ist komprimiert (nur ungepackte Einträge von Sicherwerk werden unterstützt)",
    ];
    match TEXTE.iter().copied().find(|x| *x == s) {
        Some(de) => t(de).to_string(),
        None => s.to_string(),
    }
}

fn protokoll_kurz(p: &Protokoll) -> String {
    let groesse = plan::groesse_text(p.bytes);
    let dauer = plan::dauer_text(p.dauer_s);
    match p.taetigkeit {
        Taetigkeit::Sichern => tf("{name}: {ergebnis} – {neu} neu, {geaendert} geändert, {entfernt} entfernt, {groesse} in {dauer}",
            &[
                ("name", &p.auftrag_name),
                ("ergebnis", &t(p.ergebnis.text())),
                ("neu", &p.neu),
                ("geaendert", &p.geaendert),
                ("entfernt", &p.entfernt),
                ("groesse", &groesse),
                ("dauer", &dauer),
            ],
        ),
        Taetigkeit::Pruefen => tf("{name}: Prüfen {ergebnis} – {n} Dateien gelesen, {gleich} mit der Quelle verglichen, {fehler} Fehler",
            &[
                ("name", &p.auftrag_name),
                ("ergebnis", &t(p.ergebnis.text())),
                ("n", &p.geprueft),
                ("gleich", &p.unveraendert),
                ("fehler", &p.fehler_anzahl),
            ],
        ),
        Taetigkeit::Wiederherstellen => tf("{name}: Wiederherstellen {ergebnis} – {n} Dateien ({groesse})",
            &[("name", &p.auftrag_name), ("ergebnis", &t(p.ergebnis.text())), ("n", &p.neu), ("groesse", &groesse)],
        ),
    }
}

fn stand_text(s: &Stand) -> String {
    match s {
        Stand::Spiegel => t("Aktueller Stand (Spiegel)").to_string(),
        Stand::Ordner(n) => tf("Version vom {datum}", &[("datum", &plan::datum_anzeige(n))]),
        Stand::Zip(n) => tf("Version vom {datum} (ZIP)", &[("datum", &plan::datum_anzeige(n))]),
        Stand::Entfernt(n) => tf("Beim Spiegeln entfernt am {datum}", &[("datum", &plan::datum_anzeige(n))]),
    }
}

// ------------------------------------------------------------------ Zeichnen

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.job_abfragen();
        self.anschluss_pruefen();
        if self.job.is_none() && self.zuletzt_geladen.elapsed() >= Duration::from_secs(15) {
            self.neu_laden();
        }
        // Schliessen, während etwas läuft: nachfragen.
        if ctx.input(|i| i.viewport().close_requested()) && !self.darf_schliessen {
            let laeuft_sicherung = self.job.as_ref().is_some_and(|j| matches!(j.art, JobArt::Sichern | JobArt::Wiederherstellen));
            if laeuft_sicherung || self.dirty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.schliessen_frage = true;
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(150));
        } else {
            ctx.request_repaint_after(Duration::from_secs(5));
        }

        let gesp = self.gespeichert().cloned();
        stil::kopfleiste(ctx, |ui| self.kopf(ui, gesp.as_ref()));
        if let Some(a) = gesp.as_ref() {
            let n = self.wh.auswahl.len();
            if self.tab == Tab::Wiederherstellen && self.wh.auftrag_id == Some(a.id) && n > 0 {
                stil::auswahlleiste(ctx, &tf("{n} ausgewählt", &[("n", &n)]), |ui| self.auswahlzeile(ui, a));
            }
        }
        stil::fussleiste(ctx, |ui| self.fuss(ui, gesp.as_ref()));
        self.liste(ctx);
        stil::mitte(ctx, |ui| {
            egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(ui, |ui| self.mitte(ui));
        });
        self.fenster(ctx);
    }
}

impl App {
    /// Kann der Auftrag jetzt laufen? Sonst mit Grund.
    fn bereit(&self, a: &Auftrag) -> (bool, &'static str) {
        let frei = self.job.is_none() && !self.dirty();
        let bereit = frei && !a.quellen.is_empty() && !a.ziel.trim().is_empty();
        let warum = if self.job.is_some() {
            t("Es läuft gerade etwas.")
        } else if self.dirty() {
            t("Zuerst die Änderungen speichern.")
        } else {
            t("Zuerst Quellordner und Ziel wählen.")
        };
        (bereit, warum)
    }

    fn neu_klick(&mut self) {
        if self.dirty() {
            self.wechsel = Some(None);
            // Nach dem Speichern/Verwerfen wird ein neuer Auftrag angelegt.
            self.neuer_nach_wechsel = true;
        } else {
            self.neuer_auftrag();
        }
    }

    /// Kopfleiste: Neuer Auftrag | Vorschau, Prüfen; rechts der Speicherort.
    fn kopf(&mut self, ui: &mut egui::Ui, gesp: Option<&Auftrag>) {
        stil::titel(ui, "Sicherwerk");
        stil::trenner(ui);
        if stil::knopf_an(ui, self.job.is_none(), Sym::Plus, t("Neuer Auftrag")).clicked() {
            self.neu_klick();
        }
        if let Some(a) = gesp {
            let (bereit, warum) = self.bereit(a);
            stil::trenner(ui);
            let r = stil::knopf_an(ui, bereit, Sym::Auge, t("Vorschau"));
            let r = r.on_hover_text(if bereit { t("Zeigt, was beim Sichern passieren würde – ohne etwas zu verändern.") } else { warum });
            if r.clicked() {
                self.vorschau_starten(a.clone());
            }
            let r = stil::knopf_an(ui, bereit, Sym::Schild, t("Prüfen"));
            let r = r.on_hover_text(if bereit {
                t("Stichprobe: Dateien der Sicherung lesen und per Prüfsumme mit der Quelle vergleichen.")
            } else {
                warum
            });
            if r.clicked() {
                self.pruefen_starten(a.clone());
            }
        }
        stil::rechts(ui, |ui| {
            if let Some(p) = store::datei() {
                stil::leise(ui, true, Sym::Info, "", "")
                    .on_hover_text(format!("{}\n{}", t("Hier speichert Sicherwerk Aufträge und Protokolle."), p.display()));
            }
            stil::grau(ui, t("Ordner regelmässig sichern"));
        });
    }

    /// Auswahlleiste im Reiter Wiederherstellen.
    fn auswahlzeile(&mut self, ui: &mut egui::Ui, a: &Auftrag) {
        let r = stil::knopf_an(ui, self.job.is_none(), Sym::Upload, t("Wiederherstellen nach …"))
            .on_hover_text(t("In einen frei gewählten Ordner. Vorhandene Dateien werden nie überschrieben – bei gleichem Namen heisst die neue «Name (2)»."));
        if r.clicked() {
            if let Some(ziel) = rfd::FileDialog::new().set_title(t("Wohin wiederherstellen?")).pick_folder() {
                if let Some(s) = self.wh.gewaehlt.clone() {
                    let auswahl: Vec<String> = self.wh.auswahl.iter().cloned().collect();
                    self.wiederherstellen_starten(a.clone(), s, auswahl, ziel);
                }
            }
        }
        stil::rechts(ui, |ui| {
            if stil::leise(ui, true, Sym::Kreuz, t("Auswahl leeren"), "").clicked() {
                self.wh.auswahl.clear();
            }
        });
    }

    /// Fussleiste: links Meldung, rechts «Jetzt sichern» (beim Laufen: Abbrechen).
    fn fuss(&mut self, ui: &mut egui::Ui, gesp: Option<&Auftrag>) {
        ui.horizontal(|ui| {
            stil::rechts(ui, |ui| {
                if let Some(a) = gesp {
                    let (bereit, warum) = self.bereit(a);
                    let r = stil::haupt(ui, bereit, Sym::Speichern, t("Jetzt sichern"));
                    let r = if bereit { r } else { r.on_hover_text(warum) };
                    if r.clicked() {
                        if a.vorschau_gesehen {
                            self.sichern_starten(a.clone(), Ausloeser::Hand);
                        } else {
                            // Vor dem ersten Lauf (und nach Änderungen) zuerst zeigen, was passiert.
                            self.vorschau_starten(a.clone());
                        }
                    }
                }
                if let Some(j) = &self.job {
                    if !matches!(j.art, JobArt::Staende) && stil::knopf_an(ui, !j.fs.abgebrochen(), Sym::Stopp, t("Abbrechen")).clicked() {
                        j.fs.abbrechen();
                    }
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| match &self.meldung {
                    Some((text, ton)) => {
                        let c = match ton {
                            Ton::Gut => gruen(),
                            Ton::Warnung => orange(),
                            Ton::Fehler => rot(),
                        };
                        ui.add(egui::Label::new(egui::RichText::new(text).size(13.0).color(c)).truncate());
                    }
                    None => {
                        let n = self.daten.auftraege.len();
                        stil::grau(ui, &tf("{n} Aufträge", &[("n", &n)]));
                    }
                });
            });
        });
    }

    fn liste(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("auftraege")
            .resizable(true)
            .default_width(270.0)
            .min_width(220.0)
            .frame(
                egui::Frame::none()
                    .fill(thema::bg())
                    .stroke(egui::Stroke::new(1.0, thema::border()))
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    stil::symbol(ui, Sym::Schild, 16.0, akzent());
                    ui.label(egui::RichText::new(t("Sicherungsaufträge")).size(14.0).strong());
                });
                ui.add_space(6.0);
                if self.daten.auftraege.is_empty() {
                    stil::hinweis(ui, t("Noch keine Aufträge. Oben «Neuer Auftrag» wählen."));
                }
                let mut klick: Option<u64> = None;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for a in &self.daten.auftraege {
                        let gewaehlt = self.sel == Some(a.id);
                        let fill = if gewaehlt { stil::toenung() } else { thema::panel() };
                        let rand = if gewaehlt { stil::mix(stil::farbe(), thema::border(), 0.5) } else { thema::border() };
                        let resp = egui::Frame::none()
                            .fill(fill)
                            .stroke(egui::Stroke::new(1.0, rand))
                            .rounding(10.0)
                            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(egui::RichText::new(&a.name).strong());
                                let art = match a.art {
                                    Art::Spiegeln => t("Spiegeln"),
                                    Art::Versionen => t("Versionen"),
                                };
                                let zp = t(a.zeitplan.text());
                                grau(ui, format!("{art} · {zp}"));
                                ui.horizontal(|ui| {
                                    if self.job.as_ref().is_some_and(|j| j.auftrag_id == a.id) {
                                        stil::marke(ui, t("läuft …"), stil::blau(), 0.0);
                                    } else {
                                        match self.daten.letztes(a.id, Taetigkeit::Sichern) {
                                            Some(p) => {
                                                stil::marke(ui, t(p.ergebnis.text()), ergebnis_farbe(p.ergebnis), 0.0);
                                                grau(ui, p.start.get(..16).unwrap_or(&p.start).to_string());
                                            }
                                            None => grau(ui, t("Noch nie gesichert")),
                                        }
                                    }
                                });
                            })
                            .response;
                        let resp = ui.interact(resp.rect, ui.id().with(("auftrag", a.id)), egui::Sense::click());
                        if resp.clicked() && !gewaehlt {
                            klick = Some(a.id);
                        }
                        ui.add_space(5.0);
                    }
                });
                if let Some(id) = klick {
                    if self.dirty() {
                        self.wechsel = Some(Some(id));
                    } else {
                        self.waehlen(Some(id));
                    }
                }
            });
    }

    fn mitte(&mut self, ui: &mut egui::Ui) {
        let Some(gesp) = self.gespeichert().cloned() else {
            stil::leer(
                ui,
                Sym::Schild,
                t("Willkommen bei Sicherwerk"),
                t("Lege einen Sicherungsauftrag an: welche Ordner, wohin (USB-Stick, zweite Festplatte, NAS) und wie."),
            );
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                if stil::knopf(ui, Sym::Plus, t("Neuer Auftrag")).clicked() {
                    self.neuer_auftrag();
                }
            });
            return;
        };
        self.aktionen(ui, &gesp);
        ui.add_space(10.0);
        stil::wahl(
            ui,
            &mut self.tab,
            &[
                (Tab::Einstellungen, t("Einstellungen")),
                (Tab::Zeitplan, t("Zeitplan")),
                (Tab::Protokoll, t("Protokoll")),
                (Tab::Wiederherstellen, t("Wiederherstellen")),
            ],
        );
        ui.add_space(6.0);
        match self.tab {
            Tab::Einstellungen => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.einstellungen(ui));
            }
            Tab::Zeitplan => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.zeitplan_tab(ui, &gesp));
            }
            Tab::Protokoll => self.protokoll_tab(ui, &gesp),
            Tab::Wiederherstellen => self.wiederherstellen_tab(ui, &gesp),
        }
    }

    /// Kopf des gewählten Auftrags: Name, letzter Lauf, Knöpfe, Fortschritt.
    fn aktionen(&mut self, ui: &mut egui::Ui, a: &Auftrag) {
        karte().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&a.name).size(18.0).strong());
                    match self.daten.letztes(a.id, Taetigkeit::Sichern) {
                        Some(p) => {
                            let txt = tf("Letzte Sicherung: {zeit} – {ergebnis}",
                                &[("zeit", &p.start), ("ergebnis", &t(p.ergebnis.text()))],
                            );
                            ui.label(egui::RichText::new(txt).color(ergebnis_farbe(p.ergebnis)));
                        }
                        None => grau(ui, t("Noch keine Sicherung.")),
                    }
                });
                if let Some(j) = &self.job {
                    if j.auftrag_id != a.id {
                        stil::rechts(ui, |ui| {
                            stil::marke(ui, &tf("läuft: {name}", &[("name", &j.name)]), stil::blau(), 0.0);
                        });
                    }
                }
            });
            if let Some(j) = &self.job {
                ui.add_space(8.0);
                ui.separator();
                let fs_ = j.fs.clone();
                let titel = match j.art {
                    JobArt::Sichern => tf("Sichern: {name}", &[("name", &j.name)]),
                    JobArt::Pruefen => tf("Prüfen: {name}", &[("name", &j.name)]),
                    JobArt::Vorschau => tf("Vorschau: {name}", &[("name", &j.name)]),
                    JobArt::Wiederherstellen => tf("Wiederherstellen: {name}", &[("name", &j.name)]),
                    JobArt::Staende | JobArt::Inhalt => tf("Sicherung lesen: {name}", &[("name", &j.name)]),
                };
                let vergangen = plan::dauer_text(j.start.elapsed().as_secs());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(titel).strong());
                    ui.label(egui::RichText::new(t(fs_.phase().text())).color(thema::muted()));
                    stil::rechts(ui, |ui| {
                        ui.label(egui::RichText::new(vergangen).color(thema::muted()));
                    });
                });
                let bar = match fs_.anteil() {
                    Some(f) => egui::ProgressBar::new(f).show_percentage(),
                    None => egui::ProgressBar::new(0.0).animate(true),
                };
                ui.add(bar.fill(stil::farbe()));
                use std::sync::atomic::Ordering::Relaxed;
                let df = fs_.dateien_fertig.load(Relaxed);
                let dg = fs_.dateien_gesamt.load(Relaxed);
                let bf = plan::groesse_text(fs_.bytes_fertig.load(Relaxed));
                let bg = plan::groesse_text(fs_.bytes_gesamt.load(Relaxed));
                let zeile = tf("{df} von {dg} Dateien · {bf} von {bg}", &[("df", &df), ("dg", &dg), ("bf", &bf), ("bg", &bg)]);
                grau(ui, zeile);
                let akt = fs_.aktuell();
                if !akt.is_empty() {
                    let kurz: String = if akt.chars().count() > 110 {
                        let n = akt.chars().count();
                        format!("…{}", akt.chars().skip(n - 108).collect::<String>())
                    } else {
                        akt
                    };
                    grau(ui, kurz);
                }
            }
        });
    }

    // -------------------------------------------------------------- Einstellungen

    fn einstellungen(&mut self, ui: &mut egui::Ui) {
        let Some(mut e) = self.entwurf.take() else { return };
        let mut quelle_weg: Option<usize> = None;
        let mut quelle_dazu: Option<String> = None;

        stil::karte(ui, "name", Sym::Stift, t("Name"), true, |ui| {
            ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(f32::INFINITY).hint_text(t("z. B. Fotos auf USB-Stick")));
        });

        stil::karte(ui, "quellen", Sym::Ordner, t("Quellordner"), true, |ui| {
            stil::hinweis(ui, t("Diese Ordner werden gesichert. Sicherwerk liest sie nur – es verändert und löscht darin nie etwas."));
            if e.quellen.is_empty() {
                ui.label(egui::RichText::new(t("Noch kein Ordner gewählt.")).color(orange()));
            }
            for (i, q) in e.quellen.iter().enumerate() {
                ui.horizontal(|ui| {
                    stil::symbol(ui, Sym::Ordner, 16.0, akzent());
                    ui.label(egui::RichText::new(&q.pfad).monospace());
                    grau(ui, tf("→ im Ziel: {name}", &[("name", &q.name)]));
                    stil::rechts(ui, |ui| {
                        if stil::leise(ui, true, Sym::Kreuz, "", t("Aus dem Auftrag entfernen")).clicked() {
                            quelle_weg = Some(i);
                        }
                    });
                });
            }
            ui.horizontal(|ui| {
                if stil::knopf(ui, Sym::Plus, t("Ordner wählen …")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_title(t("Quellordner wählen")).pick_folder() {
                        quelle_dazu = Some(p.display().to_string());
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.neuer_pfad)
                        .desired_width(300.0)
                        .hint_text(tf("oder Pfad eingeben, z. B. {beispiel}", &[("beispiel", &"\\\\nas\\daten")])),
                );
                if stil::knopf_text(ui, !self.neuer_pfad.trim().is_empty(), t("Hinzufügen")).clicked() {
                    quelle_dazu = Some(self.neuer_pfad.trim().to_string());
                    self.neuer_pfad.clear();
                }
            });
        });

        stil::karte(ui, "ziel", Sym::Speichern, t("Ziel"), true, |ui| {
            stil::hinweis(ui, t("USB-Stick, zweite Festplatte oder Netzwerkpfad (NAS). Darin legt Sicherwerk einen Ordner mit dem Auftragsnamen an."));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut e.ziel).desired_width((ui.available_width() - 110.0).max(160.0)).hint_text(t("z. B. E:\\Sicherung oder /media/oli/USB")));
                if stil::knopf(ui, Sym::OrdnerAuf, t("Wählen …")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_title(t("Ziel wählen")).pick_folder() {
                        e.ziel = p.display().to_string();
                    }
                }
            });
            if e.ziel.trim().is_empty() {
                ui.label(egui::RichText::new(t("Noch kein Ziel gewählt.")).color(orange()));
            } else {
                let ord = motor::auftrag_ordner(&e);
                grau(ui, tf("Sicherungen landen in: {pfad}", &[("pfad", &ord.display())]));
                let z = e.ziel.trim().trim_end_matches(['/', '\\']).to_lowercase();
                let innen = e.quellen.iter().any(|q| {
                    let q = q.pfad.trim().trim_end_matches(['/', '\\']).to_lowercase();
                    !q.is_empty() && (z == q || z.starts_with(&format!("{q}/")) || z.starts_with(&format!("{q}\\")))
                });
                if innen {
                    ui.label(egui::RichText::new(t("Das Ziel liegt in einem Quellordner – so kann nicht gesichert werden.")).color(rot()));
                }
            }
        });

        stil::karte(ui, "art", Sym::Kopieren, t("Art der Sicherung"), true, |ui| {
            stil::wahl(ui, &mut e.art, &[(Art::Spiegeln, t("Spiegeln")), (Art::Versionen, t("Versionen"))]);
            match e.art {
                Art::Spiegeln => {
                    stil::hinweis(ui, t("Spiegeln – nur Neues und Geändertes kopieren"));
                    stil::hinweis(ui, t("Verglichen wird nach Grösse und Änderungszeit. Das Ziel ist danach ein genaues Abbild der Quelle."));
                    stil::feld(ui, t("Im Ziel nicht mehr vorhandene Dateien:"), |ui| {
                        stil::liste(
                            ui,
                            "entfernte",
                            &mut e.entfernte,
                            &[
                                (Entfernte::Verschieben, t("nach _entfernt/<Datum> verschieben")),
                                (Entfernte::Loeschen, t("im Ziel löschen")),
                                (Entfernte::Behalten, t("im Ziel behalten")),
                            ],
                        );
                    });
                }
                Art::Versionen => {
                    stil::hinweis(ui, t("Versionen – bei jedem Lauf ein eigener, datierter Stand"));
                    stil::feld(ui, t("Form:"), |ui| {
                        stil::wahl(ui, &mut e.form, &[(VersionForm::Ordner, t("Ordner")), (VersionForm::Zip, t("ZIP-Datei"))]);
                    });
                    ui.horizontal(|ui| {
                        ui.label(t("Aufbewahren: die letzten"));
                        ui.add(egui::DragValue::new(&mut e.behalten).range(1..=999));
                        ui.label(t("Versionen"));
                    });
                    if e.form == VersionForm::Ordner {
                        stil::schalter(ui, &mut e.verknuepfen, t("Unveränderte Dateien nicht doppelt speichern (harte Verknüpfung)"));
                        stil::hinweis(ui, t("Jede Version sieht vollständig aus, belegt aber nur Platz für Geändertes. Geht auf NTFS und Linux-Dateisystemen, nicht auf FAT/exFAT – dann wird kopiert."));
                    } else {
                        stil::hinweis(ui, t("ZIP ohne Kompression: schnell, und jede Datei lässt sich mit jedem Entpacker öffnen."));
                    }
                }
            }
        });

        stil::karte(ui, "ausschluesse", Sym::Kreuz, t("Ausschlüsse"), true, |ui| {
            stil::hinweis(ui, t("Ein Muster pro Zeile. Ohne / gilt es für Datei- und Ordnernamen überall (*.tmp, node_modules, Cache), mit / für Pfade ab dem Quellordner (Projekte/*/build). ** steht für beliebig viele Ordner."));
            ui.add(egui::TextEdit::multiline(&mut self.ausschluss_text).desired_rows(5).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            ui.horizontal_wrapped(|ui| {
                grau(ui, t("Schnell dazu:"));
                for v in AUSSCHLUSS_VORSCHLAEGE {
                    let schon = self.ausschluss_text.lines().any(|l| l.trim().eq_ignore_ascii_case(v));
                    if ui.add_enabled(!schon, egui::Button::new(egui::RichText::new(v).monospace()).small()).clicked() {
                        if !self.ausschluss_text.is_empty() && !self.ausschluss_text.ends_with('\n') {
                            self.ausschluss_text.push('\n');
                        }
                        self.ausschluss_text.push_str(v);
                    }
                }
            });
            e.ausschluesse =
                self.ausschluss_text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        });

        stil::karte(ui, "pruefen", Sym::Schild, t("Prüfen"), false, |ui| {
            ui.horizontal(|ui| {
                ui.label(t("Stichprobe:"));
                ui.add(egui::DragValue::new(&mut e.stichprobe).range(1..=100_000));
                ui.label(t("Dateien je Prüfung"));
            });
        });

        if let Some(i) = quelle_weg {
            if i < e.quellen.len() {
                e.quellen.remove(i);
            }
        }
        if let Some(p) = quelle_dazu {
            let gke = cfg!(windows);
            let gleich = |a: &str, b: &str| {
                let a = a.trim().trim_end_matches(['/', '\\']);
                let b = b.trim().trim_end_matches(['/', '\\']);
                if gke {
                    a.eq_ignore_ascii_case(b)
                } else {
                    a == b
                }
            };
            if e.quellen.iter().any(|q| gleich(&q.pfad, &p)) {
                self.melden(t("Dieser Ordner ist schon im Auftrag."), Ton::Warnung);
            } else {
                let namen: Vec<&str> = e.quellen.iter().map(|q| q.name.as_str()).collect();
                let name = auftrag::quellname(&p, &namen);
                e.quellen.push(Quelle { pfad: p, name });
            }
        }

        ui.add_space(10.0);
        let dirty = self.dirty_mit(&e);
        ui.horizontal(|ui| {
            if stil::knopf_an(ui, dirty, Sym::Speichern, t("Speichern")).clicked() {
                self.entwurf = Some(e.clone());
                self.speichern();
                e = self.entwurf.clone().unwrap_or(e.clone());
            }
            if stil::knopf_text(ui, dirty, t("Verwerfen")).clicked() {
                if let Some(a) = self.gespeichert().cloned() {
                    self.ausschluss_text = a.ausschluesse.join("\n");
                    e = a;
                }
            }
            if dirty {
                ui.label(egui::RichText::new(t("Ungespeicherte Änderungen")).color(orange()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = stil::gefahr(ui, self.job.is_none(), Sym::Papierkorb, t("Auftrag löschen …"));
                if r.clicked() {
                    self.loeschen_frage = Some(e.id);
                }
            });
        });
        self.entwurf = Some(e);
    }

    fn dirty_mit(&self, e: &Auftrag) -> bool {
        self.gespeichert().is_some_and(|a| a != e)
    }

    // -------------------------------------------------------------- Zeitplan

    fn zeitplan_tab(&mut self, ui: &mut egui::Ui, gesp: &Auftrag) {
        let Some(mut e) = self.entwurf.take() else { return };
        stil::karte(ui, "wann", Sym::Kalender, t("Wann sichern?"), true, |ui| {
            let wahl: Vec<(Zeitplan, &str)> = Zeitplan::ALLE.iter().map(|z| (*z, t(z.text()))).collect();
            stil::liste(ui, "zeitplan", &mut e.zeitplan, &wahl);
            ui.add_space(6.0);
            match e.zeitplan {
                Zeitplan::Taeglich | Zeitplan::Woechentlich => {
                    ui.horizontal(|ui| {
                        if e.zeitplan == Zeitplan::Woechentlich {
                            ui.label(t("am"));
                            let tag = (e.wochentag as usize).min(6);
                            egui::ComboBox::from_id_salt("wochentag").selected_text(t(WOCHENTAGE[tag])).show_ui(ui, |ui| {
                                for (i, w) in WOCHENTAGE.iter().enumerate() {
                                    ui.selectable_value(&mut e.wochentag, i as u8, t(w));
                                }
                            });
                        }
                        ui.label(t("um"));
                        ui.add(egui::DragValue::new(&mut e.stunde).range(0..=23).custom_formatter(|v, _| format!("{:02}", v as i64)));
                        ui.label(":");
                        ui.add(egui::DragValue::new(&mut e.minute).range(0..=59).custom_formatter(|v, _| format!("{:02}", v as i64)));
                        ui.label(t("Uhr"));
                    });
                    grau(ui, t("War der Computer zu dieser Zeit aus, holt Windows die Sicherung beim nächsten Start nach."));
                }
                Zeitplan::BeimAnschliessen => {
                    grau(ui, t("Sicherwerk sichert einmal pro Tag, sobald das Ziel erreichbar ist: beim Programmstart und solange das Fenster offen ist. Unter Windows kann zusätzlich eine Aufgabe alle 15 Minuten im Hintergrund nachsehen."));
                }
                Zeitplan::Keiner => grau(ui, t("Gesichert wird nur mit «Jetzt sichern».")),
            }
        });
        ui.add_space(8.0);
        let dirty = self.dirty_mit(&e);
        ui.horizontal(|ui| {
            if stil::knopf_an(ui, dirty, Sym::Speichern, t("Speichern")).clicked() {
                self.entwurf = Some(e.clone());
                self.speichern();
                e = self.entwurf.clone().unwrap_or(e.clone());
            }
            if stil::knopf_text(ui, dirty, t("Verwerfen")).clicked() {
                e.zeitplan = gesp.zeitplan;
                e.stunde = gesp.stunde;
                e.minute = gesp.minute;
                e.wochentag = gesp.wochentag;
            }
        });
        let dirty = self.dirty_mit(&e);
        self.entwurf = Some(e);
        ui.add_space(10.0);
        if gesp.zeitplan == Zeitplan::Keiner {
            return;
        }
        if cfg!(windows) {
            self.windows_aufgabe(ui, gesp, dirty);
        } else {
            self.linux_anleitung(ui, gesp);
        }
    }

    fn windows_aufgabe(&mut self, ui: &mut egui::Ui, a: &Auftrag, dirty: bool) {
        stil::karte(ui, "aufgabe", Sym::Uhr, t("Windows-Aufgabenplanung"), true, |ui| {
            grau(ui, t("Damit gesichert wird, auch wenn Sicherwerk nicht offen ist, trägt Sicherwerk eine Aufgabe ein. Sie startet «sicherwerk --auftrag» ohne Fenster; das Ergebnis steht danach im Protokoll."));
            ui.add_space(4.0);
            if a.aufgabe_eingerichtet {
                ui.horizontal(|ui| {
                    stil::symbol(ui, Sym::Haken, 16.0, gruen());
                    ui.label(egui::RichText::new(t("In der Aufgabenplanung eingetragen")).color(gruen()));
                });
            } else {
                ui.label(egui::RichText::new(t("Nicht in der Aufgabenplanung eingetragen")).color(orange()));
            }
            let warum = if dirty {
                t("Zuerst die Änderungen speichern.")
            } else if !a.vorschau_gesehen {
                t("Vor dem ersten Eintragen einmal die Vorschau ansehen.")
            } else {
                ""
            };
            ui.horizontal(|ui| {
                let darf = !dirty && a.vorschau_gesehen;
                let r = stil::knopf_an(ui, darf, Sym::Kalender, if a.aufgabe_eingerichtet { t("Neu eintragen") } else { t("Eintragen") });
                let r = if warum.is_empty() || darf { r } else { r.on_hover_text(warum) };
                if r.clicked() {
                    let ergebnis = aufgabe::einrichten(a);
                    let ok = ergebnis.is_ok();
                    let id = a.id;
                    match store::aendern(|d| {
                        if let Some(x) = d.auftraege.iter_mut().find(|x| x.id == id) {
                            x.aufgabe_eingerichtet = ok;
                        }
                    }) {
                        Ok(_) => self.neu_laden(),
                        Err(e) => self.store_fehler(&e),
                    }
                    match ergebnis {
                        Ok(()) => self.melden(t("In der Aufgabenplanung eingetragen."), Ton::Gut),
                        Err(x) => self.melden(&format!("{} {x}", t("Aufgabenplanung:")), Ton::Fehler),
                    }
                }
                if a.aufgabe_eingerichtet && stil::gefahr(ui, true, Sym::Kreuz, t("Austragen")).clicked() {
                    let mut r = aufgabe::entfernen(&a.name);
                    if a.zeitplan == Zeitplan::BeimAnschliessen {
                        r = r.and(aufgabe::anschluss_aufraeumen(&self.daten.auftraege, a.id));
                    }
                    let id = a.id;
                    match store::aendern(|d| {
                        if let Some(x) = d.auftraege.iter_mut().find(|x| x.id == id) {
                            x.aufgabe_eingerichtet = false;
                        }
                    }) {
                        Ok(_) => self.neu_laden(),
                        Err(e) => self.store_fehler(&e),
                    }
                    match r {
                        Ok(()) => self.melden(t("Aus der Aufgabenplanung ausgetragen."), Ton::Gut),
                        Err(x) => self.melden(&format!("{} {x}", t("Aufgabenplanung:")), Ton::Fehler),
                    }
                }
            });
            ui.add_space(4.0);
            grau(ui, tf("Tipp: Für ein NAS besser den Netzwerkpfad ({beispiel}) als einen Laufwerksbuchstaben verwenden – verbundene Laufwerke fehlen Aufgaben oft.", &[("beispiel", &"\\\\nas\\freigabe")]));
        });
    }

    fn linux_anleitung(&mut self, ui: &mut egui::Ui, a: &Auftrag) {
        let Some(tx) = aufgabe::linux_texte(a) else { return };
        let mut kopiert: Option<String> = None;
        stil::karte(ui, "linux", Sym::Uhr, t("Unter Linux selbst einrichten"), true, |ui| {
            grau(ui, t("Sicherwerk richtet unter Linux nichts automatisch ein. Wähle eine der beiden Möglichkeiten. Der Auftrag läuft dann ohne Fenster, das Ergebnis steht im Protokoll."));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(t("Möglichkeit 1: cron")).strong());
            grau(ui, t("«crontab -e» öffnen und diese Zeile anfügen:"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut tx.cron.as_str()).font(egui::TextStyle::Monospace).desired_width(ui.available_width() - 90.0));
                if stil::leise(ui, true, Sym::Kopieren, t("Kopieren"), "").clicked() {
                    kopiert = Some(tx.cron.clone());
                }
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t("Möglichkeit 2: systemd-Timer (holt verpasste Läufe nach)")).strong());
            let d_pfad = format!("~/.config/systemd/user/{}.service", tx.einheit);
            let t_pfad = format!("~/.config/systemd/user/{}.timer", tx.einheit);
            for (pfad, inhalt, id) in [(&d_pfad, &tx.dienst, "dienst"), (&t_pfad, &tx.timer, "timer")] {
                ui.horizontal(|ui| {
                    grau(ui, tf("Datei {pfad}:", &[("pfad", pfad)]));
                    if stil::leise(ui, true, Sym::Kopieren, t("Kopieren"), "").clicked() {
                        kopiert = Some(inhalt.clone());
                    }
                });
                ui.push_id(id, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut inhalt.as_str())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(inhalt.lines().count().max(3)),
                    );
                });
            }
            let befehl = format!("systemctl --user daemon-reload && systemctl --user enable --now {}.timer", tx.einheit);
            grau(ui, t("Danach im Terminal:"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut befehl.as_str()).font(egui::TextStyle::Monospace).desired_width(ui.available_width() - 90.0));
                if stil::leise(ui, true, Sym::Kopieren, t("Kopieren"), "").clicked() {
                    kopiert = Some(befehl.clone());
                }
            });
            grau(ui, t("Soll der Timer auch laufen, wenn du nicht angemeldet bist: «loginctl enable-linger»."));
            if a.zeitplan == Zeitplan::BeimAnschliessen {
                grau(ui, t("«beim Anschliessen»: eine gemeinsame Einheit für alle solchen Aufträge – sie schaut alle 15 Minuten nach."));
            }
        });
        if let Some(s) = kopiert {
            ui.output_mut(|o| o.copied_text = s);
            self.melden(t("In die Zwischenablage kopiert."), Ton::Gut);
        }
    }

    // -------------------------------------------------------------- Protokoll

    fn protokoll_tab(&mut self, ui: &mut egui::Ui, a: &Auftrag) {
        let liste: Vec<&Protokoll> = self.daten.protokolle.iter().rev().filter(|p| p.auftrag_id == a.id).take(300).collect();
        if liste.is_empty() {
            grau(ui, t("Noch keine Einträge."));
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, p) in liste.iter().enumerate() {
                ui.push_id(i, |ui| {
                    let kopf = format!(
                        "{}   {} · {} · {}",
                        p.start,
                        t(p.taetigkeit.text()),
                        t(p.ergebnis.text()),
                        t(p.ausloeser.text())
                    );
                    egui::CollapsingHeader::new(egui::RichText::new(kopf).color(ergebnis_farbe(p.ergebnis))).default_open(i == 0).show(ui, |ui| {
                        egui::Grid::new("werte").num_columns(2).spacing([16.0, 3.0]).show(ui, |ui| {
                            let zeile = |ui: &mut egui::Ui, k: &str, v: String| {
                                ui.label(egui::RichText::new(k).color(thema::muted()));
                                ui.label(v);
                                ui.end_row();
                            };
                            match p.taetigkeit {
                                Taetigkeit::Sichern => {
                                    zeile(ui, t("Neu kopiert"), p.neu.to_string());
                                    zeile(ui, t("Geändert"), p.geaendert.to_string());
                                    if p.entfernt > 0 {
                                        zeile(ui, t("Im Ziel entfernt"), p.entfernt.to_string());
                                    }
                                    if p.verknuepft > 0 {
                                        zeile(ui, t("Verknüpft (unverändert)"), p.verknuepft.to_string());
                                    }
                                    if p.unveraendert > 0 {
                                        zeile(ui, t("Unverändert"), p.unveraendert.to_string());
                                    }
                                    zeile(ui, t("Kopiert"), plan::groesse_text(p.bytes));
                                    if !p.version.is_empty() {
                                        zeile(ui, t("Version"), p.version.clone());
                                    }
                                    if p.versionen_geloescht > 0 {
                                        zeile(ui, t("Alte Versionen entfernt"), p.versionen_geloescht.to_string());
                                    }
                                }
                                Taetigkeit::Pruefen => {
                                    zeile(ui, t("Dateien gelesen"), p.geprueft.to_string());
                                    zeile(ui, t("Mit der Quelle verglichen"), p.unveraendert.to_string());
                                    if !p.version.is_empty() {
                                        zeile(ui, t("Version"), p.version.clone());
                                    }
                                }
                                Taetigkeit::Wiederherstellen => {
                                    zeile(ui, t("Dateien"), p.neu.to_string());
                                    zeile(ui, t("Grösse"), plan::groesse_text(p.bytes));
                                    if !p.version.is_empty() {
                                        zeile(ui, t("Stand"), plan::datum_anzeige(&p.version));
                                    }
                                }
                            }
                            zeile(ui, t("Dauer"), plan::dauer_text(p.dauer_s));
                            zeile(ui, t("Fehler"), p.fehler_anzahl.to_string());
                        });
                        if !p.fehler.is_empty() {
                            ui.add_space(4.0);
                            for f in &p.fehler {
                                let c = if f.art == FehlerArt::Hinweis { thema::muted() } else { rot() };
                                ui.label(egui::RichText::new(fehler_text(f)).color(c).small());
                            }
                            if p.fehler_anzahl as usize > p.fehler.iter().filter(|f| f.art != FehlerArt::Hinweis).count() {
                                grau(ui, t("(weitere Fehler nicht gespeichert)"));
                            }
                        }
                    });
                });
            }
        });
    }

    // -------------------------------------------------------------- Wiederherstellen

    fn wiederherstellen_tab(&mut self, ui: &mut egui::Ui, a: &Auftrag) {
        if self.wh.auftrag_id != Some(a.id) {
            self.wh.zuruecksetzen(Some(a.id));
        }
        if !self.wh.geladen && self.job.is_none() {
            self.staende_laden(a.clone());
        }
        let laeuft = self.job.as_ref().is_some_and(|j| matches!(j.art, JobArt::Staende | JobArt::Inhalt));
        ui.horizontal(|ui| {
            ui.label(t("Stand:"));
            let aktuell = self.wh.gewaehlt.as_ref().map(stand_text).unwrap_or_else(|| t("– keiner –").to_string());
            let mut neu: Option<Stand> = None;
            egui::ComboBox::from_id_salt("stand").width(320.0).selected_text(aktuell).show_ui(ui, |ui| {
                for s in &self.wh.staende {
                    if ui.selectable_label(self.wh.gewaehlt.as_ref() == Some(s), stand_text(s)).clicked() {
                        neu = Some(s.clone());
                    }
                }
            });
            if let Some(s) = neu {
                if self.wh.gewaehlt.as_ref() != Some(&s) {
                    self.inhalt_laden(a.clone(), s);
                }
            }
            if stil::leise(ui, self.job.is_none(), Sym::Aktualisieren, "", t("Aktualisieren")).clicked() {
                self.staende_laden(a.clone());
            }
            if laeuft {
                ui.spinner();
            }
        });
        if let Some(f) = &self.wh.fehler {
            ui.label(egui::RichText::new(f).color(rot()));
            return;
        }
        if self.wh.geladen && !laeuft && self.wh.staende.is_empty() {
            grau(ui, t("Noch keine Sicherung gefunden (oder das Ziel ist nicht angeschlossen)."));
            return;
        }
        if self.wh.gewaehlt.is_none() || laeuft {
            return;
        }
        ui.add_space(6.0);
        // Pfadleiste und Suche
        ui.horizontal(|ui| {
            if ui.selectable_label(self.wh.pfad.is_empty(), t("Alles")).clicked() {
                self.wh.pfad.clear();
            }
            let teile: Vec<String> = if self.wh.pfad.is_empty() { Vec::new() } else { self.wh.pfad.split('/').map(|s| s.to_string()).collect() };
            for (i, teil) in teile.iter().enumerate() {
                ui.label("›");
                if ui.selectable_label(i + 1 == teile.len(), teil).clicked() {
                    self.wh.pfad = teile[..=i].join("/");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.wh.filter).desired_width(220.0).hint_text(t("Suchen …")));
            });
        });
        ui.add_space(4.0);
        // Liste
        let zeilen: Vec<Knoten> = if self.wh.filter.trim().is_empty() {
            self.wh.kinder.get(&self.wh.pfad).cloned().unwrap_or_default()
        } else {
            let f = self.wh.filter.trim().to_lowercase();
            self.wh
                .inhalt
                .iter()
                .filter(|e| e.rel.to_lowercase().contains(&f))
                .take(2000)
                .map(|e| Knoten {
                    rel: e.rel.clone(),
                    name: e.rel.clone(),
                    ordner: e.ordner,
                    groesse: e.groesse,
                    zeit: e.zeit,
                })
                .collect()
        };
        let unten_hoehe = 48.0;
        let hoehe = (ui.available_height() - unten_hoehe).max(120.0);
        let mut umschalten: Option<String> = None;
        let mut oeffnen: Option<String> = None;
        let zeilen_h = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
        karte().inner_margin(egui::Margin::symmetric(8.0, 6.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if zeilen.is_empty() {
                grau(ui, t("Leer."));
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(hoehe).show_rows(ui, zeilen_h, zeilen.len(), |ui, bereich| {
                for k in zeilen.get(bereich).unwrap_or(&[]) {
                    ui.horizontal(|ui| {
                        let status = self.wh.gewaehlt(&k.rel);
                        let mut an = status.is_some();
                        let ueber = status == Some(true);
                        let r = ui.add_enabled(!ueber, egui::Checkbox::without_text(&mut an));
                        let r = r.on_disabled_hover_text(t("Mit dem Ordner darüber ausgewählt"));
                        if r.clicked() {
                            umschalten = Some(k.rel.clone());
                        }
                        if k.ordner {
                            stil::symbol(ui, Sym::Ordner, 15.0, akzent());
                            let txt = egui::RichText::new(&k.name).color(akzent());
                            if ui.add(egui::Label::new(txt).sense(egui::Sense::click())).on_hover_text(t("Öffnen")).clicked() {
                                oeffnen = Some(k.rel.clone());
                            }
                        } else {
                            stil::symbol(ui, Sym::Datei, 15.0, thema::muted());
                            ui.label(&k.name);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if k.zeit != 0 {
                                    ui.label(egui::RichText::new(lauf::zeit_text(k.zeit)).color(thema::muted()).small());
                                }
                                ui.label(egui::RichText::new(plan::groesse_text(k.groesse)).color(thema::muted()).small());
                            });
                        }
                    });
                }
            });
        });
        if let Some(r) = umschalten {
            self.wh.umschalten(&r);
        }
        if let Some(r) = oeffnen {
            self.wh.pfad = r;
            self.wh.filter.clear();
        }
        if self.wh.auswahl.is_empty() {
            ui.add_space(6.0);
            stil::hinweis(ui, t("Häkchen setzen, um Dateien oder Ordner wiederherzustellen."));
        }
    }

    // -------------------------------------------------------------- Fenster (Dialoge)

    fn fenster(&mut self, ctx: &egui::Context) {
        self.vorschau_fenster(ctx);
        // Ungespeicherte Änderungen beim Wechseln
        if let Some(ziel) = self.wechsel {
            let mut zu = false;
            egui::Window::new(t("Ungespeicherte Änderungen"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(t("Der Auftrag hat ungespeicherte Änderungen."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if stil::haupt(ui, true, Sym::Speichern, t("Speichern")).clicked() {
                            if self.speichern() {
                                self.nach_wechsel(ziel);
                            }
                            zu = true;
                        }
                        if stil::gefahr(ui, true, Sym::Papierkorb, t("Verwerfen")).clicked() {
                            self.nach_wechsel(ziel);
                            zu = true;
                        }
                        if stil::knopf_text(ui, true, t("Abbrechen")).clicked() {
                            self.neuer_nach_wechsel = false;
                            zu = true;
                        }
                    });
                });
            if zu {
                self.wechsel = None;
            }
        }
        // Auftrag löschen
        if let Some(id) = self.loeschen_frage {
            let name = self.daten.auftrag(id).map(|a| a.name.clone()).unwrap_or_default();
            let mut zu = false;
            egui::Window::new(t("Auftrag löschen?"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(tf("«{name}» wird gelöscht.", &[("name", &name)]));
                    ui.label(t("Die Sicherungen im Ziel bleiben erhalten; das Protokoll bleibt ebenfalls."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if stil::gefahr(ui, true, Sym::Papierkorb, t("Löschen")).clicked() {
                            self.auftrag_loeschen(id);
                            zu = true;
                        }
                        if stil::knopf_text(ui, true, t("Abbrechen")).clicked() {
                            zu = true;
                        }
                    });
                });
            if zu {
                self.loeschen_frage = None;
            }
        }
        // Schliessen
        if self.schliessen_frage {
            let mut zu = false;
            let laeuft = self.job.is_some();
            egui::Window::new(t("Sicherwerk schliessen?"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if laeuft {
                        ui.label(t("Es läuft gerade eine Sicherung oder Wiederherstellung."));
                    }
                    if self.dirty() {
                        ui.label(t("Der Auftrag hat ungespeicherte Änderungen."));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let text = if laeuft { t("Abbrechen und schliessen") } else { t("Ohne Speichern schliessen") };
                        if stil::haupt(ui, true, Sym::Pfeil, t("Weiterarbeiten")).clicked() {
                            zu = true;
                        }
                        if stil::gefahr(ui, true, Sym::Kreuz, text).clicked() {
                            if let Some(j) = &self.job {
                                j.fs.abbrechen();
                                self.schliessen_nach_job = true;
                            } else {
                                self.darf_schliessen = true;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            zu = true;
                        }
                    });
                });
            if zu {
                self.schliessen_frage = false;
            }
        }
    }

    fn nach_wechsel(&mut self, ziel: Option<u64>) {
        if self.neuer_nach_wechsel {
            self.neuer_nach_wechsel = false;
            self.waehlen(self.sel);
            self.neuer_auftrag();
        } else {
            self.waehlen(ziel);
        }
    }

    fn vorschau_fenster(&mut self, ctx: &egui::Context) {
        let Some((id, v)) = &self.vorschau else { return };
        let id = *id;
        let mut zu = false;
        let mut starten = false;
        let a = self.daten.auftrag(id).cloned();
        let entfernte = a.as_ref().map(|a| a.entfernte).unwrap_or(Entfernte::Verschieben);
        let name = a.as_ref().map(|a| a.name.clone()).unwrap_or_default();
        let mut liste_wahl = self.vorschau_liste;
        let job_frei = self.job.is_none();
        egui::Window::new(tf("Vorschau: {name}", &[("name", &name)]))
            .id(egui::Id::new("vorschau_fenster"))
            .collapsible(false)
            .resizable(true)
            .default_size([680.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if v.fatal {
                    ui.label(egui::RichText::new(t("So kann nicht gesichert werden:")).color(rot()).strong());
                } else if v.abgebrochen {
                    ui.label(egui::RichText::new(t("Abgebrochen – die Vorschau ist unvollständig.")).color(orange()));
                } else {
                    grau(ui, tf("Ziel: {pfad}", &[("pfad", &v.ordner)]));
                    ui.add_space(4.0);
                    egui::Grid::new("vorschau_werte").num_columns(2).spacing([16.0, 3.0]).show(ui, |ui| {
                        let zeile = |ui: &mut egui::Ui, k: &str, w: String| {
                            ui.label(egui::RichText::new(k).color(thema::muted()));
                            ui.label(egui::RichText::new(w).strong());
                            ui.end_row();
                        };
                        match v.art {
                            Some(Art::Versionen) => {
                                zeile(ui, t("Neue Version"), v.version.clone());
                                zeile(ui, t("Dateien"), v.dateien.to_string());
                                if v.verknuepfen > 0 {
                                    zeile(ui, t("Davon unverändert (verknüpft)"), v.verknuepfen.to_string());
                                }
                                zeile(ui, t("Zu kopieren"), format!("{} · {}", v.neu_anzahl, plan::groesse_text(v.bytes)));
                                if !v.versionen_weg.is_empty() {
                                    let liste: Vec<String> = v.versionen_weg.iter().map(|n| plan::datum_anzeige(n)).collect();
                                    zeile(ui, t("Alte Versionen, die entfernt werden"), liste.join(", "));
                                }
                            }
                            _ => {
                                zeile(ui, t("Neu"), v.neu_anzahl.to_string());
                                zeile(ui, t("Geändert"), v.ersetzen_anzahl.to_string());
                                let wie = match entfernte {
                                    Entfernte::Verschieben => t("Nach _entfernt verschieben"),
                                    Entfernte::Loeschen => t("Im Ziel löschen"),
                                    Entfernte::Behalten => t("Im Ziel nicht mehr vorhanden (bleiben)"),
                                };
                                let n = if entfernte == Entfernte::Behalten { 0 } else { v.entfernen_anzahl };
                                zeile(ui, wie, n.to_string());
                                zeile(ui, t("Unverändert"), v.gleich.to_string());
                                zeile(ui, t("Zu kopieren"), plan::groesse_text(v.bytes));
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut liste_wahl, 0, tf("Zu kopieren ({n})", &[("n", &v.neu_anzahl)]));
                        if v.art == Some(Art::Spiegeln) {
                            ui.selectable_value(&mut liste_wahl, 1, tf("Geändert ({n})", &[("n", &v.ersetzen_anzahl)]));
                            ui.selectable_value(&mut liste_wahl, 2, tf("Entfernen ({n})", &[("n", &v.entfernen_anzahl)]));
                        }
                    });
                    let zeilen: Vec<String> = match liste_wahl {
                        1 => v.ersetzen.iter().map(|(p, g)| format!("{p}   ({})", plan::groesse_text(*g))).collect(),
                        2 => v.entfernen.clone(),
                        _ => v.neu.iter().map(|(p, g)| format!("{p}   ({})", plan::groesse_text(*g))).collect(),
                    };
                    let h = ui.text_style_height(&egui::TextStyle::Small) + 4.0;
                    egui::Frame::none().fill(thema::bg()).rounding(6.0).inner_margin(egui::Margin::same(6.0)).show(ui, |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(220.0).show_rows(ui, h, zeilen.len(), |ui, bereich| {
                            for z in zeilen.get(bereich).unwrap_or(&[]) {
                                ui.label(egui::RichText::new(z).small().monospace());
                            }
                        });
                    });
                    let gezeigt = match liste_wahl {
                        1 => (v.ersetzen.len() as u64, v.ersetzen_anzahl),
                        2 => (v.entfernen.len() as u64, v.entfernen_anzahl),
                        _ => (v.neu.len() as u64, v.neu_anzahl),
                    };
                    if gezeigt.0 < gezeigt.1 {
                        grau(ui, tf("(die ersten {n} von {m})", &[("n", &gezeigt.0), ("m", &gezeigt.1)]));
                    }
                }
                if !v.fehler.is_empty() {
                    ui.add_space(6.0);
                    ui.push_id("vorschau_fehler", |ui| {
                        egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                            for f in v.fehler.iter().take(100) {
                                let c = if f.art == FehlerArt::Hinweis { thema::muted() } else { rot() };
                                ui.label(egui::RichText::new(fehler_text(f)).color(c).small());
                            }
                        });
                    });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let darf = !v.fatal && !v.abgebrochen && job_frei && a.is_some();
                    if stil::haupt(ui, darf, Sym::Speichern, t("Sichern starten")).clicked() {
                        starten = true;
                    }
                    if stil::knopf_text(ui, true, t("Schliessen")).clicked() {
                        zu = true;
                    }
                });
            });
        self.vorschau_liste = liste_wahl;
        if starten {
            self.vorschau = None;
            self.vorschau_gesehen_setzen(id);
            if let Some(a) = self.daten.auftrag(id).cloned() {
                self.sichern_starten(a, Ausloeser::Hand);
            }
        } else if zu {
            // Auch nur angesehen zählt: danach darf ohne Vorschau gesichert werden.
            if self.vorschau.as_ref().is_some_and(|(_, v)| !v.fatal && !v.abgebrochen) {
                self.vorschau_gesehen_setzen(id);
            }
            self.vorschau = None;
        }
    }
}
