//! Oberfläche «Doppelte Dateien»: Ordner wählen, im Hintergrund suchen, Gruppen als
//! virtualisierte Liste zeigen, je Gruppe eine Datei behalten und den Rest nach Rückfrage in den
//! Papierkorb verschieben. Die eigentliche Arbeit macht `doppelt` (ohne Oberfläche, getestet).

use crate::doppelt::{self, Auftrag, Fortschritt, Grund, Gruppe, Loeschergebnis, Optionen, Phase, Regel, Statistik};
use crate::scan::{fmt_size, group_thousands};
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::thema;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Suchergebnis = Option<(Vec<Gruppe>, Statistik)>;

enum Arbeit {
    Suche(Receiver<Suchergebnis>),
    Loeschen(Receiver<Vec<Loeschergebnis>>),
}

#[derive(Clone, Copy)]
enum Zeile {
    Kopf(usize),
    Datei(usize, usize),
}

struct Bericht {
    verschoben: u64,
    bytes: u64,
    nicht: Vec<(PathBuf, Grund)>,
}

const ZEILE: f32 = 26.0;

pub struct Doppelt {
    ordner: Option<PathBuf>,
    unterordner: bool,
    min_groesse: u64,
    regel: Regel,
    /// Einstellungen der letzten Suche (Unterordner, Mindestgrösse)
    gesucht_mit: Option<(bool, u64)>,
    gruppen: Vec<Gruppe>,
    /// je Gruppe und Datei: true = in den Papierkorb
    auswahl: Vec<Vec<bool>>,
    zeilen: Vec<Zeile>,
    stat: Option<Statistik>,
    fp: Arc<Fortschritt>,
    arbeit: Option<Arbeit>,
    gestartet: Option<Instant>,
    status: String,
    rueckfrage: bool,
    bericht: Option<Bericht>,
}

impl Doppelt {
    pub fn new() -> Self {
        Doppelt {
            ordner: None,
            unterordner: true,
            min_groesse: 1,
            regel: Regel::KuerzesterPfad,
            gesucht_mit: None,
            gruppen: Vec::new(),
            auswahl: Vec::new(),
            zeilen: Vec::new(),
            stat: None,
            fp: Arc::new(Fortschritt::new()),
            arbeit: None,
            gestartet: None,
            status: t("Ordner wählen, um doppelte Dateien zu finden.").into(),
            rueckfrage: false,
            bericht: None,
        }
    }

    pub fn beschaeftigt(&self) -> bool {
        self.arbeit.is_some()
    }

    /// Ordner aus der Speicherplatz-Ansicht übernehmen, solange hier noch keiner gewählt ist.
    pub fn ordner_vorschlagen(&mut self, p: Option<PathBuf>) {
        if self.ordner.is_none() && !self.beschaeftigt() {
            self.ordner = p;
        }
    }

    fn ordner_waehlen(&mut self) {
        let mut d = rfd::FileDialog::new();
        if let Some(o) = &self.ordner {
            d = d.set_directory(o);
        }
        if let Some(o) = d.pick_folder() {
            self.ordner = Some(o);
            self.suche_starten();
        }
    }

    fn suche_starten(&mut self) {
        let Some(ordner) = self.ordner.clone() else { return };
        if self.beschaeftigt() {
            return;
        }
        let fp = Arc::new(Fortschritt::new());
        let opt = Optionen { unterordner: self.unterordner, min_groesse: self.min_groesse, ..Optionen::default() };
        let (tx, rx) = channel();
        let f = fp.clone();
        let start = std::thread::Builder::new().name("doppelt-suche".into()).spawn(move || {
            let r = doppelt::suchen(&ordner, &opt, &f);
            let _ = tx.send(r);
        });
        if let Err(e) = start {
            self.status = tf("Suche liess sich nicht starten: {e}", &[("e", &e)]);
            return;
        }
        self.fp = fp;
        self.arbeit = Some(Arbeit::Suche(rx));
        self.gesucht_mit = Some((self.unterordner, self.min_groesse));
        self.gruppen.clear();
        self.auswahl.clear();
        self.zeilen.clear();
        self.stat = None;
        self.bericht = None;
        self.gestartet = Some(Instant::now());
        self.status = t("Wird gesucht …").into();
    }

