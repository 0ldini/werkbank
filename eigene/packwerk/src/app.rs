//! Oberfläche (egui): Archiv anzeigen (Ordner-Ansicht mit Liste), entpacken, neues Archiv
//! zusammenstellen und erstellen. Lange Arbeit läuft im Hintergrund (archiv.rs) und wird
//! hier abgefragt; Rückfragen (Passwort, vorhandene Dateien, Archiv-Bombe, Überschreiben)
//! erscheinen als kleine Fenster.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use crate::anzeige::{self, Baum, Blatt, Sortierung};
use crate::archiv::{
    self, Bericht, EintragArt, EntpackAuftrag, Fehler, Fortschritt, Inhalt, Konflikt, PackAuftrag,
    PackBericht, Stufe,
};
use crate::regeln::{self, Art, Start, StartFehler, Warnung};
use crate::sprache::{t, tf};
use crate::thema;

const ZEILE: f32 = 24.0;
const BREITE_GROESSE: f32 = 92.0;
const BREITE_GEPACKT: f32 = 92.0;
const BREITE_ZEIT: f32 = 132.0;

fn akzent() -> egui::Color32 {
    thema::pick(
        egui::Color32::from_rgb(0xb4, 0x53, 0x09),
        egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
    )
}
fn akzent_weich() -> egui::Color32 {
    thema::pick(
        egui::Color32::from_rgb(0xfe, 0xf3, 0xc7),
        egui::Color32::from_rgb(0x45, 0x32, 0x0d),
    )
}
fn auf_akzent() -> egui::Color32 {
    thema::pick(
        egui::Color32::WHITE,
        egui::Color32::from_rgb(0x17, 0x18, 0x1C),
    )
}
fn rot() -> egui::Color32 {
    thema::pick(
        egui::Color32::from_rgb(0xb9, 0x1c, 0x1c),
        egui::Color32::from_rgb(0xf8, 0x71, 0x71),
    )
}
fn gruen() -> egui::Color32 {
    thema::pick(
        egui::Color32::from_rgb(0x15, 0x80, 0x3d),
        egui::Color32::from_rgb(0x4a, 0xde, 0x80),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Modus {
    Start,
    Ansicht,
    Packen,
}

enum Ergebnis {
    Gelesen(PathBuf, Option<String>, Result<Inhalt, Fehler>),
    Entpackt(Box<EntpackAuftrag>, Result<Bericht, Fehler>),
    Gepackt(Box<PackAuftrag>, Result<PackBericht, Fehler>),
}

struct Lauf {
    fp: Arc<Fortschritt>,
    rx: Receiver<Ergebnis>,
    titel: &'static str,
}

enum Dialog {
    PasswortLesen {
        pfad: PathBuf,
        eingabe: String,
        falsch: bool,
        fokus: bool,
    },
    PasswortEntpacken {
        auftrag: Box<EntpackAuftrag>,
        eingabe: String,
        falsch: bool,
        fokus: bool,
    },
    Konflikt {
        auftrag: Box<EntpackAuftrag>,
        anzahl: usize,
        beispiele: Vec<PathBuf>,
    },
    Bombe {
        auftrag: Box<EntpackAuftrag>,
        warnungen: Vec<String>,
    },
    Ueberschreiben {
        auftrag: Box<PackAuftrag>,
    },
    Fertig {
        titel: String,
        zeilen: Vec<String>,
        details: Vec<String>,
        ordner: Option<PathBuf>,
    },
    Meldung {
        titel: String,
        text: String,
    },
    Hilfe,
}

/// Was der Benutzer im Dialog gedrückt hat.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wahl {
    Ok,
    Abbrechen,
    Ueberschreiben,
    Umbenennen,
    Ueberspringen,
    OrdnerZeigen,
}

pub struct App {
    ctx: egui::Context,
    modus: Modus,

    // Ansicht
    inhalt: Option<Inhalt>,
    baum: Option<Baum>,
    ordner: usize,
    auswahl: HashSet<usize>,
    anker: Option<usize>,
    sortierung: Sortierung,
    absteigend: bool,
    ziel: String,
    passwort: Option<String>,
    /// Nach dem Lesen sofort alles hierhin entpacken (Kommandozeile `--entpacken`).
    danach_entpacken: Option<PathBuf>,

    // Neues Archiv
    quellen: Vec<PathBuf>,
    pack_art: Art,
    stufe: Stufe,
    pack_pw: String,
    pack_pw2: String,
    pw_zeigen: bool,
    pack_name: String,
    pack_name_auto: bool,
    pack_ordner: Option<PathBuf>,

    lauf: Option<Lauf>,
    dialog: Option<Dialog>,
    status: String,
    status_fehler: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start: Result<Start, StartFehler>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for f in style.text_styles.values_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        let mut app = App {
            ctx: cc.egui_ctx.clone(),
            modus: Modus::Start,
            inhalt: None,
            baum: None,
            ordner: 0,
            auswahl: HashSet::new(),
            anker: None,
            sortierung: Sortierung::Name,
            absteigend: false,
            ziel: String::new(),
            passwort: None,
            danach_entpacken: None,
            quellen: Vec::new(),
            pack_art: Art::Zip,
            stufe: Stufe::Normal,
            pack_pw: String::new(),
            pack_pw2: String::new(),
            pw_zeigen: false,
            pack_name: String::new(),
            pack_name_auto: true,
            pack_ordner: None,
            lauf: None,
            dialog: None,
            status: String::new(),
            status_fehler: false,
        };
        match start {
            Ok(Start::Leer) => {}
            Ok(Start::Oeffnen(p)) => app.oeffnen(p),
            Ok(Start::Entpacken { archiv, ziel }) => {
                let ziel = ziel.unwrap_or_else(|| regeln::standard_ziel(&archiv, t("Entpackt")));
                app.danach_entpacken = Some(ziel);
                app.oeffnen(archiv);
            }
            Ok(Start::Packen(v)) => {
                app.modus = Modus::Packen;
                app.quellen_dazu(v);
            }
            Ok(Start::Hilfe) => app.dialog = Some(Dialog::Hilfe),
            Err(f) => {
                let text = match f {
                    StartFehler::ArchivFehlt => t("Nach --entpacken fehlt das Archiv.").to_string(),
                    StartFehler::ZuVieleAngaben => {
                        t("Zu viele Angaben. Aufruf: packwerk --entpacken <archiv> [ziel]")
                            .to_string()
                    }
                    StartFehler::UnbekannteOption(o) => tf("Unbekannte Option: {o}", &[("o", &o)]),
                };
                app.dialog = Some(Dialog::Meldung {
                    titel: t("Kommandozeile").to_string(),
                    text,
                });
            }
        }
        app
    }

    fn beschaeftigt(&self) -> bool {
        self.lauf.is_some()
    }

    fn melden(&mut self, text: impl Into<String>, fehler: bool) {
        self.status = text.into();
        self.status_fehler = fehler;
    }

    // -----------------------------------------------------------------------
    // Hintergrund-Aufträge
    // -----------------------------------------------------------------------

    fn starten(
        &mut self,
        titel: &'static str,
        arbeit: impl FnOnce(&Fortschritt) -> Ergebnis + Send + 'static,
    ) {
        let fp = Arc::new(Fortschritt::default());
        let (tx, rx) = mpsc::channel();
        let fp2 = Arc::clone(&fp);
        let ctx = self.ctx.clone();
        let gestartet = std::thread::Builder::new()
            .name("packwerk-arbeit".into())
            .spawn(move || {
                let e = arbeit(&fp2);
                let _ = tx.send(e);
                ctx.request_repaint();
            });
        match gestartet {
            Ok(_) => {
                self.lauf = Some(Lauf { fp, rx, titel });
                self.melden(titel, false);
            }
            Err(e) => self.melden(tf("Konnte nicht starten: {e}", &[("e", &e)]), true),
        }
    }

    fn abfragen(&mut self) {
        let Some(lauf) = &self.lauf else { return };
        let ergebnis = match lauf.rx.try_recv() {
            Ok(e) => e,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.lauf = None;
                self.melden(t("Der Auftrag wurde unerwartet beendet."), true);
                return;
            }
        };
        self.lauf = None;
        match ergebnis {
            Ergebnis::Gelesen(pfad, pw, r) => self.gelesen(pfad, pw, r),
            Ergebnis::Entpackt(a, r) => self.entpackt(a, r),
            Ergebnis::Gepackt(a, r) => self.gepackt(a, r),
        }
    }

    // -----------------------------------------------------------------------
    // Öffnen
    // -----------------------------------------------------------------------

    fn oeffnen_dialog(&mut self) {
        let mut d = rfd::FileDialog::new()
            .set_title(t("Archiv öffnen"))
            .add_filter(t("Archive"), &regeln::ENDUNGEN)
            .add_filter(t("Alle Dateien"), &["*"]);
        if let Some(p) = self
            .inhalt
            .as_ref()
            .and_then(|i| i.pfad.parent().map(Path::to_path_buf))
        {
            d = d.set_directory(p);
        }
        if let Some(p) = d.pick_file() {
            self.oeffnen(p);
        }
    }

    fn oeffnen(&mut self, pfad: PathBuf) {
        self.oeffnen_mit(pfad, None);
    }

    fn oeffnen_mit(&mut self, pfad: PathBuf, passwort: Option<String>) {
        if self.beschaeftigt() {
            return;
        }
        let pw = passwort;
        self.starten(t("Archiv wird gelesen …"), move |fp| {
            let r = archiv::lesen(&pfad, pw.as_deref(), fp);
            Ergebnis::Gelesen(pfad, pw, r)
        });
    }

    fn gelesen(&mut self, pfad: PathBuf, pw: Option<String>, r: Result<Inhalt, Fehler>) {
        match r {
            Ok(inhalt) => {
                let blaetter: Vec<Blatt> = inhalt
                    .eintraege
                    .iter()
                    .map(|e| Blatt {
                        teile: regeln::anzeige_teile(&e.name),
                        ist_ordner: e.art == EintragArt::Ordner,
                        groesse: e.groesse,
                        gepackt: e.gepackt,
                        zeit: e.zeit,
                    })
                    .collect();
                self.baum = Some(Baum::neu(&blaetter));
                self.ordner = 0;
                self.auswahl.clear();
                self.anker = None;
                self.passwort = pw;
                self.ziel = regeln::standard_ziel(&pfad, t("Entpackt"))
                    .display()
                    .to_string();
                let name = pfad
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.ctx
                    .send_viewport_cmd(egui::ViewportCommand::Title(format!("{name} – Packwerk")));
                let n = inhalt.eintraege.len();
                let mut text = tf(
                    "{name} geöffnet: {n} Einträge.",
                    &[("name", &name), ("n", &n)],
                );
                let unsicher = inhalt.unsichere();
                if unsicher > 0 {
                    text.push(' ');
                    text.push_str(&tf(
                        "{n} Einträge mit unsicherem Pfad werden nicht entpackt.",
                        &[("n", &unsicher)],
                    ));
                }
                self.melden(text, unsicher > 0);
                self.inhalt = Some(inhalt);
                self.modus = Modus::Ansicht;
                if let Some(ziel) = self.danach_entpacken.take() {
                    self.ziel = ziel.display().to_string();
                    self.entpacken_starten(None, 0);
                }
            }
            Err(Fehler::PasswortNoetig) => {
                self.dialog = Some(Dialog::PasswortLesen {
                    pfad,
                    eingabe: String::new(),
                    falsch: false,
                    fokus: true,
                });
            }
            Err(Fehler::PasswortFalsch) => {
                self.dialog = Some(Dialog::PasswortLesen {
                    pfad,
                    eingabe: String::new(),
                    falsch: true,
                    fokus: true,
                });
            }
            Err(Fehler::Abgebrochen) => {
                self.danach_entpacken = None;
                self.melden(t("Abgebrochen."), false);
            }
            Err(f) => {
                self.danach_entpacken = None;
                let text = f.text();
                self.melden(text.clone(), true);
                let name = pfad.display().to_string();
                self.dialog = Some(Dialog::Meldung {
                    titel: t("Archiv lässt sich nicht öffnen").to_string(),
                    text: format!("{name}\n\n{text}"),
                });
            }
        }
    }

    // -----------------------------------------------------------------------
    // Entpacken
    // -----------------------------------------------------------------------

    /// Einträge der Auswahl (Knoten im aktuellen Ordner → Einträge darunter).
    fn auswahl_eintraege(&self) -> HashSet<usize> {
        let Some(baum) = &self.baum else {
            return HashSet::new();
        };
        self.auswahl
            .iter()
            .flat_map(|&k| baum.eintraege_unter(k))
            .collect()
    }

    fn entpacken_starten(&mut self, auswahl: Option<HashSet<usize>>, weglassen: usize) {
        let Some(inhalt) = &self.inhalt else { return };
        let ziel = self.ziel.trim();
        if ziel.is_empty() {
            self.melden(t("Bitte einen Zielordner wählen."), true);
            return;
        }
        let auftrag = EntpackAuftrag {
            inhalt: inhalt.clone(),
            passwort: self.passwort.clone(),
            ziel: PathBuf::from(ziel),
            auswahl,
            weglassen,
            konflikt: Konflikt::Fragen,
        };
        self.entpacken_pruefen(Box::new(auftrag));
    }

    /// Passwort und Archiv-Bombe prüfen, dann starten.
    fn entpacken_pruefen(&mut self, auftrag: Box<EntpackAuftrag>) {
        let gewaehlt = |i: usize| auftrag.auswahl.as_ref().is_none_or(|s| s.contains(&i));
        let braucht_pw = auftrag
            .inhalt
            .eintraege
            .iter()
            .enumerate()
            .any(|(i, e)| gewaehlt(i) && e.verschluesselt && e.art == EintragArt::Datei);
        if braucht_pw && auftrag.passwort.as_deref().is_none_or(str::is_empty) {
            self.dialog = Some(Dialog::PasswortEntpacken {
                auftrag,
                eingabe: String::new(),
                falsch: false,
                fokus: true,
            });
            return;
        }
        let (mut gesamt, mut anzahl) = (0u64, 0usize);
        for (i, e) in auftrag.inhalt.eintraege.iter().enumerate() {
            if gewaehlt(i) {
                gesamt = gesamt.saturating_add(e.groesse);
                anzahl += 1;
            }
        }
        let mut warnungen: Vec<String> =
            regeln::bomben_warnungen(gesamt, auftrag.inhalt.archivgroesse, anzahl)
                .into_iter()
                .map(|w| match w {
                    Warnung::SehrGross(b) => {
                        let g = anzeige::groesse(b);
                        tf("Entpackt wird das Archiv {g} gross.", &[("g", &g)])
                    }
                    Warnung::HoheKompression(f) => tf(
                        "Ungewöhnlich stark gepackt (Faktor {f}) – typisch für Archiv-Bomben.",
                        &[("f", &f)],
                    ),
                    Warnung::VieleEintraege(n) => tf("Sehr viele Einträge: {n}.", &[("n", &n)]),
                })
                .collect();
        if auftrag.inhalt.ueberlappend {
            warnungen
                .push(t("Einträge im ZIP überlappen sich – typisch für Zip-Bomben.").to_string());
        }
        if !warnungen.is_empty() {
            self.dialog = Some(Dialog::Bombe { auftrag, warnungen });
            return;
        }
        self.entpacken_ausfuehren(auftrag);
    }

    fn entpacken_ausfuehren(&mut self, auftrag: Box<EntpackAuftrag>) {
        if self.beschaeftigt() {
            return;
        }
        self.starten(t("Wird entpackt …"), move |fp| {
            let r = archiv::entpacken(&auftrag, fp);
            Ergebnis::Entpackt(auftrag, r)
        });
    }

    fn entpackt(&mut self, auftrag: Box<EntpackAuftrag>, r: Result<Bericht, Fehler>) {
        match r {
            Ok(b) => {
                let g = anzeige::groesse(b.bytes);
                let mut zeilen = vec![tf(
                    "{n} Dateien ({g}) entpackt.",
                    &[("n", &b.dateien), ("g", &g)],
                )];
                if b.ordner > 0 {
                    zeilen.push(tf("{n} Ordner angelegt.", &[("n", &b.ordner)]));
                }
                if !b.uebersprungen.is_empty() {
                    zeilen.push(tf("{n} übersprungen.", &[("n", &b.uebersprungen.len())]));
                }
                if !b.abgelehnt.is_empty() {
                    zeilen.push(tf(
                        "{n} abgelehnt oder fehlgeschlagen.",
                        &[("n", &b.abgelehnt.len())],
                    ));
                }
                let mut details: Vec<String> = b
                    .abgelehnt
                    .iter()
                    .map(|(n, g)| format!("✕ {n} – {g}"))
                    .collect();
                details.extend(b.uebersprungen.iter().map(|n| format!("↷ {n}")));
                self.melden(zeilen.join(" "), !b.abgelehnt.is_empty());
                self.dialog = Some(Dialog::Fertig {
                    titel: t("Entpacken fertig").to_string(),
                    zeilen,
                    details,
                    ordner: Some(b.ziel),
                });
            }
            Err(Fehler::Konflikte(anzahl, beispiele)) => {
                self.dialog = Some(Dialog::Konflikt {
                    auftrag,
                    anzahl,
                    beispiele,
                });
            }
            Err(Fehler::PasswortNoetig) => {
                self.dialog = Some(Dialog::PasswortEntpacken {
                    auftrag,
                    eingabe: String::new(),
                    falsch: false,
                    fokus: true,
                });
            }
            Err(Fehler::PasswortFalsch) => {
                self.dialog = Some(Dialog::PasswortEntpacken {
                    auftrag,
                    eingabe: String::new(),
                    falsch: true,
                    fokus: true,
                });
            }
            Err(Fehler::Abgebrochen) => {
                self.melden(
                    t("Abgebrochen. Bereits entpackte Dateien bleiben im Zielordner."),
                    false,
                );
            }
            Err(f) => {
                let text = f.text();
                self.melden(text.clone(), true);
                self.dialog = Some(Dialog::Meldung {
                    titel: t("Entpacken fehlgeschlagen").to_string(),
                    text,
                });
            }
        }
    }

    // -----------------------------------------------------------------------
    // Neues Archiv
    // -----------------------------------------------------------------------

    fn neues_archiv(&mut self) {
        self.modus = Modus::Packen;
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Title("Packwerk".to_string()));
    }

    fn quellen_dazu(&mut self, neu: Vec<PathBuf>) {
        for p in neu {
            if !self.quellen.contains(&p) {
                self.quellen.push(p);
            }
        }
        if self.pack_ordner.is_none() {
            self.pack_ordner = self
                .quellen
                .first()
                .and_then(|q| q.parent().map(Path::to_path_buf));
        }
        if self.pack_name_auto {
            self.pack_name = regeln::archivname_vorschlag(&self.quellen, t("Archiv"));
        }
        if self.pack_art == Art::Gz && !self.gz_moeglich() {
            self.pack_art = Art::TarGz;
        }
    }

    fn gz_moeglich(&self) -> bool {
        self.quellen.len() == 1 && self.quellen[0].is_file()
    }

    fn pack_ziel(&self) -> Option<PathBuf> {
        let name = self.pack_name.trim();
        if name.is_empty() {
            return None;
        }
        let ordner = self.pack_ordner.clone()?;
        let datei = if self.pack_art == Art::Gz {
            let quelle = self
                .quellen
                .first()
                .and_then(|q| q.file_name())
                .map(|n| n.to_string_lossy().into_owned());
            if self.pack_name_auto {
                format!("{}.gz", quelle.unwrap_or_else(|| name.to_string()))
            } else {
                format!("{name}.gz")
            }
        } else {
            format!("{name}.{}", self.pack_art.endung())
        };
        Some(ordner.join(datei))
    }

    fn speichern_unter(&mut self) {
        let mut d = rfd::FileDialog::new().set_title(t("Archiv speichern unter"));
        if let Some(o) = &self.pack_ordner {
            d = d.set_directory(o);
        }
        if let Some(z) = self
            .pack_ziel()
            .and_then(|z| z.file_name().map(|n| n.to_string_lossy().into_owned()))
        {
            d = d.set_file_name(z);
        }
        if let Some(p) = d.save_file() {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(a) = regeln::art_aus_name(&name) {
                if a != Art::Gz || self.gz_moeglich() {
                    self.pack_art = a;
                }
            }
            let stamm = regeln::name_ohne_endung(&name);
            if !stamm.is_empty() {
                self.pack_name = stamm;
                self.pack_name_auto = false;
            }
            self.pack_ordner = p.parent().map(Path::to_path_buf);
        }
    }

    fn packen_starten(&mut self) {
        if self.quellen.is_empty() {
            self.melden(t("Bitte zuerst Dateien oder Ordner hinzufügen."), true);
            return;
        }
        let Some(ziel) = self.pack_ziel() else {
            self.melden(t("Bitte Name und Ordner für das Archiv angeben."), true);
            return;
        };
        let passwort = if self.pack_art.kann_passwort() && !self.pack_pw.is_empty() {
            if self.pack_pw != self.pack_pw2 {
                self.melden(t("Die beiden Passwörter stimmen nicht überein."), true);
                return;
            }
            Some(self.pack_pw.clone())
        } else {
            None
        };
        if self.pack_art == Art::Gz && !self.gz_moeglich() {
            self.melden(Fehler::GzNurEineDatei.text(), true);
            return;
        }
        let auftrag = Box::new(PackAuftrag {
            quellen: self.quellen.clone(),
            ziel: ziel.clone(),
            art: self.pack_art,
            stufe: self.stufe,
            passwort,
            ueberschreiben: false,
        });
        if ziel.exists() {
            self.dialog = Some(Dialog::Ueberschreiben { auftrag });
            return;
        }
        self.packen_ausfuehren(auftrag);
    }

    fn packen_ausfuehren(&mut self, auftrag: Box<PackAuftrag>) {
        if self.beschaeftigt() {
            return;
        }
        self.starten(t("Archiv wird erstellt …"), move |fp| {
            let r = archiv::packen(&auftrag, fp);
            Ergebnis::Gepackt(auftrag, r)
        });
    }

    fn gepackt(&mut self, auftrag: Box<PackAuftrag>, r: Result<PackBericht, Fehler>) {
        match r {
            Ok(b) => {
                let name = b
                    .ziel
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let g = anzeige::groesse(b.bytes);
                let a = anzeige::groesse(b.archivgroesse);
                let mut zeilen = vec![
                    tf("{name} erstellt.", &[("name", &name)]),
                    tf(
                        "{n} Dateien ({g}) → {a}.",
                        &[("n", &b.dateien), ("g", &g), ("a", &a)],
                    ),
                ];
                if auftrag.passwort.is_some() {
                    zeilen.push(t("Mit Passwort verschlüsselt (AES-256).").to_string());
                }
                if !b.uebersprungen.is_empty() {
                    zeilen.push(tf("{n} übersprungen.", &[("n", &b.uebersprungen.len())]));
                }
                let details = b
                    .uebersprungen
                    .iter()
                    .map(|(p, g)| format!("↷ {p} – {g}"))
                    .collect();
                self.melden(zeilen.join(" "), false);
                self.pack_pw.clear();
                self.pack_pw2.clear();
                self.dialog = Some(Dialog::Fertig {
                    titel: t("Archiv erstellt").to_string(),
                    zeilen,
                    details,
                    ordner: b.ziel.parent().map(Path::to_path_buf),
                });
            }
            Err(Fehler::ZielExistiert(_)) if !auftrag.ueberschreiben => {
                self.dialog = Some(Dialog::Ueberschreiben { auftrag });
            }
            Err(Fehler::Abgebrochen) => {
                self.melden(t("Abgebrochen. Es wurde kein Archiv angelegt."), false)
            }
            Err(f) => {
                let text = f.text();
                self.melden(text.clone(), true);
                self.dialog = Some(Dialog::Meldung {
                    titel: t("Packen fehlgeschlagen").to_string(),
                    text,
                });
            }
        }
    }

    // -----------------------------------------------------------------------
    // Ziehen und Ablegen
    // -----------------------------------------------------------------------

    fn ablegen(&mut self, ctx: &egui::Context) {
        let pfade: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if pfade.is_empty() {
            return;
        }
        if self.beschaeftigt() || self.dialog.is_some() {
            self.melden(
                t("Bitte warten, bis der laufende Auftrag fertig ist."),
                true,
            );
            return;
        }
        if self.modus == Modus::Packen {
            self.quellen_dazu(pfade);
            return;
        }
        if pfade.len() == 1 && pfade[0].is_file() && archiv::art_ermitteln(&pfade[0]).is_ok() {
            let p = pfade.into_iter().next().unwrap_or_default();
            self.oeffnen(p);
            return;
        }
        self.neues_archiv();
        self.quellen_dazu(pfade);
    }

    // -----------------------------------------------------------------------
    // Oberfläche
    // -----------------------------------------------------------------------

    fn kopfzeile(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Packwerk")
                    .size(19.0)
                    .strong()
                    .color(akzent()),
            );
            ui.add_space(12.0);
            if ui
                .button(t("📂 Öffnen …"))
                .on_hover_text(t("Archiv öffnen (ZIP, 7z, TAR, TAR.GZ, GZ)"))
                .clicked()
            {
                self.oeffnen_dialog();
            }
            if ui
                .add(chip(t("➕ Neues Archiv"), self.modus == Modus::Packen))
                .on_hover_text(t("Dateien und Ordner zu einem Archiv packen"))
                .clicked()
            {
                self.neues_archiv();
            }
            if self.inhalt.is_some()
                && ui
                    .add(chip(t("🗂 Archiv anzeigen"), self.modus == Modus::Ansicht))
                    .clicked()
            {
                self.modus = Modus::Ansicht;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("?").on_hover_text(t("Hilfe")).clicked() {
                    self.dialog = Some(Dialog::Hilfe);
                }
            });
        });
        if self.modus == Modus::Ansicht && self.inhalt.is_some() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Ziel:")).color(thema::muted()));
                let breite = (ui.available_width() - 360.0).max(160.0);
                ui.add(egui::TextEdit::singleline(&mut self.ziel).desired_width(breite))
                    .on_hover_text(t("Ordner, in den entpackt wird (wird bei Bedarf angelegt)"));
                if ui.button(t("Wählen …")).clicked() {
                    let mut d = rfd::FileDialog::new().set_title(t("Zielordner wählen"));
                    let jetzt = PathBuf::from(self.ziel.trim());
                    if let Some(p) = jetzt.parent().filter(|p| p.is_dir()) {
                        d = d.set_directory(p);
                    }
                    if let Some(p) = d.pick_folder() {
                        self.ziel = p.display().to_string();
                    }
                }
                let n_auswahl = self.auswahl.len();
                if ui
                    .add_enabled(n_auswahl > 0, egui::Button::new(t("Auswahl entpacken")))
                    .on_hover_text(t(
                        "Nur die markierten Einträge entpacken (relativ zum aktuellen Ordner)",
                    ))
                    .clicked()
                {
                    let eintraege = self.auswahl_eintraege();
                    let weglassen = self
                        .baum
                        .as_ref()
                        .map(|b| b.pfad(self.ordner).len())
                        .unwrap_or(0);
                    self.entpacken_starten(Some(eintraege), weglassen);
                }
                let gross = egui::Button::new(
                    egui::RichText::new(t("Alles entpacken"))
                        .strong()
                        .color(auf_akzent()),
                )
                .fill(akzent());
                if ui.add(gross).clicked() {
                    self.entpacken_starten(None, 0);
                }
            });
            if let Some(inhalt) = &self.inhalt {
                if inhalt.verschluesselt {
                    ui.horizontal(|ui| {
                        let text = if self.passwort.is_some() {
                            t("🔒 Verschlüsselt – Passwort ist gesetzt.")
                        } else {
                            t("🔒 Verschlüsselt – beim Entpacken wird nach dem Passwort gefragt.")
                        };
                        ui.label(egui::RichText::new(text).color(thema::muted()));
                        if self.passwort.is_some()
                            && ui.small_button(t("Passwort vergessen")).clicked()
                        {
                            self.passwort = None;
                        }
                    });
                }
            }
        }
        ui.add_space(6.0);
    }

    fn fusszeile(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        if let Some(lauf) = &self.lauf {
            let anteil = lauf.fp.anteil();
            let aktuell = lauf.fp.aktuell();
            let titel = lauf.titel;
            let mut abbrechen = false;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(titel);
                if ui.button(t("Abbrechen")).clicked() {
                    abbrechen = true;
                }
                let bar = match anteil {
                    Some(a) => egui::ProgressBar::new(a).show_percentage(),
                    None => egui::ProgressBar::new(0.0).animate(true),
                };
                ui.add(bar.desired_width((ui.available_width() * 0.45).max(120.0)));
                ui.add(
                    egui::Label::new(egui::RichText::new(aktuell).color(thema::muted())).truncate(),
                );
            });
            if abbrechen {
                lauf.fp
                    .abbrechen
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        } else {
            ui.horizontal(|ui| {
                let farbe = if self.status_fehler {
                    rot()
                } else {
                    thema::muted()
                };
                ui.add(egui::Label::new(egui::RichText::new(&self.status).color(farbe)).truncate());
                if let (Modus::Ansicht, Some(inhalt)) = (self.modus, &self.inhalt) {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let gesamt: u64 = inhalt
                            .eintraege
                            .iter()
                            .map(|e| e.groesse)
                            .fold(0, u64::saturating_add);
                        let g = anzeige::groesse(gesamt);
                        let a = anzeige::groesse(inhalt.archivgroesse);
                        let n = inhalt.eintraege.len();
                        let art = inhalt.art.name();
                        ui.label(
                            egui::RichText::new(tf(
                                "{art} · {n} Einträge · {g} entpackt · {a} gepackt",
                                &[("art", &art), ("n", &n), ("g", &g), ("a", &a)],
                            ))
                            .color(thema::muted()),
                        );
                    });
                }
            });
        }
        ui.add_space(4.0);
    }

    fn startseite(&mut self, ui: &mut egui::Ui) {
        ui.add_space(ui.available_height() * 0.18);
        ui.vertical_centered(|ui| {
            egui::Frame::none()
                .fill(thema::panel())
                .rounding(16.0)
                .stroke(egui::Stroke::new(1.5_f32, thema::border()))
                .inner_margin(32.0)
                .show(ui, |ui| {
                    ui.set_width(460.0_f32.min(ui.available_width()));
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("📦").size(48.0).color(akzent()));
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(t("Archiv hierher ziehen")).size(20.0).strong());
                        ui.label(
                            egui::RichText::new(t(
                                "ZIP, 7z, TAR, TAR.GZ und GZ öffnen – oder Dateien und Ordner ablegen, um ein neues Archiv zu packen.",
                            ))
                            .color(thema::muted()),
                        );
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            let w = ui.available_width();
                            ui.add_space((w - 300.0).max(0.0) / 2.0);
                            let b = egui::Button::new(egui::RichText::new(t("Archiv öffnen …")).strong().color(auf_akzent()))
                                .fill(akzent())
                                .min_size(egui::vec2(140.0, 32.0));
                            if ui.add(b).clicked() {
                                self.oeffnen_dialog();
                            }
                            if ui.add(egui::Button::new(t("Neues Archiv …")).min_size(egui::vec2(140.0, 32.0))).clicked() {
                                self.neues_archiv();
                            }
                        });
                    });
                });
        });
    }

    fn ansicht(&mut self, ui: &mut egui::Ui) {
        let Some(baum) = &self.baum else { return };
        let archivname = self
            .inhalt
            .as_ref()
            .and_then(|i| i.pfad.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Brotkrumen
        let mut neuer_ordner: Option<usize> = None;
        ui.horizontal(|ui| {
            let oben = baum.knoten.get(self.ordner).and_then(|k| k.eltern);
            if ui
                .add_enabled(oben.is_some(), egui::Button::new("⬆"))
                .on_hover_text(t("Eine Ebene höher (Rücktaste)"))
                .clicked()
            {
                neuer_ordner = oben;
            }
            let mut kette = Vec::new();
            let mut k = Some(self.ordner);
            while let Some(i) = k {
                kette.push(i);
                k = baum.knoten.get(i).and_then(|n| n.eltern);
                if kette.len() > baum.knoten.len() {
                    break;
                }
            }
            kette.reverse();
            for (n, &i) in kette.iter().enumerate() {
                if n > 0 {
                    ui.label(egui::RichText::new("›").color(thema::muted()));
                }
                let name = if i == 0 {
                    format!("📦 {archivname}")
                } else {
                    baum.knoten[i].name.clone()
                };
                if ui.add(egui::Button::new(name).frame(false)).clicked() {
                    neuer_ordner = Some(i);
                }
            }
        });
        ui.add_space(4.0);

        let kinder = baum.kinder_sortiert(self.ordner, self.sortierung, self.absteigend);

        // Tastatur
        let (zurueck, alle) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Backspace),
                i.modifiers.command && i.key_pressed(egui::Key::A),
            )
        });
        if zurueck && !ui.ctx().wants_keyboard_input() {
            if let Some(e) = baum.knoten.get(self.ordner).and_then(|k| k.eltern) {
                neuer_ordner = Some(e);
            }
        }
        if alle && !ui.ctx().wants_keyboard_input() {
            self.auswahl = kinder.iter().copied().collect();
        }

        // Kopf der Tabelle
        let breite = ui.available_width();
        let (kopf, _) = ui.allocate_exact_size(egui::vec2(breite, ZEILE), egui::Sense::hover());
        let spalten = Spalten::neu(kopf);
        let mut neue_sortierung: Option<Sortierung> = None;
        for (rect, text, s, rechts) in [
            (spalten.name, t("Name"), Sortierung::Name, false),
            (spalten.groesse, t("Grösse"), Sortierung::Groesse, true),
            (spalten.gepackt, t("Gepackt"), Sortierung::Gepackt, true),
            (spalten.zeit, t("Geändert"), Sortierung::Zeit, false),
        ] {
            let id = ui.id().with(("kopf", text));
            let r = ui.interact(rect, id, egui::Sense::click());
            let pfeil = if self.sortierung == s {
                if self.absteigend {
                    " ▼"
                } else {
                    " ▲"
                }
            } else {
                ""
            };
            let farbe = if r.hovered() {
                thema::text()
            } else {
                thema::muted()
            };
            let (pos, align) = if rechts {
                (
                    rect.right_center() - egui::vec2(8.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                )
            } else {
                (
                    rect.left_center() + egui::vec2(8.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                )
            };
            ui.painter().with_clip_rect(rect).text(
                pos,
                align,
                format!("{text}{pfeil}"),
                egui::FontId::proportional(12.5),
                farbe,
            );
            if r.clicked() {
                neue_sortierung = Some(s);
            }
        }
        ui.painter().hline(
            kopf.x_range(),
            kopf.bottom(),
            egui::Stroke::new(1.0_f32, thema::border()),
        );

        // Zeilen
        let mut klick: Option<(usize, usize, egui::Modifiers)> = None; // (Position, Knoten, Tasten)
        let mut doppel: Option<usize> = None;
        let unsicher: HashSet<usize> = self
            .inhalt
            .as_ref()
            .map(|i| {
                i.eintraege
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.unsicher())
                    .map(|(n, _)| n)
                    .collect()
            })
            .unwrap_or_default();
        let verschl: bool = self.inhalt.as_ref().is_some_and(|i| i.verschluesselt);
        ui.spacing_mut().item_spacing.y = 0.0;
        if kinder.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(t("Dieser Ordner ist leer.")).color(thema::muted()))
            });
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ZEILE, kinder.len(), |ui, bereich| {
                for pos in bereich {
                    let Some(&k) = kinder.get(pos) else { continue };
                    let Some(kn) = baum.knoten.get(k) else {
                        continue;
                    };
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), ZEILE),
                        egui::Sense::click(),
                    );
                    let gewaehlt = self.auswahl.contains(&k);
                    let fuellung = if gewaehlt {
                        Some(akzent_weich())
                    } else if resp.hovered() {
                        Some(thema::hover())
                    } else {
                        None
                    };
                    if let Some(f) = fuellung {
                        ui.painter().rect_filled(rect, 4.0, f);
                    }
                    let sp = Spalten::neu(rect);
                    let ist_unsicher = kn.eintrag.is_some_and(|e| unsicher.contains(&e));
                    let symbol = if kn.ist_ordner {
                        "📁"
                    } else {
                        match kn
                            .eintrag
                            .and_then(|e| self.inhalt.as_ref().and_then(|i| i.eintraege.get(e)))
                            .map(|e| e.art)
                        {
                            Some(EintragArt::Verknuepfung) => "🔗",
                            Some(EintragArt::Sonstiges) => "❔",
                            _ => "📄",
                        }
                    };
                    let schloss = if verschl && !kn.ist_ordner {
                        " 🔒"
                    } else {
                        ""
                    };
                    let warn = if ist_unsicher { "⚠ " } else { "" };
                    let farbe_name = if ist_unsicher { rot() } else { thema::text() };
                    let p = ui.painter();
                    p.with_clip_rect(sp.name).text(
                        sp.name.left_center() + egui::vec2(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{symbol}  {warn}{}{schloss}", kn.name),
                        egui::FontId::proportional(13.5),
                        farbe_name,
                    );
                    let gr = if kn.ist_ordner && kn.dateien == 0 {
                        String::new()
                    } else {
                        anzeige::groesse(kn.groesse)
                    };
                    p.with_clip_rect(sp.groesse).text(
                        sp.groesse.right_center() - egui::vec2(8.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        gr,
                        egui::FontId::proportional(12.5),
                        thema::muted(),
                    );
                    let gp = kn
                        .gepackt
                        .map(anzeige::groesse)
                        .unwrap_or_else(|| "–".to_string());
                    p.with_clip_rect(sp.gepackt).text(
                        sp.gepackt.right_center() - egui::vec2(8.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        gp,
                        egui::FontId::proportional(12.5),
                        thema::muted(),
                    );
                    let zeit = kn.zeit.map(|z| z.text()).unwrap_or_default();
                    p.with_clip_rect(sp.zeit).text(
                        sp.zeit.left_center() + egui::vec2(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        zeit,
                        egui::FontId::proportional(12.5),
                        thema::muted(),
                    );
                    let resp = if ist_unsicher {
                        resp.on_hover_text(t(
                            "Unsicherer Pfad – dieser Eintrag wird nicht entpackt.",
                        ))
                    } else if kn.ist_ordner {
                        let n = kn.dateien;
                        resp.on_hover_text(tf(
                            "{n} Dateien – Doppelklick öffnet den Ordner",
                            &[("n", &n)],
                        ))
                    } else {
                        resp
                    };
                    if resp.double_clicked() && kn.ist_ordner {
                        doppel = Some(k);
                    } else if resp.clicked() {
                        klick = Some((pos, k, ui.input(|i| i.modifiers)));
                    }
                }
            });

        // Auswerten (nach dem Zeichnen, weil `baum` geliehen ist)
        if let Some(s) = neue_sortierung {
            if self.sortierung == s {
                self.absteigend = !self.absteigend;
            } else {
                self.sortierung = s;
                self.absteigend = matches!(
                    s,
                    Sortierung::Groesse | Sortierung::Gepackt | Sortierung::Zeit
                );
            }
        }
        if let Some((pos, k, m)) = klick {
            if m.shift {
                let von = self.anker.unwrap_or(pos).min(pos);
                let bis = self.anker.unwrap_or(pos).max(pos);
                if !m.command {
                    self.auswahl.clear();
                }
                for i in von..=bis {
                    if let Some(&kk) = kinder.get(i) {
                        self.auswahl.insert(kk);
                    }
                }
            } else if m.command {
                if !self.auswahl.remove(&k) {
                    self.auswahl.insert(k);
                }
                self.anker = Some(pos);
            } else {
                self.auswahl.clear();
                self.auswahl.insert(k);
                self.anker = Some(pos);
            }
        }
        if let Some(k) = doppel.or(neuer_ordner) {
            self.ordner = k;
            self.auswahl.clear();
            self.anker = None;
        }
    }

    fn packen_ansicht(&mut self, ui: &mut egui::Ui) {
        egui::SidePanel::right("pack_optionen")
            .resizable(false)
            .exact_width(320.0)
            .frame(
                egui::Frame::none()
                    .fill(thema::bg())
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show_inside(ui, |ui| self.pack_optionen(ui));

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t("INHALT DES NEUEN ARCHIVS"))
                    .size(11.0)
                    .strong()
                    .color(thema::muted()),
            );
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button(t("📄 Dateien hinzufügen …")).clicked() {
                if let Some(v) = rfd::FileDialog::new()
                    .set_title(t("Dateien hinzufügen"))
                    .pick_files()
                {
                    self.quellen_dazu(v);
                }
            }
            if ui.button(t("📁 Ordner hinzufügen …")).clicked() {
                if let Some(v) = rfd::FileDialog::new()
                    .set_title(t("Ordner hinzufügen"))
                    .pick_folders()
                {
                    self.quellen_dazu(v);
                }
            }
            if ui
                .add_enabled(
                    !self.quellen.is_empty(),
                    egui::Button::new(t("Liste leeren")),
                )
                .clicked()
            {
                self.quellen.clear();
                if self.pack_name_auto {
                    self.pack_name.clear();
                }
            }
        });
        ui.add_space(6.0);
        if self.quellen.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("📥").size(40.0).color(thema::muted()));
                ui.label(
                    egui::RichText::new(t("Dateien und Ordner hierher ziehen"))
                        .size(16.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(t("oder mit den Knöpfen oben hinzufügen."))
                        .color(thema::muted()),
                );
            });
            return;
        }
        let mut weg: Option<usize> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, q) in self.quellen.iter().enumerate() {
                    egui::Frame::none()
                        .fill(thema::panel())
                        .rounding(8.0)
                        .stroke(egui::Stroke::new(1.0_f32, thema::border()))
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let ordner = q.is_dir();
                                ui.label(if ordner { "📁" } else { "📄" });
                                let name = q
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                ui.label(egui::RichText::new(name).strong());
                                let eltern = q
                                    .parent()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button("✕")
                                            .on_hover_text(t("Aus der Liste entfernen"))
                                            .clicked()
                                        {
                                            weg = Some(i);
                                        }
                                        if !ordner {
                                            if let Ok(m) = std::fs::metadata(q) {
                                                ui.label(
                                                    egui::RichText::new(anzeige::groesse(m.len()))
                                                        .color(thema::muted()),
                                                );
                                            }
                                        }
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(eltern).color(thema::muted()),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );
                            });
                        });
                    ui.add_space(4.0);
                }
            });
        if let Some(i) = weg {
            if i < self.quellen.len() {
                self.quellen.remove(i);
            }
            if self.pack_name_auto {
                self.pack_name = regeln::archivname_vorschlag(&self.quellen, t("Archiv"));
            }
        }
    }

    fn pack_optionen(&mut self, ui: &mut egui::Ui) {
        abschnitt(ui, t("FORMAT"));
        ui.horizontal_wrapped(|ui| {
            for a in [Art::Zip, Art::SiebenZ, Art::Tar, Art::TarGz, Art::Gz] {
                let erlaubt = a != Art::Gz || self.gz_moeglich();
                let r = ui.add_enabled(erlaubt, chip(a.name(), self.pack_art == a));
                let r = if a == Art::Gz {
                    r.on_hover_text(t("GZ packt genau eine Datei"))
                } else {
                    r
                };
                if r.clicked() {
                    self.pack_art = a;
                }
            }
        });
        let hinweis = match self.pack_art {
            Art::Zip => t("Läuft überall, auch ohne Zusatzprogramm (Deflate)."),
            Art::SiebenZ => t("Packt am stärksten (LZMA2, solid)."),
            Art::Tar => t("Nur gebündelt, nicht gepackt."),
            Art::TarGz => t("Üblich unter Linux."),
            Art::Gz => t("Eine einzelne Datei packen."),
        };
        ui.label(
            egui::RichText::new(hinweis)
                .size(12.0)
                .color(thema::muted()),
        );

        ui.add_space(10.0);
        abschnitt(ui, t("STÄRKE"));
        ui.add_enabled_ui(self.pack_art != Art::Tar, |ui| {
            ui.horizontal(|ui| {
                for (s, text) in [
                    (Stufe::Schnell, t("Schnell")),
                    (Stufe::Normal, t("Normal")),
                    (Stufe::Stark, t("Stark")),
                ] {
                    if ui.add(chip(text, self.stufe == s)).clicked() {
                        self.stufe = s;
                    }
                }
            });
        });

        ui.add_space(10.0);
        abschnitt(ui, t("PASSWORT"));
        if self.pack_art.kann_passwort() {
            ui.add(
                egui::TextEdit::singleline(&mut self.pack_pw)
                    .password(!self.pw_zeigen)
                    .hint_text(t("ohne Passwort leer lassen"))
                    .desired_width(f32::INFINITY),
            );
            if !self.pack_pw.is_empty() {
                ui.add(
                    egui::TextEdit::singleline(&mut self.pack_pw2)
                        .password(!self.pw_zeigen)
                        .hint_text(t("Passwort wiederholen"))
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.pw_zeigen, t("Passwort anzeigen"));
                    if !self.pack_pw2.is_empty() {
                        if self.pack_pw == self.pack_pw2 {
                            ui.label(egui::RichText::new(t("✔ gleich")).color(gruen()));
                        } else {
                            ui.label(egui::RichText::new(t("✕ verschieden")).color(rot()));
                        }
                    }
                });
                let text = if self.pack_art == Art::SiebenZ {
                    t("AES-256; auch die Dateinamen werden verschlüsselt.")
                } else {
                    t("AES-256; die Dateinamen bleiben sichtbar. Ältere Windows-Explorer können AES-ZIPs nicht öffnen.")
                };
                ui.label(egui::RichText::new(text).size(12.0).color(thema::muted()));
            }
        } else {
            ui.label(
                egui::RichText::new(t("Für TAR und GZ nicht möglich – ZIP oder 7z wählen."))
                    .size(12.0)
                    .color(thema::muted()),
            );
        }

        ui.add_space(10.0);
        abschnitt(ui, t("SPEICHERN UNTER"));
        ui.horizontal(|ui| {
            let endung = if self.pack_art == Art::Gz {
                ".gz".to_string()
            } else {
                format!(".{}", self.pack_art.endung())
            };
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.pack_name)
                    .desired_width(200.0)
                    .hint_text(t("Name")),
            );
            if r.changed() {
                self.pack_name_auto = false;
            }
            ui.label(endung);
        });
        let ordner = self
            .pack_ordner
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| t("(kein Ordner gewählt)").to_string());
        ui.add(
            egui::Label::new(egui::RichText::new(ordner).size(12.0).color(thema::muted()))
                .truncate(),
        );
        ui.horizontal(|ui| {
            if ui.button(t("Ordner …")).clicked() {
                let mut d = rfd::FileDialog::new().set_title(t("Ordner für das Archiv"));
                if let Some(o) = &self.pack_ordner {
                    d = d.set_directory(o);
                }
                if let Some(p) = d.pick_folder() {
                    self.pack_ordner = Some(p);
                }
            }
            if ui.button(t("Speichern unter …")).clicked() {
                self.speichern_unter();
            }
        });

        ui.add_space(16.0);
        let bereit = !self.quellen.is_empty() && self.pack_ziel().is_some();
        let knopf = egui::Button::new(
            egui::RichText::new(t("Archiv erstellen"))
                .strong()
                .size(15.0)
                .color(auf_akzent()),
        )
        .fill(akzent())
        .min_size(egui::vec2(ui.available_width(), 36.0));
        if ui.add_enabled(bereit, knopf).clicked() {
            self.packen_starten();
        }
    }

    // -----------------------------------------------------------------------
    // Dialoge
    // -----------------------------------------------------------------------

    fn dialoge(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.dialog.take() else {
            return;
        };
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let mut wahl: Option<Wahl> = if esc { Some(Wahl::Abbrechen) } else { None };

        let titel = match &d {
            Dialog::PasswortLesen { .. } | Dialog::PasswortEntpacken { .. } => {
                t("Passwort").to_string()
            }
            Dialog::Konflikt { .. } => t("Dateien schon vorhanden").to_string(),
            Dialog::Bombe { .. } => t("Vorsicht beim Entpacken").to_string(),
            Dialog::Ueberschreiben { .. } => t("Archiv ersetzen?").to_string(),
            Dialog::Fertig { titel, .. } | Dialog::Meldung { titel, .. } => titel.clone(),
            Dialog::Hilfe => t("Hilfe").to_string(),
        };
        egui::Window::new(titel)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.set_max_width(520.0);
                match &mut d {
                    Dialog::PasswortLesen { eingabe, falsch, fokus, .. }
                    | Dialog::PasswortEntpacken { eingabe, falsch, fokus, .. } => {
                        if *falsch {
                            ui.label(egui::RichText::new(t("Das Passwort stimmt nicht. Bitte noch einmal.")).color(rot()));
                        } else {
                            ui.label(t("Das Archiv ist verschlüsselt. Bitte Passwort eingeben:"));
                        }
                        let r = ui.add(egui::TextEdit::singleline(eingabe).password(true).desired_width(f32::INFINITY));
                        if *fokus {
                            r.request_focus();
                            *fokus = false;
                        }
                        if enter && !eingabe.is_empty() {
                            wahl = Some(Wahl::Ok);
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!eingabe.is_empty(), egui::Button::new(t("OK"))).clicked() {
                                wahl = Some(Wahl::Ok);
                            }
                            if ui.button(t("Abbrechen")).clicked() {
                                wahl = Some(Wahl::Abbrechen);
                            }
                        });
                    }
                    Dialog::Konflikt { anzahl, beispiele, .. } => {
                        let n = *anzahl;
                        ui.label(tf("{n} Dateien gibt es im Zielordner schon:", &[("n", &n)]));
                        for b in beispiele.iter() {
                            let name = b.display().to_string();
                            ui.add(egui::Label::new(egui::RichText::new(name).size(12.0).color(thema::muted())).truncate());
                        }
                        if n > beispiele.len() {
                            let mehr = n - beispiele.len();
                            ui.label(egui::RichText::new(tf("… und {n} weitere", &[("n", &mehr)])).size(12.0).color(thema::muted()));
                        }
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(t("Beide behalten")).on_hover_text(t("Neue Dateien bekommen «(2)» im Namen")).clicked() {
                                wahl = Some(Wahl::Umbenennen);
                            }
                            if ui.button(t("Überspringen")).on_hover_text(t("Vorhandene Dateien bleiben unverändert")).clicked() {
                                wahl = Some(Wahl::Ueberspringen);
                            }
                            if ui.add(egui::Button::new(egui::RichText::new(t("Überschreiben")).color(rot()))).clicked() {
                                wahl = Some(Wahl::Ueberschreiben);
                            }
                            if ui.button(t("Abbrechen")).clicked() {
                                wahl = Some(Wahl::Abbrechen);
                            }
                        });
                    }
                    Dialog::Bombe { warnungen, .. } => {
                        for w in warnungen.iter() {
                            ui.label(egui::RichText::new(format!("⚠ {w}")).color(rot()));
                        }
                        ui.add_space(4.0);
                        ui.label(t("Das kann sehr viel Speicherplatz belegen. Trotzdem entpacken?"));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button(t("Trotzdem entpacken")).clicked() {
                                wahl = Some(Wahl::Ok);
                            }
                            if ui.button(t("Abbrechen")).clicked() {
                                wahl = Some(Wahl::Abbrechen);
                            }
                        });
                    }
                    Dialog::Ueberschreiben { auftrag } => {
                        let p = auftrag.ziel.display().to_string();
                        ui.label(tf("Die Datei gibt es schon:\n{p}\n\nSoll sie durch das neue Archiv ersetzt werden?", &[("p", &p)]));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.add(egui::Button::new(egui::RichText::new(t("Ersetzen")).color(rot()))).clicked() {
                                wahl = Some(Wahl::Ueberschreiben);
                            }
                            if ui.button(t("Abbrechen")).clicked() {
                                wahl = Some(Wahl::Abbrechen);
                            }
                        });
                    }
                    Dialog::Fertig { zeilen, details, ordner, .. } => {
                        for z in zeilen.iter() {
                            ui.label(z.as_str());
                        }
                        if !details.is_empty() {
                            ui.add_space(6.0);
                            egui::CollapsingHeader::new(t("Einzelheiten")).default_open(false).show(ui, |ui| {
                                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                                    for zeile in details.iter().take(2000) {
                                        ui.label(egui::RichText::new(zeile.as_str()).size(12.0).color(thema::muted()));
                                    }
                                });
                            });
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ordner.is_some() && ui.button(t("Ordner öffnen")).clicked() {
                                wahl = Some(Wahl::OrdnerZeigen);
                            }
                            if ui.button(t("Schliessen")).clicked() || enter {
                                wahl = Some(Wahl::Ok);
                            }
                        });
                    }
                    Dialog::Meldung { text, .. } => {
                        ui.label(text.as_str());
                        ui.add_space(8.0);
                        if ui.button(t("OK")).clicked() || enter {
                            wahl = Some(Wahl::Ok);
                        }
                    }
                    Dialog::Hilfe => {
                        hilfe_text(ui);
                        ui.add_space(8.0);
                        if ui.button(t("Schliessen")).clicked() || enter {
                            wahl = Some(Wahl::Ok);
                        }
                    }
                }
            });

        let Some(w) = wahl else {
            self.dialog = Some(d);
            return;
        };
        match d {
            Dialog::PasswortLesen { pfad, eingabe, .. } => {
                if w == Wahl::Ok {
                    self.oeffnen_mit(pfad, Some(eingabe));
                } else {
                    self.danach_entpacken = None;
                    self.melden(
                        t("Ohne Passwort lässt sich das Archiv nicht öffnen."),
                        false,
                    );
                }
            }
            Dialog::PasswortEntpacken {
                mut auftrag,
                eingabe,
                ..
            } => {
                if w == Wahl::Ok {
                    self.passwort = Some(eingabe.clone());
                    auftrag.passwort = Some(eingabe);
                    self.entpacken_pruefen(auftrag);
                }
            }
            Dialog::Konflikt { mut auftrag, .. } => {
                let k = match w {
                    Wahl::Ueberschreiben => Some(Konflikt::Ueberschreiben),
                    Wahl::Umbenennen => Some(Konflikt::Umbenennen),
                    Wahl::Ueberspringen => Some(Konflikt::Ueberspringen),
                    _ => None,
                };
                if let Some(k) = k {
                    auftrag.konflikt = k;
                    self.entpacken_ausfuehren(auftrag);
                }
            }
            Dialog::Bombe { auftrag, .. } => {
                if w == Wahl::Ok {
                    self.entpacken_ausfuehren(auftrag);
                }
            }
            Dialog::Ueberschreiben { mut auftrag } => {
                if w == Wahl::Ueberschreiben {
                    auftrag.ueberschreiben = true;
                    self.packen_ausfuehren(auftrag);
                }
            }
            Dialog::Fertig { ordner, .. } => {
                if w == Wahl::OrdnerZeigen {
                    if let Some(o) = ordner {
                        archiv::ordner_zeigen(&o);
                    }
                }
            }
            Dialog::Meldung { .. } | Dialog::Hilfe => {}
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.abfragen();
        self.ablegen(ctx);
        if self.lauf.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let gesperrt = self.lauf.is_some() || self.dialog.is_some();

        egui::TopBottomPanel::top("kopf")
            .frame(
                egui::Frame::none()
                    .fill(thema::panel())
                    .inner_margin(egui::Margin::symmetric(12.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.add_enabled_ui(!gesperrt, |ui| self.kopfzeile(ui));
            });
        egui::TopBottomPanel::bottom("fuss")
            .frame(
                egui::Frame::none()
                    .fill(thema::panel())
                    .inner_margin(egui::Margin::symmetric(12.0, 4.0)),
            )
            .show(ctx, |ui| self.fusszeile(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(thema::bg())
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0)),
            )
            .show(ctx, |ui| {
                ui.add_enabled_ui(!gesperrt, |ui| match self.modus {
                    Modus::Start => self.startseite(ui),
                    Modus::Ansicht => self.ansicht(ui),
                    Modus::Packen => self.packen_ansicht(ui),
                });
            });

        self.dialoge(ctx);

        // Hinweis beim Ziehen über das Fenster
        let schwebt = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if schwebt {
            let text = if self.modus == Modus::Packen {
                t("Loslassen, um hinzuzufügen")
            } else {
                t("Loslassen: Archiv öffnen oder neues Archiv packen")
            };
            let p = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("ablegen"),
            ));
            let r = ctx.screen_rect();
            p.rect_filled(
                r,
                0.0,
                thema::pick(
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 210),
                    egui::Color32::from_rgba_unmultiplied(23, 24, 28, 210),
                ),
            );
            p.rect_stroke(r.shrink(10.0), 14.0, egui::Stroke::new(2.0_f32, akzent()));
            p.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(22.0),
                akzent(),
            );
        }
    }
}