    fn loeschen_starten(&mut self) {
        if self.beschaeftigt() {
            return;
        }
        let auftraege: Vec<Auftrag> = self
            .gruppen
            .iter()
            .zip(&self.auswahl)
            .filter(|(g, a)| a.iter().any(|l| *l) && a.iter().any(|l| !*l) && a.len() == g.eintraege.len())
            .map(|(g, a)| Auftrag { gruppe: g.clone(), loeschen: a.clone() })
            .collect();
        if auftraege.is_empty() {
            return;
        }
        let fp = Arc::new(Fortschritt::new());
        let (tx, rx) = channel();
        let f = fp.clone();
        let start = std::thread::Builder::new().name("doppelt-papierkorb".into()).spawn(move || {
            // Nur in den Papierkorb, nie endgültig löschen.
            let r = doppelt::loeschen(&auftraege, &f, &mut |p: &Path| trash::delete(p).map_err(|e| e.to_string()));
            let _ = tx.send(r);
        });
        if let Err(e) = start {
            self.status = tf("Papierkorb liess sich nicht starten: {e}", &[("e", &e)]);
            return;
        }
        self.fp = fp;
        self.arbeit = Some(Arbeit::Loeschen(rx));
        self.gestartet = Some(Instant::now());
        self.status = t("Wird geprüft und verschoben …").into();
    }