/// Spaltenaufteilung einer Tabellenzeile.
struct Spalten {
    name: egui::Rect,
    groesse: egui::Rect,
    gepackt: egui::Rect,
    zeit: egui::Rect,
}

impl Spalten {
    fn neu(r: egui::Rect) -> Spalten {
        let rest = (r.width() - BREITE_GROESSE - BREITE_GEPACKT - BREITE_ZEIT).max(80.0);
        let x0 = r.left();
        let x1 = x0 + rest;
        let x2 = x1 + BREITE_GROESSE;
        let x3 = x2 + BREITE_GEPACKT;
        let x4 = (x3 + BREITE_ZEIT).max(r.right());
        let y = r.y_range();
        Spalten {
            name: egui::Rect::from_x_y_ranges(x0..=x1, y),
            groesse: egui::Rect::from_x_y_ranges(x1..=x2, y),
            gepackt: egui::Rect::from_x_y_ranges(x2..=x3, y),
            zeit: egui::Rect::from_x_y_ranges(x3..=x4, y),
        }
    }
}

fn chip(text: &str, sel: bool) -> egui::Button<'static> {
    let t = egui::RichText::new(text.to_string()).color(if sel { akzent() } else { thema::text() });
    egui::Button::new(t)
        .fill(if sel { akzent_weich() } else { thema::panel() })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if sel { akzent() } else { thema::border() },
        ))
        .rounding(8.0)
}