    /// Ergebnis des Hintergrund-Threads abholen (jedes Bild, auch in der anderen Ansicht).
    pub fn poll(&mut self) {
        let Some(arbeit) = &self.arbeit else { return };
        let secs = format!("{:.1}", self.gestartet.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0));
        match arbeit {
            Arbeit::Suche(rx) => match rx.try_recv() {
                Err(TryRecvError::Empty) => {}
                Ok(Some((gruppen, stat))) => {
                    self.arbeit = None;
                    self.gruppen = gruppen;
                    self.stat = Some(stat);
                    self.regel_anwenden();
                    self.status = tf("{n} Gruppen gefunden in {secs} s", &[("n", &group_thousands(self.gruppen.len() as u64)), ("secs", &secs)]);
                }
                Ok(None) => {
                    self.arbeit = None;
                    self.status = t("Abgebrochen.").into();
                }
                Err(TryRecvError::Disconnected) => {
                    self.arbeit = None;
                    self.status = t("Die Suche ist unerwartet beendet worden.").into();
                }
            },
            Arbeit::Loeschen(rx) => match rx.try_recv() {
                Err(TryRecvError::Empty) => {}
                Ok(r) => {
                    self.arbeit = None;
                    self.nach_loeschen(r);
                }
                Err(TryRecvError::Disconnected) => {
                    self.arbeit = None;
                    self.status = t("Das Verschieben ist unerwartet beendet worden. Bitte neu suchen.").into();
                    self.gruppen.clear();
                    self.auswahl.clear();
                    self.zeilen.clear();
                }
            },
        }
    }

    fn nach_loeschen(&mut self, r: Vec<Loeschergebnis>) {
        let mut b = Bericht { verschoben: 0, bytes: 0, nicht: Vec::new() };
        let mut weg = HashSet::new();
        for x in r {
            match x.erfolg {
                Ok(()) => {
                    b.verschoben = b.verschoben.saturating_add(1);
                    b.bytes = b.bytes.saturating_add(x.bytes);
                    weg.insert(x.pfad);
                }
                Err(g) => b.nicht.push((x.pfad, g)),
            }
        }
        doppelt::nach_loeschen(&mut self.gruppen, &weg);
        self.regel_anwenden();
        self.status = tf(
            "{n} Dateien in den Papierkorb verschoben ({size}), {m} nicht",
            &[("n", &group_thousands(b.verschoben)), ("size", &fmt_size(b.bytes)), ("m", &group_thousands(b.nicht.len() as u64))],
        );
        self.bericht = Some(b);
    }

    /// Auswahl aller Gruppen nach der gewählten Regel (manuelle Änderungen gehen verloren).
    fn regel_anwenden(&mut self) {
        self.auswahl = self.gruppen.iter().map(|g| doppelt::auswahl_nach_regel(g, self.regel)).collect();
        self.zeilen.clear();
        for (i, g) in self.gruppen.iter().enumerate() {
            self.zeilen.push(Zeile::Kopf(i));
            for j in 0..g.eintraege.len() {
                self.zeilen.push(Zeile::Datei(i, j));
            }
        }
    }

    /// (Dateien, Bytes) der markierten Dateien.
    fn markiert(&self) -> (u64, u64) {
        let mut n = 0u64;
        let mut b = 0u64;
        for (g, a) in self.gruppen.iter().zip(&self.auswahl) {
            for (e, l) in g.eintraege.iter().zip(a) {
                if *l {
                    n = n.saturating_add(1 + e.links.len() as u64);
                    b = b.saturating_add(g.groesse);
                }
            }
        }
        (n, b)
    }

    fn csv_speichern(&mut self) {
        let Some(p) = rfd::FileDialog::new().set_file_name("doppelte-dateien.csv").save_file() else { return };
        let text = doppelt::csv(&self.gruppen, &self.auswahl, [t("Gruppe;Bytes;SHA-256;Aktion;Pfad"), t("behalten"), t("Papierkorb"), t("harter Link")]);
        self.status = match std::fs::write(&p, text) {
            Ok(()) => tf("Gespeichert: {p}", &[("p", &p.display())]),
            Err(e) => tf("CSV nicht gespeichert: {e}", &[("e", &e)]),
        };
    }

    // --------------------------------------------------------------------------- Oberfläche

    /// Knöpfe in der Kopfleiste (nach den Reitern).
    pub fn kopf(&mut self, ui: &mut egui::Ui) {
        if stil::knopf_an(ui, !self.beschaeftigt(), Sym::OrdnerAuf, t("Ordner wählen")).clicked() {
            self.ordner_waehlen();
        }
        if let Some(r) = &self.ordner {
            ui.add(egui::Label::new(egui::RichText::new(r.display().to_string()).color(thema::muted())).truncate());
        }
        let darf = !self.gruppen.is_empty() && !self.beschaeftigt();
        stil::rechts(ui, |ui| {
            if stil::leise(ui, darf, Sym::Download, t("CSV speichern …"), t("Liste der doppelten Dateien als CSV")).clicked() {
                self.csv_speichern();
            }
        });
    }

    /// Fussleiste, Seitenleiste, Liste und Fenster.
    pub fn zeigen(&mut self, ctx: &egui::Context) {
        if self.beschaeftigt() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.fuss(ctx);
        self.seite(ctx);
        stil::mitte(ctx, |ui| self.liste(ui));
        self.fenster(ctx);
    }

    fn fortschritt_text(&self) -> String {
        let fp = &self.fp;
        let n = |a: &std::sync::atomic::AtomicU64| group_thousands(a.load(Ordering::Relaxed));
        let prozent = || {
            let g = fp.bytes_gesamt.load(Ordering::Relaxed);
            let b = fp.bytes.load(Ordering::Relaxed).min(g);
            if g == 0 {
                0
            } else {
                (b as f64 / g as f64 * 100.0).round() as u64
            }
        };
        match fp.phase() {
            Phase::Sammeln => tf("Sammle Dateien … {files} Dateien · {dirs} Ordner", &[("files", &n(&fp.dateien)), ("dirs", &n(&fp.ordner))]),
            Phase::Vorpruefen => tf("Vergleiche Anfang und Ende … {n} von {m} Dateien", &[("n", &n(&fp.erledigt)), ("m", &n(&fp.gesamt))]),
            Phase::Vergleichen => tf(
                "Vergleiche Inhalte … {n} von {m} Dateien · {p} %",
                &[("n", &n(&fp.erledigt)), ("m", &n(&fp.gesamt)), ("p", &prozent())],
            ),
            Phase::Loeschen => tf("Prüfe und verschiebe … {n} von {m}", &[("n", &n(&fp.erledigt)), ("m", &n(&fp.gesamt))]),
            Phase::Fertig => t("Wird abgeschlossen …").into(),
        }
    }

    fn fuss(&mut self, ctx: &egui::Context) {
        let (n, b) = self.markiert();
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.beschaeftigt() {
                    ui.spinner();
                    stil::grau(ui, &self.fortschritt_text());
                } else {
                    stil::grau(ui, &self.status);
                }
                stil::rechts(ui, |ui| {
                    if self.beschaeftigt() {
                        if stil::knopf(ui, Sym::Stopp, t("Abbrechen")).clicked() {
                            self.fp.abbrechen.store(true, Ordering::Relaxed);
                        }
                    } else if self.ordner.is_none() {
                        if stil::haupt(ui, true, Sym::OrdnerAuf, t("Ordner wählen")).clicked() {
                            self.ordner_waehlen();
                        }
                    } else {
                        if stil::haupt(ui, n > 0, Sym::Papierkorb, t("In den Papierkorb …")).clicked() {
                            self.rueckfrage = true;
                        }
                        let wort = if self.gesucht_mit.is_some() { t("Neu suchen") } else { t("Suchen") };
                        if stil::knopf(ui, Sym::Suche, wort).clicked() {
                            self.suche_starten();
                        }
                        if n > 0 {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(tf("{n} markiert · {size}", &[("n", &group_thousands(n)), ("size", &fmt_size(b))])).strong());
                        }
                    }
                });
            });
        });
    }

    fn seite(&mut self, ctx: &egui::Context) {
        let frei = !self.beschaeftigt();
        stil::seitenleiste(ctx, 300.0, |ui| {
            stil::karte(ui, "doppelt-suche", Sym::Suche, t("Suche"), true, |ui| {
                ui.add_enabled_ui(frei, |ui| {
                    stil::schalter(ui, &mut self.unterordner, t("mit Unterordnern"));
                    stil::feld(ui, t("Mindestgrösse"), |ui| {
                        stil::liste(
                            ui,
                            "doppelt-min",
                            &mut self.min_groesse,
                            &[
                                (1, t("alle Dateien (ausser leere)")),
                                (1024, t("ab 1 KB")),
                                (100 * 1024, t("ab 100 KB")),
                                (1024 * 1024, t("ab 1 MB")),
                                (10 * 1024 * 1024, t("ab 10 MB")),
                                (100 * 1024 * 1024, t("ab 100 MB")),
                            ],
                        );
                    });
                });
                stil::hinweis(ui, t("Gleicher Inhalt zählt, nicht der Name. Leere Dateien, Verknüpfungen und harte Links gelten nie als doppelt."));
                if self.gesucht_mit.is_some_and(|g| g != (self.unterordner, self.min_groesse)) {
                    stil::hinweis(ui, t("Geänderte Einstellungen gelten ab der nächsten Suche («Neu suchen»)."));
                }
            });
            stil::karte(ui, "doppelt-behalten", Sym::Haken, t("Behalten"), true, |ui| {
                ui.add_enabled_ui(frei, |ui| {
                    let vorher = self.regel;
                    stil::liste(
                        ui,
                        "doppelt-regel",
                        &mut self.regel,
                        &[
                            (Regel::KuerzesterPfad, t("kürzester Pfad behalten")),
                            (Regel::Aelteste, t("älteste behalten")),
                            (Regel::Neueste, t("neueste behalten")),
                        ],
                    );
                    if vorher != self.regel {
                        self.regel_anwenden();
                    }
                    ui.horizontal_wrapped(|ui| {
                        if stil::knopf_an(ui, !self.gruppen.is_empty(), Sym::Rueckgaengig, t("Regel neu anwenden")).clicked() {
                            self.regel_anwenden();
                        }
                        if stil::knopf_an(ui, !self.gruppen.is_empty(), Sym::Kreuz, t("Nichts markieren")).clicked() {
                            for a in &mut self.auswahl {
                                a.iter_mut().for_each(|l| *l = false);
                            }
                        }
                    });
                });
                stil::hinweis(ui, t("Je Gruppe bleibt mindestens eine Datei. Mit den Häkchen lässt sich die Wahl pro Datei ändern."));
            });
            stil::karte(ui, "doppelt-ergebnis", Sym::Info, t("Ergebnis"), true, |ui| {
                let gesamt: u64 = self.gruppen.iter().fold(0u64, |a, g| a.saturating_add(g.freigebbar()));
                let dateien: u64 = self.gruppen.iter().map(|g| g.eintraege.len() as u64).sum();
                let (n, b) = self.markiert();
                zahl(ui, t("Gruppen"), &group_thousands(self.gruppen.len() as u64));
                zahl(ui, t("Dateien in Gruppen"), &group_thousands(dateien));
                zahl(ui, t("Freigebbar"), &fmt_size(gesamt));
                zahl(ui, t("Markiert"), &tf("{n} · {size}", &[("n", &group_thousands(n)), ("size", &fmt_size(b))]));
                if let Some(s) = &self.stat {
                    ui.separator();
                    zahl(ui, t("Dateien durchsucht"), &group_thousands(s.dateien));
                    zahl(ui, t("Ordner durchsucht"), &group_thousands(s.ordner));
                    if s.zu_klein > 0 {
                        zahl(ui, t("Zu klein / leer"), &group_thousands(s.zu_klein));
                    }
                    if s.unlesbar > 0 {
                        zahl(ui, t("Ordner ohne Zugriff"), &group_thousands(s.unlesbar));
                    }
                    if s.fehler > 0 {
                        zahl(ui, t("Dateien nicht lesbar"), &group_thousands(s.fehler));
                    }
                    if s.verknuepfungen > 0 {
                        zahl(ui, t("Verknüpfungen übergangen"), &group_thousands(s.verknuepfungen));
                    }
                    if s.zu_tief > 0 {
                        zahl(ui, t("Ordner zu tief verschachtelt"), &group_thousands(s.zu_tief));
                    }
                    if s.limit_erreicht {
                        ui.colored_label(stil::orange(), t("Sehr viele Dateien: die Suche hat nur einen Teil berücksichtigt."));
                    }
                }
                stil::hinweis(ui, t("Der Platz wird erst frei, wenn der Papierkorb geleert wird."));
            });
        });
    }

    fn liste(&mut self, ui: &mut egui::Ui) {
        if self.gruppen.is_empty() {
            if self.beschaeftigt() {
                stil::leer(ui, Sym::Suche, t("Wird gesucht …"), "");
            } else if self.gesucht_mit.is_some() {
                stil::leer(ui, Sym::Haken, t("Keine doppelten Dateien"), t("Im gewählten Ordner hat keine Datei denselben Inhalt wie eine andere."));
            } else {
                stil::leer(ui, Sym::Kopieren, t("Doppelte Dateien finden"), t("Oben einen Ordner wählen. Verglichen wird der Inhalt, nicht der Name."));
            }
            return;
        }
        let frei = !self.beschaeftigt();
        let zeilen = self.zeilen.len();
        let gruppen = &self.gruppen;
        let auswahl = &mut self.auswahl;
        let alle = &self.zeilen;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ZEILE, zeilen, |ui, bereich| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for z in alle.get(bereich).unwrap_or_default() {
                match *z {
                    Zeile::Kopf(i) => {
                        if let Some(g) = gruppen.get(i) {
                            kopf_zeile(ui, g);
                        }
                    }
                    Zeile::Datei(i, j) => {
                        if let (Some(g), Some(a)) = (gruppen.get(i), auswahl.get_mut(i)) {
                            datei_zeile(ui, g, a, j, frei);
                        }
                    }
                }
            }
        });
    }

    fn fenster(&mut self, ctx: &egui::Context) {
        if self.rueckfrage {
            let (n, b) = self.markiert();
            let mut ja = false;
            let mut nein = false;
            egui::Window::new(t("In den Papierkorb verschieben?"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(egui::RichText::new(tf("{n} Dateien ({size}) kommen in den Papierkorb.", &[("n", &group_thousands(n)), ("size", &fmt_size(b))])).strong());
                    ui.add_space(4.0);
                    stil::hinweis(ui, t("Vorher wird jede Datei noch einmal mit der behaltenen verglichen. Geänderte oder fehlende Dateien werden übersprungen. Endgültig gelöscht wird nichts."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if stil::gefahr(ui, n > 0, Sym::Papierkorb, t("In den Papierkorb")).clicked() {
                            ja = true;
                        }
                        if stil::knopf(ui, Sym::Kreuz, t("Abbrechen")).clicked() {
                            nein = true;
                        }
                    });
                });
            if ja {
                self.rueckfrage = false;
                self.loeschen_starten();
            } else if nein || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.rueckfrage = false;
            }
        }
        let mut zu = false;
        if let Some(b) = &self.bericht {
            egui::Window::new(t("Ergebnis"))
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_max_width(560.0);
                    ui.label(
                        egui::RichText::new(tf("{n} Dateien in den Papierkorb verschoben ({size}).", &[("n", &group_thousands(b.verschoben)), ("size", &fmt_size(b.bytes))]))
                            .strong(),
                    );
                    if !b.nicht.is_empty() {
                        ui.add_space(4.0);
                        ui.colored_label(stil::orange(), tf("{n} Dateien nicht verschoben:", &[("n", &group_thousands(b.nicht.len() as u64))]));
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            for (p, g) in &b.nicht {
                                ui.horizontal(|ui| {
                                    ui.add(egui::Label::new(egui::RichText::new(grund_text(g)).color(thema::muted())));
                                    ui.add(egui::Label::new(p.display().to_string()).truncate());
                                });
                            }
                        });
                    }
                    ui.add_space(8.0);
                    if stil::knopf(ui, Sym::Haken, t("Schliessen")).clicked() {
                        zu = true;
                    }
                });
        }
        if zu {
            self.bericht = None;
        }
    }
}

fn grund_text(g: &Grund) -> String {
    match g {
        Grund::Fehlt => t("Datei fehlt").into(),
        Grund::Geaendert => t("seit der Suche geändert").into(),
        Grund::KeinPartner => t("behaltene Datei fehlt oder wurde geändert").into(),
        Grund::GleicheDatei => t("ist jetzt dieselbe Datei wie die behaltene").into(),
        Grund::Lesefehler(e) => tf("nicht lesbar: {e}", &[("e", e)]),
        Grund::Papierkorb(e) => tf("Papierkorb ging nicht: {e}", &[("e", e)]),
        Grund::Abgebrochen => t("abgebrochen").into(),
    }
}

fn zahl(ui: &mut egui::Ui, name: &str, wert: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(name).color(thema::muted()));
        stil::rechts(ui, |ui| {
            ui.label(egui::RichText::new(wert).strong());
        });
    });
}

fn zeile(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(egui::vec2(w, ZEILE), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(egui::vec2(w, ZEILE));
        ui.set_max_height(ZEILE);
        add(ui);
    });
}

fn kopf_zeile(ui: &mut egui::Ui, g: &Gruppe) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, ZEILE), egui::Sense::hover());
    let innen = rect.shrink2(egui::vec2(0.0, 2.0));
    ui.painter().rect_filled(innen, 6.0, stil::toenung());
    let mut kind = ui.new_child(egui::UiBuilder::new().max_rect(innen.shrink2(egui::vec2(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let n = g.eintraege.len() as u64;
    kind.label(egui::RichText::new(tf("{n} gleiche Dateien zu je {size}", &[("n", &group_thousands(n)), ("size", &fmt_size(g.groesse))])).strong());
    kind.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new(tf("{size} freigebbar", &[("size", &fmt_size(g.freigebbar()))])).color(stil::akzent()).strong());
    });
}