fn abschnitt(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(11.0)
            .strong()
            .color(thema::muted()),
    );
    ui.add_space(2.0);
}

fn hilfe_text(ui: &mut egui::Ui) {
    let zeilen = [
        t("Archiv öffnen: «Öffnen …», Doppelklick im Dateimanager («Öffnen mit») oder ins Fenster ziehen."),
        t("Ordner im Archiv mit Doppelklick öffnen, mit ⬆ oder der Rücktaste zurück."),
        t("Markieren: Klick, Strg+Klick (mehrere), Umschalt+Klick (Bereich), Strg+A (alle)."),
        t("«Alles entpacken» oder «Auswahl entpacken» schreibt in den Zielordner oben."),
        t("Neues Archiv: Dateien und Ordner hineinziehen, Format wählen, «Archiv erstellen»."),
        t("Schutz: Pfade, die aus dem Zielordner zeigen, werden abgelehnt; sehr grosse oder stark gepackte Archive werden vorher gemeldet; vorhandene Dateien werden nie ohne Rückfrage überschrieben."),
    ];
    for z in zeilen {
        ui.label(format!("• {z}"));
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new(t("Kommandozeile")).strong());
    ui.label(
        egui::RichText::new(
            "packwerk <archiv>\npackwerk --entpacken <archiv> [ziel]\npackwerk <datei/ordner> …",
        )
        .monospace(),
    );
}