fn datei_zeile(ui: &mut egui::Ui, g: &Gruppe, auswahl: &mut [bool], j: usize, frei: bool) {
    let Some(e) = g.eintraege.get(j) else { return };
    let mut l = auswahl.get(j).copied().unwrap_or(false);
    let erlaubt = l || doppelt::darf_loeschen(auswahl, j);
    let mut umschalten = false;
    zeile(ui, |ui| {
        ui.add_space(10.0);
        let r = ui
            .add_enabled(frei && erlaubt, egui::Checkbox::without_text(&mut l))
            .on_hover_text(t("Markiert = kommt in den Papierkorb"))
            .on_disabled_hover_text(t("Mindestens eine Datei der Gruppe bleibt erhalten."));
        if r.changed() {
            umschalten = true;
        }
        if l {
            stil::marke(ui, t("PAPIERKORB"), stil::rot(), 88.0);
        } else {
            stil::marke(ui, t("BEHALTEN"), stil::gruen(), 88.0);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(doppelt::datum(e.geaendert)).on_hover_text(t("Zuletzt geändert (UTC)"));
            if !e.links.is_empty() {
                let liste: Vec<String> = e.links.iter().map(|p| p.display().to_string()).collect();
                ui.label(egui::RichText::new(tf("+{n} harte Links", &[("n", &e.links.len())])).color(thema::muted())).on_hover_text(tf(
                    "Derselbe Dateieintrag unter weiteren Namen (kein Doppel, belegt keinen zusätzlichen Platz). Wird die Datei verschoben, kommen diese Namen mit:\n{liste}",
                    &[("liste", &liste.join("\n"))],
                ));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let farbe = if l { thema::muted() } else { thema::text() };
                ui.add(egui::Label::new(egui::RichText::new(e.pfad.display().to_string()).color(farbe)).truncate()).on_hover_text(e.pfad.display().to_string());
            });
        });
    });
    if umschalten {
        doppelt::umschalten(auswahl, j);
    }
}
