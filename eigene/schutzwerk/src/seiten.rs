//! Oberfläche der neuen Bereiche: Virenscan, Ressourcen-Wächter, Autostart,
//! Verbindungen, Kamera & Mikrofon, Fernzugriff – dazu die Befunde für die Übersicht,
//! die Warnleiste der Hintergrund-Wache und die Rückfrage vor Eingriffen.
//! Alles Lesen; Eingriffe (Autostart) nur nach Bestätigung und rückgängig machbar.

use crate::app::{amber, card, erklaerung, grn, h1, human_size, note, orange, red, short_path, state_color, state_dot, App, Frage, Section};
use crate::autostart::{self, Aktion, Art};
use crate::prozparse::{Ort, Sig, SigStatus};
use crate::scan::{self, ScanArt};
use crate::scanparse;
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::sys::{self, sperre, State};
use crate::thema;
use crate::waechter::Proz;
use crate::zugriff::{Geraet, VerbProz};

/// Ein Befund für die Übersicht.
pub(crate) struct Befund {
    pub bereich: Section,
    pub titel: String,
    pub state: State,
    pub detail: String,
    pub was: &'static str,
    pub tun: &'static str,
}

fn befund(bereich: Section, titel: &str, state: State, detail: String, was: &'static str, tun: &'static str) -> Befund {
    Befund { bereich, titel: titel.to_string(), state, detail, was, tun }
}

/// Kleine graue Überschrift in einer Karte.
fn abschnitt(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).strong().color(thema::muted()));
    ui.add_space(6.0);
}

fn weak_trunc(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(thema::muted())).truncate());
}

fn ort_text(o: Ort) -> Option<&'static str> {
    match o {
        Ort::Normal | Ort::System => None,
        Ort::Temp => Some(t("TEMP-ORDNER")),
        Ort::AppData => Some(t("APPDATA")),
        Ort::Downloads => Some(t("DOWNLOADS")),
        Ort::Oeffentlich => Some(t("ÖFFENTLICHER ORDNER")),
        Ort::Papierkorb => Some(t("PAPIERKORB")),
        Ort::Geloescht => Some(t("DATEI GELÖSCHT")),
    }
}

fn ort_marke(ui: &mut egui::Ui, o: Ort) {
    if let Some(txt) = ort_text(o) {
        stil::marke(ui, txt, if o.stark() { orange() } else { amber() }, 0.0);
    }
}

fn sig_marke(ui: &mut egui::Ui, s: Option<&Sig>) {
    let Some(s) = s else { return };
    let r = match s.status {
        SigStatus::Gueltig => stil::marke(ui, t("SIGNIERT"), grn(), 0.0),
        SigStatus::Unsigniert => stil::marke(ui, t("UNSIGNIERT"), orange(), 0.0),
        SigStatus::Ungueltig => stil::marke(ui, t("SIGNATUR UNGÜLTIG"), red(), 0.0),
        SigStatus::Unbekannt => stil::marke(ui, t("SIGNATUR ?"), thema::muted(), 0.0),
    };
    if !s.von.is_empty() {
        r.on_hover_text(tf("Herausgeber: {von}", &[("von", &s.von)]));
    }
}

/// Minuten:Sekunden.
fn uhr(sek: u64) -> String {
    if sek >= 3600 {
        format!("{}:{:02}:{:02}", sek / 3600, sek / 60 % 60, sek % 60)
    } else {
        format!("{}:{:02}", sek / 60, sek % 60)
    }
}

/// «vor 5 Min.», «vor 3 Std.», «vor 2 Tagen».
fn vor(sek: u64) -> String {
    if sek < 90 {
        t("gerade eben").to_string()
    } else if sek < 3600 {
        tf("vor {n} Min.", &[("n", &(sek / 60))])
    } else if sek < 86_400 * 2 {
        tf("vor {n} Std.", &[("n", &(sek / 3600))])
    } else {
        tf("vor {n} Tagen", &[("n", &(sek / 86_400))])
    }
}

/// Dauer in Worten (kurz).
fn dauer(sek: u64) -> String {
    if sek < 60 {
        tf("{n} s", &[("n", &sek)])
    } else if sek < 3600 {
        tf("{n} Min.", &[("n", &(sek / 60))])
    } else {
        tf("{n} Std.", &[("n", &(sek / 3600))])
    }
}

fn kurz_name(p: &str) -> String {
    p.rsplit(['\\', '/']).next().unwrap_or(p).to_string()
}

/// Ergebnis eines beendeten Scans (für Anzeige und Übersicht).
struct ScanBild {
    titel: &'static str,
    fertig: bool,
    laeuft_s: u64,
    code: Option<i32>,
    fehler: Option<String>,
    abgebrochen: bool,
    clam: bool,
    update: bool,
    funde: Vec<scanparse::Fund>,
    gescannt: Option<u64>,
    zeilen: Vec<String>,
}

impl App {
    fn sig(&self, pfad: &str) -> Option<&Sig> {
        if pfad.is_empty() {
            return None;
        }
        self.sigs.get(&pfad.to_lowercase())
    }

    fn sig_pruefen(&mut self, pfade: Vec<String>) {
        let pfade: Vec<String> = pfade.into_iter().filter(|p| !p.is_empty() && !self.sigs.contains_key(&p.to_lowercase())).collect();
        if !pfade.is_empty() {
            self.sig_job.start(move || sys::signaturen(pfade));
        }
    }

    /// Einstufung eines Prozesses: Ok, Warn (ansehen) oder Bad (sehr auffällig).
    fn proz_stufe(&self, p: &Proz, leerlauf: Option<u64>) -> State {
        let sig = self.sig(&p.exe).map(|s| s.status);
        if p.miner || p.imitation.is_some() || sig == Some(SigStatus::Ungueltig) {
            return State::Bad;
        }
        let im_leerlauf = leerlauf.is_some_and(|l| l >= 120);
        if p.ort.stark() || (im_leerlauf && p.hohe_last(self.schwelle.min(50.0))) || (sig == Some(SigStatus::Unsigniert) && p.ort.ungewoehnlich()) {
            return State::Warn;
        }
        State::Ok
    }

    fn verb_stufe(&self, v: &VerbProz) -> State {
        let sig = self.sig(&v.exe).map(|s| s.status);
        if v.miner || v.imitation.is_some() || sig == Some(SigStatus::Ungueltig) {
            return State::Bad;
        }
        if v.mining_port() || v.ort.stark() || (sig == Some(SigStatus::Unsigniert) && v.ort.ungewoehnlich()) {
            return State::Warn;
        }
        State::Ok
    }

    fn eintrag_stufe(e: &autostart::Eintrag) -> State {
        let sig = e.sig.as_ref().map(|s| s.status);
        if sig == Some(SigStatus::Ungueltig) && !e.aus {
            return State::Bad;
        }
        let ort = crate::prozparse::ort_von(&e.exe, cfg!(windows));
        if e.neu || (!e.aus && (ort.stark() || (sig == Some(SigStatus::Unsigniert) && ort.ungewoehnlich()))) {
            return State::Warn;
        }
        State::Ok
    }

    fn scan_bild(&self) -> Option<ScanBild> {
        let g = sperre(&self.scan.lauf);
        let l = g.as_ref()?;
        let update = l.titel == "Signaturen aktualisieren";
        let (funde, gescannt) = if l.fertig && !update {
            if l.clam {
                let e = scanparse::parse_clamscan(&l.zeilen.join("\n"));
                (e.funde, e.gescannt)
            } else {
                (scanparse::parse_mpcmdrun(&l.zeilen.join("\n")), None)
            }
        } else {
            (Vec::new(), None)
        };
        let n = l.zeilen.len();
        Some(ScanBild {
            titel: l.titel,
            fertig: l.fertig,
            laeuft_s: if l.fertig { l.dauer_s } else { l.start.elapsed().as_secs() },
            code: l.code,
            fehler: l.fehler.clone(),
            abgebrochen: l.abgebrochen,
            clam: l.clam,
            update,
            funde,
            gescannt,
            zeilen: l.zeilen.get(n.saturating_sub(300)..).map(|s| s.to_vec()).unwrap_or_default(),
        })
    }

    /// Text für die Fussleiste, solange ein Scan läuft.
    pub(crate) fn scan_status_text(&self) -> Option<String> {
        let b = self.scan_bild()?;
        if b.fertig {
            return None;
        }
        Some(tf("{titel} läuft seit {zeit} …", &[("titel", &t(b.titel)), ("zeit", &uhr(b.laeuft_s))]))
    }

    pub(crate) fn scan_hauptknopf(&mut self, ui: &mut egui::Ui) {
        let laeuft = self.scan.laeuft();
        if cfg!(windows) {
            if stil::haupt(ui, !laeuft && scan::mpcmdrun().is_some(), Sym::Suche, t("Schnellscan")).clicked() {
                scan::defender_scan(&self.scan, ScanArt::Schnell, "");
            }
        } else {
            let clam = sperre(&self.clam.data).clone().flatten();
            if stil::haupt(ui, !laeuft && clam.is_some() && !self.scan_pfad.trim().is_empty(), Sym::Suche, t("Ordner scannen")).clicked() {
                if let Some(c) = clam {
                    scan::clam_scan(&self.scan, &c, &self.scan_pfad);
                }
            }
        }
    }

    // ---------------- Übersicht: Befunde ----------------

    pub(crate) fn befunde(&self) -> Vec<Befund> {
        let mut v = Vec::new();

        // Virenscan
        let letzte = self.scan_bild().filter(|b| b.fertig && !b.update && b.fehler.is_none());
        let scan_funde = letzte.as_ref().map(|b| b.funde.len()).unwrap_or(0);
        let def = sperre(&self.defender.data).clone();
        let thr = sperre(&self.bedrohungen.data).clone();
        let clam_da = sperre(&self.clam.data).as_ref().is_some_and(|c| c.is_some());
        if def.is_some() || thr.is_some() || letzte.is_some() || clam_da {
            let aktiv = thr.as_ref().map(|v| v.iter().filter(|b| b.aktiv).count()).unwrap_or(0);
            let alt = def.as_ref().and_then(|d| d.sig_alter_tage).filter(|n| *n > 7);
            if aktiv > 0 || scan_funde > 0 {
                v.push(befund(
                    Section::Virenscan,
                    t("Virenscan"),
                    State::Bad,
                    tf("{n} Bedrohung(en) gemeldet", &[("n", &(aktiv.max(scan_funde)))]),
                    t("Der Virenschutz hat Schadsoftware gefunden, die eventuell noch nicht vollständig entfernt ist."),
                    t("Unter Windows: Windows-Sicherheit → Viren- & Bedrohungsschutz öffnen und den Empfehlungen folgen (meist «Entfernen» oder «Quarantäne»), danach einen vollständigen Scan starten. Unter macOS/Linux die gemeldeten Dateien löschen oder ersetzen."),
                ));
            } else if let Some(n) = alt {
                v.push(befund(
                    Section::Virenscan,
                    t("Virenscan"),
                    State::Warn,
                    tf("Virensignaturen sind {n} Tage alt", &[("n", &n)]),
                    t("Der Virenschutz kennt die neuesten Schädlinge noch nicht."),
                    t("Im Bereich Virenscan auf «Signaturen aktualisieren» klicken oder Windows Update laufen lassen."),
                ));
            } else {
                v.push(befund(Section::Virenscan, t("Virenscan"), State::Ok, t("Keine aktiven Bedrohungen gemeldet.").into(), "", ""));
            }
        }

        // Ressourcen
        if let Some(m) = sperre(&self.messung.data).as_ref() {
            let mut bad = Vec::new();
            let mut warn = Vec::new();
            for p in &m.procs {
                match self.proz_stufe(p, m.leerlauf_s) {
                    State::Bad => bad.push(p.name.clone()),
                    State::Warn => warn.push(p.name.clone()),
                    _ => {}
                }
            }
            let alarme = sperre(&self.wache.alarme).len();
            if !bad.is_empty() {
                v.push(befund(
                    Section::Ressourcen,
                    t("Ressourcen-Wächter"),
                    State::Bad,
                    tf("Sehr auffällig: {liste}", &[("liste", &bad.join(", "))]),
                    t("Ein Programm trägt den Namen eines bekannten Krypto-Miners, ahmt einen Windows-Systemprozess nach oder hat eine ungültige Signatur."),
                    t("Im Ressourcen-Wächter den Dateipfad ansehen und die Datei mit dem Virenscan prüfen. Im Zweifel den Rechner vom Netz trennen und Hilfe holen."),
                ));
            } else if !warn.is_empty() || alarme > 0 {
                let liste = if warn.is_empty() { t("dauerhaft hohe Last").to_string() } else { warn.join(", ") };
                v.push(befund(
                    Section::Ressourcen,
                    t("Ressourcen-Wächter"),
                    State::Warn,
                    tf("Ansehen: {liste}", &[("liste", &liste)]),
                    t("Ein Programm braucht im Leerlauf viel Rechenleistung oder läuft aus einem ungewöhnlichen Ordner (Temp, Downloads …). Das kann harmlos sein (Updates, Indexierung), aber auch ein versteckter Miner."),
                    t("Im Ressourcen-Wächter das Programm ansehen, die Signatur prüfen und den Ordner öffnen. Unbekanntes mit dem Virenscan prüfen."),
                ));
            } else {
                v.push(befund(Section::Ressourcen, t("Ressourcen-Wächter"), State::Ok, t("Keine auffälligen Programme.").into(), "", ""));
            }
        }

        // Autostart
        if let Some(r) = sperre(&self.autostart.data).as_ref() {
            let neu = r.eintraege.iter().filter(|e| e.neu).count();
            let stufen: Vec<State> = r.eintraege.iter().filter(|e| !e.system || e.neu).map(Self::eintrag_stufe).collect();
            let bad = stufen.iter().filter(|s| **s == State::Bad).count();
            let warn = stufen.iter().filter(|s| **s == State::Warn).count();
            let state = if bad > 0 { State::Bad } else if warn > 0 { State::Warn } else { State::Ok };
            let detail = if state == State::Ok {
                tf("{n} Einträge, nichts Auffälliges", &[("n", &r.eintraege.iter().filter(|e| !e.system).count())])
            } else {
                tf("{neu} neu, {auff} auffällig", &[("neu", &neu), ("auff", &(bad + warn).saturating_sub(neu))])
            };
            v.push(befund(
                Section::Autostart,
                t("Autostart"),
                state,
                detail,
                t("Seit der letzten Prüfung ist etwas dazugekommen, oder ein Eintrag startet ein Programm aus einem ungewöhnlichen Ordner ohne gültige Signatur."),
                t("Im Bereich Autostart die markierten Einträge ansehen. Kennst du sie (z. B. ein neu installiertes Programm), «Alle als bekannt übernehmen». Sonst deaktivieren – das lässt sich rückgängig machen."),
            ));
        }

        // Verbindungen
        if let Some(list) = sperre(&self.verb.data).as_ref() {
            let st: Vec<(State, &str)> = list.iter().map(|x| (self.verb_stufe(x), x.name.as_str())).collect();
            let schlimm = st.iter().map(|(s, _)| crate::app::rang(*s)).max().unwrap_or(0);
            let namen: Vec<&str> = st.iter().filter(|(s, _)| *s != State::Ok).map(|(_, n)| *n).collect();
            let state = match schlimm {
                3 => State::Bad,
                2 => State::Warn,
                _ => State::Ok,
            };
            let detail = if namen.is_empty() {
                tf("{n} Programme verbunden, nichts Auffälliges", &[("n", &list.len())])
            } else {
                tf("Ansehen: {liste}", &[("liste", &namen.join(", "))])
            };
            v.push(befund(
                Section::Verbindungen,
                t("Verbindungen"),
                state,
                detail,
                t("Ein Programm mit auffälligen Merkmalen (ungewöhnlicher Ordner, ohne Signatur, typischer Mining-Port) ist mit dem Internet verbunden."),
                t("Im Bereich Verbindungen das Programm und die Gegenstelle ansehen. Unbekanntes mit dem Virenscan prüfen."),
            ));
        }

        // Kamera & Mikrofon
        if let Some(k) = sperre(&self.kamera.data).as_ref().filter(|k| k.verfuegbar) {
            let aktiv: Vec<String> = k.eintraege.iter().filter(|e| e.aktiv).map(|e| e.app.clone()).collect();
            if aktiv.is_empty() {
                v.push(befund(Section::Kamera, t("Kamera & Mikrofon"), State::Ok, t("Gerade greift kein Programm zu.").into(), "", ""));
            } else {
                v.push(befund(
                    Section::Kamera,
                    t("Kamera & Mikrofon"),
                    State::Warn,
                    tf("Gerade aktiv: {liste}", &[("liste", &aktiv.join(", "))]),
                    t("Ein Programm benutzt in diesem Moment Kamera oder Mikrofon."),
                    t("Telefonierst, filmst oder sprichst du gerade? Dann ist alles in Ordnung. Sonst das Programm im Bereich Kamera & Mikrofon ansehen und beenden."),
                ));
            }
        }

        // Fernzugriff
        if let Some(f) = sperre(&self.fern.data).as_ref() {
            let mut teile: Vec<String> = Vec::new();
            if f.rdp == Some(true) {
                teile.push(t("Remotedesktop an").into());
            }
            if f.ra == Some(true) {
                teile.push(t("Remoteunterstützung erlaubt").into());
            }
            if f.ssh == Some(true) {
                teile.push(t("SSH-Server läuft").into());
            }
            for x in f.tools.iter().filter(|x| x.laeuft || x.dienst_auto) {
                teile.push(tf("{p} aktiv", &[("p", &x.produkt)]));
            }
            if teile.is_empty() {
                v.push(befund(Section::Fernzugriff, t("Fernzugriff"), State::Ok, t("Kein Fernzugriff aktiv.").into(), "", ""));
            } else {
                v.push(befund(
                    Section::Fernzugriff,
                    t("Fernzugriff"),
                    State::Warn,
                    teile.join(", "),
                    t("Über Fernzugriff kann jemand diesen Rechner bedienen. Das ist in Ordnung, wenn du es selbst eingerichtet hast und brauchst."),
                    t("Nicht benötigten Fernzugriff ausschalten bzw. das Programm deinstallieren. Schutzwerk ändert dabei nichts von selbst."),
                ));
            }
        }
        v
    }

    // ---------------- Warnleiste der Hintergrund-Wache ----------------

    pub(crate) fn alarm_leiste(&mut self, ctx: &egui::Context) {
        let alarme: Vec<_> = sperre(&self.wache.alarme).iter().filter(|a| !self.alarm_weg.contains(&a.pid)).cloned().collect();
        let Some(a) = alarme.first() else { return };
        let mut ansehen = false;
        let mut weg = false;
        egui::TopBottomPanel::top("schutz-alarm")
            .frame(egui::Frame::none().fill(stil::mix(orange(), thema::panel(), 0.16)).inner_margin(egui::Margin::symmetric(14.0, 8.0)).stroke(egui::Stroke::new(1.0, orange())))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    stil::symbol(ui, Sym::Warnung, 18.0, orange());
                    ui.label(
                        egui::RichText::new(tf(
                            "Dauerlast: {name} nutzt seit {zeit} rund {last} % der Rechenleistung.",
                            &[("name", &a.name), ("zeit", &dauer(a.sekunden)), ("last", &(a.last.round() as u32))],
                        ))
                        .strong(),
                    );
                    if alarme.len() > 1 {
                        stil::grau(ui, &tf("(+{n} weitere)", &[("n", &(alarme.len() - 1))]));
                    }
                    stil::rechts(ui, |ui| {
                        if stil::leise(ui, true, Sym::Kreuz, t("Ausblenden"), "").clicked() {
                            weg = true;
                        }
                        if stil::knopf(ui, Sym::Diagramm, t("Ansehen")).clicked() {
                            ansehen = true;
                        }
                    });
                });
            });
        if weg {
            for a in &alarme {
                self.alarm_weg.insert(a.pid);
            }
        }
        if ansehen {
            self.section = Section::Ressourcen;
            self.messung.start(|| crate::waechter::messen(3));
        }
    }

    // ---------------- Rückfrage vor Eingriffen ----------------

    pub(crate) fn frage_fenster(&mut self, ctx: &egui::Context) {
        let Some(f) = &self.frage else { return };
        let (titel, text, knopf) = match f {
            Frage::Deaktivieren(e) => match e.aktion {
                Aktion::Aufgabe { .. } => (
                    t("Aufgabe deaktivieren?"),
                    tf("Die geplante Aufgabe «{name}» wird deaktiviert (nicht gelöscht). Du kannst sie unter «Gesicherte Einträge» wieder einschalten.", &[("name", &e.name)]),
                    t("Deaktivieren"),
                ),
                _ => (
                    t("Autostart-Eintrag entfernen?"),
                    tf("«{name}» wird beim Start nicht mehr ausgeführt. Der Eintrag wird vorher gesichert und lässt sich unter «Gesicherte Einträge» wiederherstellen. Die Programmdatei bleibt unverändert.", &[("name", &e.name)]),
                    t("Deaktivieren"),
                ),
            },
            Frage::Wiederherstellen(s) => (t("Wiederherstellen?"), tf("«{name}» wird wieder beim Start ausgeführt.", &[("name", &kurz_name(&s.name))]), t("Wiederherstellen")),
        };
        let mut ja = false;
        let mut nein = false;
        egui::Window::new(titel)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(text.as_str());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    stil::rechts(ui, |ui| {
                        if stil::haupt(ui, true, Sym::Haken, knopf).clicked() {
                            ja = true;
                        }
                        if stil::knopf_text(ui, true, t("Abbrechen")).clicked() {
                            nein = true;
                        }
                    });
                });
            });
        if nein {
            self.frage = None;
        }
        if ja {
            match self.frage.take() {
                Some(Frage::Deaktivieren(e)) => self.aktion.start(move || autostart::deaktivieren(&e).map(|_| t("Deaktiviert und gesichert."))),
                Some(Frage::Wiederherstellen(s)) => self.aktion.start(move || autostart::wiederherstellen(&s).map(|_| t("Wiederhergestellt."))),
                None => {}
            }
        }
    }

    // ---------------- Virenscan ----------------

    pub(crate) fn page_scan(&mut self, ui: &mut egui::Ui) {
        if cfg!(windows) {
            h1(ui, t("Virenscan"), t("Prüft Dateien mit Microsoft Defender, dem Virenschutz von Windows. Schutzwerk hat keine eigenen Virensignaturen."));
            self.defender_karte(ui);
            self.scan_karte_windows(ui);
            self.bedrohungen_karte(ui);
        } else {
            h1(ui, t("Virenscan"), t("Prüft Ordner mit ClamAV, einem freien Virenscanner. Schutzwerk hat keine eigenen Virensignaturen."));
            self.clam_karte(ui);
        }
    }

    fn defender_karte(&mut self, ui: &mut egui::Ui) {
        let d = sperre(&self.defender.data).clone();
        let laeuft = self.defender.running();
        let mut neu = false;
        card(ui, |ui| {
            abschnitt(ui, t("MICROSOFT DEFENDER"));
            match d {
                None => {
                    ui.horizontal(|ui| {
                        if laeuft {
                            ui.spinner();
                        }
                        ui.weak(if laeuft { t("wird gelesen …") } else { t("Noch nicht gelesen.") });
                    });
                }
                Some(d) if !d.verfuegbar => {
                    ui.label(t("Microsoft Defender antwortet nicht. Läuft ein anderes Virenschutzprogramm, ist das normal."));
                    if !d.produkte.is_empty() {
                        ui.weak(tf("Gemeldeter Virenschutz: {p}", &[("p", &d.produkte)]));
                    }
                }
                Some(d) => {
                    egui::Grid::new("def").num_columns(3).spacing([14.0, 8.0]).show(ui, |ui| {
                        let ja_nein = |b: Option<bool>| match b {
                            Some(true) => (State::Ok, t("Ein")),
                            Some(false) => (State::Bad, t("Aus")),
                            None => (State::Unknown, t("Unbekannt")),
                        };
                        let (s, txt) = ja_nein(d.echtzeit);
                        state_dot(ui, s);
                        ui.label(t("Echtzeitschutz"));
                        ui.label(egui::RichText::new(txt).color(state_color(s)).strong());
                        ui.end_row();

                        let (s, txt) = match d.sig_alter_tage {
                            Some(n) if n <= 3 => (State::Ok, tf("{n} Tage alt", &[("n", &n)])),
                            Some(n) if n <= 7 => (State::Warn, tf("{n} Tage alt", &[("n", &n)])),
                            Some(n) => (State::Bad, tf("{n} Tage alt", &[("n", &n)])),
                            None => (State::Unknown, t("Unbekannt").to_string()),
                        };
                        state_dot(ui, s);
                        ui.label(t("Virensignaturen"));
                        ui.label(if d.sig_stand.is_empty() { txt } else { tf("{alter} (Stand {stand})", &[("alter", &txt), ("stand", &d.sig_stand)]) });
                        ui.end_row();

                        let scan_txt = |n: Option<u32>| match n {
                            Some(0) => t("heute").to_string(),
                            Some(n) => tf("vor {n} Tagen", &[("n", &n)]),
                            None => t("noch nie").to_string(),
                        };
                        state_dot(ui, if d.schnell_alter_tage.is_some_and(|n| n <= 14) { State::Ok } else { State::Unknown });
                        ui.label(t("Letzter Schnellscan"));
                        ui.label(scan_txt(d.schnell_alter_tage));
                        ui.end_row();
                        state_dot(ui, State::Unknown);
                        ui.label(t("Letzter vollständiger Scan"));
                        ui.label(scan_txt(d.voll_alter_tage));
                        ui.end_row();
                        if !d.modus.is_empty() {
                            state_dot(ui, if d.modus.eq_ignore_ascii_case("Normal") { State::Ok } else { State::Unknown });
                            ui.label(t("Betriebsart"));
                            ui.label(if d.modus.eq_ignore_ascii_case("Normal") { t("Normal (aktiver Virenschutz)").to_string() } else { tf("{m} – ein anderes Virenschutzprogramm ist zuständig", &[("m", &d.modus)]) });
                            ui.end_row();
                        }
                        if let Some(m) = d.manipulationsschutz {
                            let (s, txt) = ja_nein(Some(m));
                            state_dot(ui, if m { State::Ok } else { State::Warn });
                            ui.label(t("Manipulationsschutz"));
                            ui.label(egui::RichText::new(txt).color(state_color(if m { s } else { State::Warn })));
                            ui.end_row();
                        }
                    });
                    if !d.produkte.is_empty() {
                        ui.add_space(4.0);
                        ui.weak(tf("Im Sicherheitscenter gemeldet: {p}", &[("p", &d.produkte)]));
                    }
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if stil::knopf_an(ui, !laeuft, Sym::Aktualisieren, t("Status neu lesen")).clicked() {
                    neu = true;
                }
                if stil::knopf(ui, Sym::Extern, t("Windows-Sicherheit öffnen")).clicked() {
                    sys::oeffnen("windowsdefender:");
                }
            });
        });
        if neu {
            self.defender.start(scan::defender_status);
        }
    }

    fn scan_karte_windows(&mut self, ui: &mut egui::Ui) {
        let laeuft = self.scan.laeuft();
        let da = scan::mpcmdrun().is_some();
        let mut start: Option<ScanArt> = None;
        let mut update = false;
        card(ui, |ui| {
            abschnitt(ui, t("SCANNEN"));
            if !da {
                ui.label(t("MpCmdRun.exe wurde nicht gefunden – Microsoft Defender ist auf diesem Rechner nicht verfügbar."));
                return;
            }
            ui.horizontal_wrapped(|ui| {
                if stil::knopf_an(ui, !laeuft, Sym::Suche, t("Schnellscan")).clicked() {
                    start = Some(ScanArt::Schnell);
                }
                if stil::knopf_an(ui, !laeuft, Sym::Schild, t("Vollständiger Scan")).clicked() {
                    start = Some(ScanArt::Voll);
                }
                if stil::knopf_an(ui, !laeuft, Sym::Download, t("Signaturen aktualisieren")).clicked() {
                    update = true;
                }
            });
            ui.add_space(8.0);
            stil::feld(ui, t("Ordner oder Datei scannen"), |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.scan_pfad).hint_text(t("z. B. C:\\Users\\Name\\Downloads")).desired_width((ui.available_width() - 110.0).max(120.0)));
                    if stil::knopf_an(ui, !laeuft && !self.scan_pfad.trim().is_empty(), Sym::Ordner, t("Scannen")).clicked() {
                        start = Some(ScanArt::Pfad);
                    }
                });
            });
            stil::hinweis(ui, t("Ein vollständiger Scan kann über eine Stunde dauern. Schutzwerk bleibt dabei bedienbar."));
            self.lauf_anzeigen(ui);
        });
        if let Some(a) = start {
            scan::defender_scan(&self.scan, a, &self.scan_pfad);
        }
        if update {
            scan::defender_update(&self.scan);
        }
    }

    /// Zustand und Ergebnis des aktuellen/letzten Scans.
    fn lauf_anzeigen(&mut self, ui: &mut egui::Ui) {
        let Some(b) = self.scan_bild() else { return };
        let mut abbrechen = false;
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t(b.titel)).strong());
            if !b.fertig {
                ui.spinner();
                ui.weak(tf("läuft seit {zeit}", &[("zeit", &uhr(b.laeuft_s))]));
                stil::rechts(ui, |ui| {
                    if stil::gefahr(ui, true, Sym::Stopp, t("Abbrechen")).clicked() {
                        abbrechen = true;
                    }
                });
            } else {
                ui.weak(tf("Dauer {zeit}", &[("zeit", &uhr(b.laeuft_s))]));
            }
        });
        if b.fertig {
            if let Some(e) = &b.fehler {
                ui.colored_label(red(), tf("Konnte nicht gestartet werden: {e}", &[("e", e)]));
            } else if b.abgebrochen {
                ui.colored_label(amber(), t("Abgebrochen."));
            } else if b.update {
                if b.code == Some(0) {
                    ui.colored_label(grn(), t("Signaturen sind aktuell."));
                } else {
                    ui.colored_label(amber(), tf("Aktualisierung fehlgeschlagen (Code {c}). Eventuell sind Administratorrechte oder eine Internetverbindung nötig.", &[("c", &b.code.unwrap_or(-1))]));
                }
            } else if !b.funde.is_empty() {
                ui.colored_label(red(), egui::RichText::new(tf("{n} Bedrohung(en) gefunden", &[("n", &b.funde.len())])).strong());
                for f in &b.funde {
                    ui.horizontal(|ui| {
                        stil::symbol(ui, Sym::Warnung, 15.0, red());
                        ui.label(egui::RichText::new(f.name.as_str()).strong());
                    });
                    for d in &f.dateien {
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            weak_trunc(ui, d);
                        });
                    }
                }
                ui.add_space(4.0);
                note(
                    ui,
                    if b.clam {
                        t("ClamAV meldet nur – es verändert nichts. Gemeldete Dateien löschen oder durch eine saubere Kopie ersetzen.")
                    } else {
                        t("Defender behandelt Funde selbst (z. B. Quarantäne). Details in der Windows-Sicherheit unter «Schutzverlauf».")
                    },
                );
            } else {
                let ok = if b.clam { b.code == Some(0) } else { b.code == Some(0) };
                if ok {
                    ui.colored_label(grn(), egui::RichText::new(t("Keine Bedrohungen gefunden.")).strong());
                    if let Some(n) = b.gescannt {
                        ui.weak(tf("{n} Dateien geprüft.", &[("n", &n)]));
                    }
                } else if !b.clam && b.code == Some(2) {
                    ui.colored_label(red(), t("Defender meldet Funde – Details in der Windows-Sicherheit unter «Schutzverlauf»."));
                } else {
                    ui.colored_label(amber(), tf("Scan beendet mit Code {c}. Siehe Ausgabe.", &[("c", &b.code.unwrap_or(-1))]));
                }
            }
        }
        if !b.zeilen.is_empty() {
            egui::CollapsingHeader::new(t("Ausgabe anzeigen")).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(200.0).stick_to_bottom(true).show(ui, |ui| {
                    for z in &b.zeilen {
                        ui.label(egui::RichText::new(z.as_str()).monospace().size(11.5));
                    }
                });
            });
        }
        if abbrechen {
            self.scan.abbrechen();
        }
    }

    fn bedrohungen_karte(&mut self, ui: &mut egui::Ui) {
        let list = sperre(&self.bedrohungen.data).clone();
        let laeuft = self.bedrohungen.running();
        let mut neu = false;
        card(ui, |ui| {
            abschnitt(ui, t("ERKANNTE BEDROHUNGEN (VERLAUF)"));
            match &list {
                None => {
                    ui.horizontal(|ui| {
                        if laeuft {
                            ui.spinner();
                        }
                        ui.weak(if laeuft { t("wird gelesen …") } else { t("Noch nicht gelesen.") });
                    });
                }
                Some(v) if v.is_empty() => {
                    ui.colored_label(grn(), t("Defender hat keine Bedrohungen gespeichert."));
                }
                Some(v) => {
                    for b in v.iter().take(100) {
                        ui.horizontal(|ui| {
                            state_dot(ui, if b.aktiv { State::Bad } else { State::Ok });
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(b.name.as_str()).strong());
                                    if b.aktiv {
                                        stil::marke(ui, t("AKTIV"), red(), 0.0);
                                    }
                                    if b.schwere >= 4 {
                                        stil::marke(ui, t("SCHWER"), orange(), 0.0);
                                    }
                                });
                                let status = b.status.parse::<u32>().map(|n| t(scanparse::status_wort(n))).unwrap_or(b.status.as_str());
                                ui.weak(format!("{} · {}", b.zeit, status));
                                for d in b.dateien.iter().take(5) {
                                    weak_trunc(ui, d);
                                }
                            });
                        });
                        ui.add_space(4.0);
                    }
                }
            }
            ui.add_space(6.0);
            if stil::knopf_an(ui, !laeuft, Sym::Aktualisieren, t("Neu laden")).clicked() {
                neu = true;
            }
        });
        if neu {
            self.bedrohungen.start(scan::defender_bedrohungen);
        }
    }

    fn clam_karte(&mut self, ui: &mut egui::Ui) {
        let clam = sperre(&self.clam.data).clone();
        let laeuft = self.clam.running();
        let scan_laeuft = self.scan.laeuft();
        let mut start = false;
        let mut update = false;
        let mut suchen = false;
        match clam {
            None => {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        if laeuft {
                            ui.spinner();
                        }
                        ui.weak(if laeuft { t("ClamAV wird gesucht …") } else { t("Noch nicht geprüft.") });
                    });
                });
            }
            Some(None) => {
                card(ui, |ui| {
                    abschnitt(ui, t("CLAMAV IST NICHT INSTALLIERT"));
                    ui.label(t("Für den Virenscan nutzt Schutzwerk ClamAV, einen freien Virenscanner. So installierst du ihn:"));
                    ui.add_space(6.0);
                    if cfg!(target_os = "macos") {
                        ui.label(t("Mit Homebrew im Terminal:"));
                        ui.label(egui::RichText::new("brew install clamav").monospace());
                    } else {
                        ui.label(t("Debian, Ubuntu, Mint im Terminal:"));
                        ui.label(egui::RichText::new("sudo apt install clamav").monospace());
                        ui.label(t("Fedora:"));
                        ui.label(egui::RichText::new("sudo dnf install clamav clamav-update").monospace());
                    }
                    ui.add_space(4.0);
                    stil::hinweis(ui, t("Danach einmal «freshclam» ausführen, damit die Virensignaturen geladen werden."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if stil::knopf(ui, Sym::Extern, t("ClamAV-Downloads öffnen")).clicked() {
                            sys::oeffnen("https://www.clamav.net/downloads");
                        }
                        if stil::knopf(ui, Sym::Aktualisieren, t("Erneut suchen")).clicked() {
                            suchen = true;
                        }
                    });
                });
            }
            Some(Some(c)) => {
                card(ui, |ui| {
                    abschnitt(ui, t("CLAMAV"));
                    ui.weak(c.version.as_str());
                    ui.add_space(8.0);
                    stil::feld(ui, t("Ordner scannen"), |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.scan_pfad).hint_text(t("z. B. /home/name/Downloads")).desired_width((ui.available_width() - 110.0).max(120.0)));
                            if stil::knopf_an(ui, !scan_laeuft && !self.scan_pfad.trim().is_empty(), Sym::Suche, t("Scannen")).clicked() {
                                start = true;
                            }
                        });
                    });
                    stil::hinweis(ui, t("Der ganze Benutzerordner kann je nach Grösse lange dauern. Schutzwerk bleibt dabei bedienbar."));
                    ui.add_space(6.0);
                    if c.freshclam.is_some() && stil::knopf_an(ui, !scan_laeuft, Sym::Download, t("Signaturen aktualisieren (freshclam)")).clicked() {
                        update = true;
                    }
                    self.lauf_anzeigen(ui);
                });
                if start {
                    scan::clam_scan(&self.scan, &c, &self.scan_pfad);
                }
                if update {
                    scan::clam_update(&self.scan, &c);
                }
            }
        }
        if suchen {
            self.clam.start(scan::clam_finden);
        }
    }

    // ---------------- Ressourcen-Wächter ----------------

    pub(crate) fn page_waechter(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Ressourcen-Wächter"), t("Findet Programme, die heimlich Rechenleistung nutzen (z. B. versteckte Krypto-Miner) oder aus ungewöhnlichen Ordnern laufen."));
        let m = sperre(&self.messung.data).clone();
        let mut pruefe: Vec<String> = Vec::new();
        let sig_ok = sys::signatur_moeglich();
        let sig_laeuft = self.sig_job.running();

        card(ui, |ui| {
            abschnitt(ui, t("HINTERGRUND-WACHE"));
            stil::schalter(ui, &mut self.wache_an, t("Dauerlast melden, solange Schutzwerk offen ist"));
            ui.add_space(4.0);
            stil::feld(ui, t("Schwelle (Anteil der gesamten Rechenleistung)"), |ui| {
                stil::schieber(ui, &mut self.schwelle, 30.0..=95.0, "%", 0);
            });
            stil::feld(ui, t("Dauer"), |ui| {
                stil::schieber(ui, &mut self.dauer_min, 1.0..=15.0, t("Min."), 0);
            });
            stil::hinweis(ui, t("Prüft alle 10 Sekunden. Liegt ein Programm länger darüber, erscheint oben eine Warnung."));
        });

        let Some(m) = m else {
            if self.messung.running() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak(t("wird gemessen …"));
                });
            } else {
                stil::leer(ui, Sym::Diagramm, t("Ressourcen-Wächter"), t("Noch nicht gemessen – auf «Messen» klicken."));
            }
            return;
        };

        let stufen: Vec<State> = m.procs.iter().map(|p| self.proz_stufe(p, m.leerlauf_s)).collect();
        let auff: Vec<usize> = (0..m.procs.len()).filter(|i| stufen.get(*i).is_some_and(|s| *s != State::Ok) || m.procs.get(*i).is_some_and(|p| p.fern.is_some() || p.hohe_last(self.schwelle))).collect();

        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(tf("{n} Prozesse", &[("n", &m.procs.len())])).strong());
                stil::grau(ui, &tf("gemessen über {s} s · {k} Kerne", &[("s", &m.sekunden), ("k", &m.kerne)]));
                match m.leerlauf_s {
                    Some(s) => stil::grau(ui, &tf("· keine Eingabe seit {d}", &[("d", &dauer(s))])),
                    None => stil::grau(ui, t("· Leerlauf nicht ermittelbar")),
                };
            });
            if cfg!(windows) && !m.gpu_verfuegbar {
                stil::hinweis(ui, t("GPU-Last ist auf diesem Rechner nicht verfügbar."));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let mut nur = self.nur_auffaellig;
                ui.allocate_ui(egui::vec2(280.0, 28.0), |ui| {
                    stil::wahl(ui, &mut nur, &[(true, t("Nur auffällige")), (false, t("Alle"))]);
                });
                self.nur_auffaellig = nur;
                if sig_ok {
                    let pfade: Vec<String> = auff.iter().filter_map(|i| m.procs.get(*i)).map(|p| p.exe.clone()).collect();
                    if stil::knopf_an(ui, !sig_laeuft && !pfade.is_empty(), Sym::Schild, t("Auffällige auf Signatur prüfen")).clicked() {
                        pruefe = pfade;
                    }
                    if sig_laeuft {
                        ui.spinner();
                    }
                }
            });
        });

        let zeigen: Vec<usize> = if self.nur_auffaellig { auff.clone() } else { (0..m.procs.len().min(200)).collect() };
        if zeigen.is_empty() {
            note(ui, t("Keine auffälligen Programme. Kein Programm braucht dauerhaft viel Leistung oder läuft aus einem ungewöhnlichen Ordner."));
        }
        let mut zeige_ordner: Option<String> = None;
        card(ui, |ui| {
            for (n, i) in zeigen.iter().enumerate() {
                let (Some(p), Some(st)) = (m.procs.get(*i), stufen.get(*i)) else { continue };
                if n > 0 {
                    ui.separator();
                }
                ui.horizontal(|ui| {
                    state_dot(ui, *st);
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(p.name.as_str()).strong());
                            ui.weak(format!("PID {}", p.pid));
                            if p.miner {
                                stil::marke(ui, t("MINER"), red(), 0.0);
                            }
                            if let Some(s) = p.imitation {
                                stil::marke(ui, t("NACHAHMUNG"), red(), 0.0).on_hover_text(tf("Gibt sich als «{s}» aus, liegt aber nicht im Windows-Systemordner.", &[("s", &s)]));
                            }
                            ort_marke(ui, p.ort);
                            if m.leerlauf_s.is_some_and(|l| l >= 120) && p.hohe_last(self.schwelle.min(50.0)) {
                                stil::marke(ui, t("LAST IM LEERLAUF"), orange(), 0.0);
                            } else if p.hohe_last(self.schwelle) {
                                stil::marke(ui, t("HOHE LAST"), amber(), 0.0);
                            }
                            if let Some(f) = p.fern {
                                stil::marke(ui, t("FERNZUGRIFF"), stil::violett(), 0.0).on_hover_text(f);
                            }
                            sig_marke(ui, self.sig(&p.exe));
                        });
                        if p.exe.is_empty() {
                            ui.weak(t("Pfad nicht lesbar (Systemprozess oder fehlende Rechte)"));
                        } else {
                            weak_trunc(ui, &p.exe);
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !p.exe.is_empty() {
                            if stil::leise(ui, true, Sym::Ordner, "", t("Im Ordner zeigen")).clicked() {
                                zeige_ordner = Some(p.exe.clone());
                            }
                            if sig_ok && stil::leise(ui, !sig_laeuft, Sym::Schild, "", t("Signatur prüfen")).clicked() {
                                pruefe = vec![p.exe.clone()];
                            }
                        }
                        let gpu = p.gpu.map(|g| format!(" · GPU {:.0} %", g)).unwrap_or_default();
                        ui.label(egui::RichText::new(format!("CPU {:.1} % · {}{}", p.cpu, human_size(p.mem), gpu)).monospace().size(12.0));
                    });
                });
            }
            if !self.nur_auffaellig && m.procs.len() > 200 {
                ui.weak(tf("… und {n} weitere mit wenig Last", &[("n", &(m.procs.len() - 200))]));
            }
        });
        erklaerung(
            ui,
            t("Markiert werden: bekannte Miner-Namen, Programme aus Temp/Downloads/öffentlichen Ordnern, nachgeahmte Windows-Namen (z. B. svchost.exe ausserhalb von System32), hohe Last während du nichts tust, und – nach der Prüfung – fehlende oder ungültige Signaturen. AppData allein ist häufig harmlos (Teams, Discord, Spotify …)."),
            t("Unbekanntes Programm? Signatur prüfen, Ordner ansehen und die Datei im Virenscan prüfen. Beenden kannst du ein Programm im Task-Manager (Windows) bzw. in der Aktivitätsanzeige (macOS)."),
        );
        if let Some(p) = zeige_ordner {
            sys::im_ordner_zeigen(&p);
        }
        if !pruefe.is_empty() {
            self.sig_pruefen(pruefe);
        }
    }

    // ---------------- Autostart ----------------

    pub(crate) fn page_autostart(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Autostart"), t("Was beim Start von selbst mitläuft – hier verstecken sich Spionage- und Schadprogramme gern."));
        if let Some(r) = &self.aktion_msg {
            match r {
                Ok(m) => ui.colored_label(grn(), *m),
                Err(e) => ui.colored_label(red(), e.as_str()),
            };
            ui.add_space(8.0);
        }
        let res = sperre(&self.autostart.data).clone();
        let Some(r) = res else {
            if self.autostart.running() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak(t("wird geprüft …"));
                });
            } else {
                stil::leer(ui, Sym::Play, t("Autostart"), t("Noch nicht geprüft – auf «Prüfen» klicken."));
            }
            return;
        };
        let sichtbar: Vec<&autostart::Eintrag> = r.eintraege.iter().filter(|e| self.auto_system || !e.system || e.neu).collect();
        let neu = r.eintraege.iter().filter(|e| e.neu).count();
        let aus = sichtbar.iter().filter(|e| e.aus).count();
        let mut uebernehmen = false;
        let mut frage: Option<Frage> = None;
        let mut zeige: Option<String> = None;

        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(tf("{n} Einträge", &[("n", &sichtbar.len())])).strong());
                if neu > 0 {
                    stil::marke(ui, &tf("{n} NEU", &[("n", &neu)]), stil::violett(), 0.0);
                }
                stil::grau(ui, &tf("· {n} ausgeschaltet", &[("n", &aus)]));
            });
            if r.basis_neu_angelegt {
                stil::hinweis(ui, t("Erste Prüfung: Der jetzige Stand wurde als bekannt gespeichert. Ab jetzt werden neue Einträge markiert."));
            } else if neu > 0 {
                stil::hinweis(ui, t("«Neu» heisst: seit dem letzten bekannten Stand dazugekommen oder geändert."));
            }
            for f in &r.fehler {
                ui.colored_label(amber(), *f);
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if stil::knopf_an(ui, neu > 0, Sym::Haken, t("Alle als bekannt übernehmen")).clicked() {
                    uebernehmen = true;
                }
            });
            stil::schalter(ui, &mut self.auto_system, t("Einträge des Betriebssystems zeigen"));
        });

        let gruppen = [
            (Art::Run, t("RUN-SCHLÜSSEL (REGISTRY)")),
            (Art::RunOnce, t("RUNONCE-SCHLÜSSEL (EINMALIG)")),
            (Art::Ordner, t("AUTOSTART-ORDNER")),
            (Art::Aufgabe, t("GEPLANTE AUFGABEN")),
            (Art::Dienst, t("DIENSTE AUSSERHALB VON C:\\WINDOWS")),
            (Art::LaunchAgent, t("LAUNCHAGENTS")),
            (Art::LaunchDaemon, t("LAUNCHDAEMONS")),
            (Art::Desktop, t("AUTOSTART (.DESKTOP)")),
            (Art::Systemd, t("SYSTEMD-BENUTZERDIENSTE")),
            (Art::Cron, t("CRONTAB")),
        ];
        for (art, titel) in gruppen {
            let liste: Vec<&&autostart::Eintrag> = sichtbar.iter().filter(|e| e.art == art).collect();
            if liste.is_empty() {
                continue;
            }
            card(ui, |ui| {
                abschnitt(ui, titel);
                for (n, e) in liste.iter().enumerate() {
                    if n > 0 {
                        ui.separator();
                    }
                    let st = Self::eintrag_stufe(e);
                    ui.horizontal(|ui| {
                        state_dot(ui, st);
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(e.name.as_str()).strong());
                                if e.neu {
                                    stil::marke(ui, t("NEU"), stil::violett(), 0.0);
                                }
                                if e.aus {
                                    stil::marke(ui, t("AUSGESCHALTET"), thema::muted(), 0.0);
                                }
                                if e.system {
                                    stil::marke(ui, t("SYSTEM"), thema::muted(), 0.0);
                                }
                                ort_marke(ui, crate::prozparse::ort_von(&e.exe, cfg!(windows)));
                                sig_marke(ui, e.sig.as_ref());
                            });
                            if !e.befehl.is_empty() {
                                weak_trunc(ui, &e.befehl);
                            }
                            ui.label(egui::RichText::new(e.ort.as_str()).size(11.0).color(thema::muted()));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let kann = !matches!(e.aktion, Aktion::Keine) && !e.aus;
                            if kann && stil::gefahr(ui, !self.aktion.running(), Sym::Stopp, t("Deaktivieren")).clicked() {
                                frage = Some(Frage::Deaktivieren((**e).clone()));
                            }
                            if !e.exe.is_empty() && stil::leise(ui, true, Sym::Ordner, "", t("Im Ordner zeigen")).clicked() {
                                zeige = Some(e.exe.clone());
                            }
                        });
                    });
                }
            });
        }

        // Gesicherte (deaktivierte) Einträge
        if !self.sicherungen.is_empty() {
            let sich = self.sicherungen.clone();
            card(ui, |ui| {
                abschnitt(ui, t("GESICHERTE EINTRÄGE"));
                for s in &sich {
                    ui.horizontal(|ui| {
                        stil::symbol(ui, Sym::Archiv, 15.0, thema::muted());
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(kurz_name(&s.name)).strong());
                            let wo = if s.art == "aufgabe" { tf("Geplante Aufgabe {n}", &[("n", &s.name)]) } else { format!("{}\\{}", s.hive, s.key) };
                            weak_trunc(ui, &wo);
                            if !s.daten.is_empty() {
                                weak_trunc(ui, &s.daten);
                            }
                            ui.weak(tf("deaktiviert {wann}", &[("wann", &vor(sys::jetzt_s().saturating_sub(s.zeit)))]));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if stil::knopf_an(ui, !self.aktion.running(), Sym::Rueckgaengig, t("Wiederherstellen")).clicked() {
                                frage = Some(Frage::Wiederherstellen(s.clone()));
                            }
                        });
                    });
                }
            });
        }
        if !cfg!(windows) {
            note(ui, t("Deaktivieren ist nur unter Windows eingebaut. Unter macOS/Linux die Datei im Ordner ansehen und selbst entscheiden."));
        }

        if uebernehmen {
            let ids: Vec<String> = r.eintraege.iter().map(|e| e.id.clone()).collect();
            match autostart::basis_speichern(&ids) {
                Ok(()) => {
                    if let Some(d) = sperre(&self.autostart.data).as_mut() {
                        for e in d.eintraege.iter_mut() {
                            e.neu = false;
                        }
                        d.basis_neu_angelegt = false;
                    }
                    self.aktion_msg = Some(Ok(t("Als bekannt gespeichert.")));
                }
                Err(e) => self.aktion_msg = Some(Err(e)),
            }
        }
        if frage.is_some() {
            self.frage = frage;
        }
        if let Some(p) = zeige {
            sys::im_ordner_zeigen(&p);
        }
    }

    // ---------------- Verbindungen ----------------

    pub(crate) fn page_verbindungen(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Verbindungen nach aussen"), t("Welche Programme gerade mit dem Internet oder dem Heimnetz verbunden sind – nur Lesen."));
        let list = sperre(&self.verb.data).clone();
        let Some(mut list) = list else {
            if self.verb.running() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak(t("wird geprüft …"));
                });
            } else {
                stil::leer(ui, Sym::Extern, t("Verbindungen"), t("Noch nicht geprüft – auf «Prüfen» klicken."));
            }
            return;
        };
        list.sort_by_key(|v| std::cmp::Reverse(crate::app::rang(self.verb_stufe(v))));
        let n_conn: usize = list.iter().map(|v| v.conns.len()).sum();
        let sig_ok = sys::signatur_moeglich();
        let sig_laeuft = self.sig_job.running();
        let mut pruefe: Vec<String> = Vec::new();
        let mut zeige: Option<String> = None;
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(tf("{p} Programme mit {c} Verbindungen", &[("p", &list.len()), ("c", &n_conn)])).strong());
                if sig_ok {
                    let pfade: Vec<String> = list.iter().filter(|v| v.ort.ungewoehnlich() || v.mining_port()).map(|v| v.exe.clone()).collect();
                    if stil::knopf_an(ui, !sig_laeuft && !pfade.is_empty(), Sym::Schild, t("Ungewöhnliche auf Signatur prüfen")).clicked() {
                        pruefe = pfade;
                    }
                    if sig_laeuft {
                        ui.spinner();
                    }
                }
            });
            stil::hinweis(ui, t("Namen der Gegenstellen (DNS) werden absichtlich nicht nachgeschlagen: das wäre langsam und würde selbst Anfragen ins Netz schicken."));
        });
        if list.is_empty() {
            note(ui, t("Gerade keine Verbindungen nach aussen."));
        }
        for v in &list {
            let st = self.verb_stufe(v);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    state_dot(ui, st);
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(v.name.as_str()).strong().size(14.0));
                            if v.pid > 0 {
                                ui.weak(format!("PID {}", v.pid));
                            }
                            if v.miner {
                                stil::marke(ui, t("MINER"), red(), 0.0);
                            }
                            if v.imitation.is_some() {
                                stil::marke(ui, t("NACHAHMUNG"), red(), 0.0);
                            }
                            if v.mining_port() {
                                stil::marke(ui, t("MINING-PORT"), orange(), 0.0);
                            }
                            ort_marke(ui, v.ort);
                            if let Some(f) = v.fern {
                                stil::marke(ui, t("FERNZUGRIFF"), stil::violett(), 0.0).on_hover_text(f);
                            }
                            sig_marke(ui, self.sig(&v.exe));
                        });
                        if !v.exe.is_empty() {
                            weak_trunc(ui, &v.exe);
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !v.exe.is_empty() {
                            if stil::leise(ui, true, Sym::Ordner, "", t("Im Ordner zeigen")).clicked() {
                                zeige = Some(v.exe.clone());
                            }
                            if sig_ok && stil::leise(ui, !sig_laeuft, Sym::Schild, "", t("Signatur prüfen")).clicked() {
                                pruefe = vec![v.exe.clone()];
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                for c in v.conns.iter().take(30) {
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        let adr = if c.remote_ip.contains(':') { format!("[{}]:{}", c.remote_ip, c.remote_port) } else { format!("{}:{}", c.remote_ip, c.remote_port) };
                        ui.label(egui::RichText::new(format!("→ {adr}")).monospace().size(12.0));
                        if let Some((name, mining)) = crate::netparse::port_name(c.remote_port) {
                            ui.label(egui::RichText::new(name).size(12.0).color(if mining { orange() } else { thema::muted() }));
                        }
                        if crate::netparse::ist_heimnetz(&c.remote_ip) {
                            ui.label(egui::RichText::new(t("Heimnetz")).size(12.0).color(thema::muted()));
                        }
                    });
                }
                if v.conns.len() > 30 {
                    ui.weak(tf("… und {n} weitere", &[("n", &(v.conns.len() - 30))]));
                }
            });
        }
        if let Some(p) = zeige {
            sys::im_ordner_zeigen(&p);
        }
        if !pruefe.is_empty() {
            self.sig_pruefen(pruefe);
        }
    }

    // ---------------- Kamera & Mikrofon ----------------

    pub(crate) fn page_kamera(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Kamera & Mikrofon"), t("Welche Programme Kamera und Mikrofon benutzt haben – und ob gerade eines zugreift."));
        let b = sperre(&self.kamera.data).clone();
        let Some(b) = b else {
            if self.kamera.running() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak(t("wird geprüft …"));
                });
            } else {
                stil::leer(ui, Sym::Kamera, t("Kamera & Mikrofon"), t("Noch nicht geprüft – auf «Prüfen» klicken."));
            }
            return;
        };
        if !b.verfuegbar {
            stil::leer(ui, Sym::Kamera, t("Unter diesem System nicht verfügbar"), t("macOS führt dieses Protokoll geschützt. Die erlaubten Apps stehen unter Systemeinstellungen → Datenschutz & Sicherheit → Kamera bzw. Mikrofon."));
            if cfg!(target_os = "macos") {
                ui.vertical_centered(|ui| {
                    if stil::knopf(ui, Sym::Extern, t("Datenschutz-Einstellungen öffnen")).clicked() {
                        sys::oeffnen("x-apple.systempreferences:com.apple.preference.security?Privacy_Camera");
                    }
                });
            }
            return;
        }
        let aktiv: Vec<&crate::zugriff::KamEintrag> = b.eintraege.iter().filter(|e| e.aktiv).collect();
        if aktiv.is_empty() {
            egui::Frame::none().fill(crate::app::grn_soft()).rounding(12.0).inner_margin(egui::Margin::symmetric(16.0, 12.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    stil::symbol(ui, Sym::Haken, 20.0, grn());
                    ui.label(egui::RichText::new(t("Gerade greift kein Programm auf Kamera oder Mikrofon zu.")).strong().color(grn()));
                });
            });
        } else {
            egui::Frame::none()
                .fill(stil::mix(red(), thema::panel(), 0.14))
                .stroke(egui::Stroke::new(1.5, red()))
                .rounding(12.0)
                .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        stil::symbol(ui, Sym::Aufnahme, 22.0, red());
                        ui.label(egui::RichText::new(t("GERADE AKTIV")).strong().size(16.0).color(red()));
                    });
                    ui.add_space(4.0);
                    for e in &aktiv {
                        let g = if e.geraet == Geraet::Kamera { t("Kamera") } else { t("Mikrofon") };
                        ui.label(egui::RichText::new(format!("{g}: {}", e.app)).strong().size(14.0));
                        if !e.pfad.is_empty() {
                            weak_trunc(ui, &e.pfad);
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(t("Telefonierst, filmst oder sprichst du gerade? Dann ist das normal. Sonst das Programm prüfen und beenden."));
                });
        }
        ui.add_space(14.0);
        if cfg!(windows) {
            card(ui, |ui| {
                abschnitt(ui, t("ZUGRIFF GRUNDSÄTZLICH"));
                for (name, erlaubt, ziel) in [(t("Kamera"), b.kamera_erlaubt, "ms-settings:privacy-webcam"), (t("Mikrofon"), b.mikro_erlaubt, "ms-settings:privacy-microphone")] {
                    ui.horizontal(|ui| {
                        let (s, txt) = match erlaubt {
                            Some(true) => (State::Ok, t("Apps dürfen zugreifen")),
                            Some(false) => (State::Ok, t("Für alle Apps gesperrt")),
                            None => (State::Unknown, t("Unbekannt")),
                        };
                        state_dot(ui, s);
                        ui.label(egui::RichText::new(name).strong());
                        ui.weak(txt);
                        stil::rechts(ui, |ui| {
                            if stil::leise(ui, true, Sym::Extern, t("Einstellungen"), "").clicked() {
                                sys::oeffnen(ziel);
                            }
                        });
                    });
                }
            });
        }
        let jetzt = sys::jetzt_s();
        for (geraet, titel) in [(Geraet::Kamera, t("KAMERA – VERLAUF")), (Geraet::Mikrofon, t("MIKROFON – VERLAUF"))] {
            let liste: Vec<&crate::zugriff::KamEintrag> = b.eintraege.iter().filter(|e| e.geraet == geraet).collect();
            card(ui, |ui| {
                abschnitt(ui, titel);
                if liste.is_empty() {
                    ui.weak(t("Keine Einträge."));
                }
                for e in liste.iter().take(60) {
                    ui.horizontal(|ui| {
                        state_dot(ui, if e.aktiv { State::Bad } else { State::Ok });
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(e.app.as_str()).strong());
                            if !e.pfad.is_empty() && e.pfad != e.app {
                                weak_trunc(ui, &short_path(&e.pfad));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if e.aktiv {
                                ui.label(egui::RichText::new(t("gerade aktiv")).color(red()).strong());
                            } else if e.start > 0 {
                                let s = sys::filetime_zu_unix(e.start);
                                let lang = sys::filetime_zu_unix(e.stop).saturating_sub(s);
                                ui.weak(tf("{wann} · {dauer} lang", &[("wann", &vor(jetzt.saturating_sub(s))), ("dauer", &dauer(lang))]))
                                    .on_hover_text(sys::filetime_text(e.start));
                            }
                        });
                    });
                }
            });
        }
        if !cfg!(windows) {
            note(ui, t("Unter Linux zeigt Schutzwerk nur, welche eigenen Programme Kamera oder Mikrofon gerade offen haben; einen Verlauf führt Linux nicht."));
        }
    }

    // ---------------- Fernzugriff ----------------

    pub(crate) fn page_fern(&mut self, ui: &mut egui::Ui) {
        h1(ui, t("Fernzugriff"), t("Programme und Einstellungen, über die jemand diesen Rechner aus der Ferne bedienen könnte."));
        let b = sperre(&self.fern.data).clone();
        let Some(b) = b else {
            if self.fern.running() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak(t("wird geprüft …"));
                });
            } else {
                stil::leer(ui, Sym::Auge, t("Fernzugriff"), t("Noch nicht geprüft – auf «Prüfen» klicken."));
            }
            return;
        };
        let zeile = |ui: &mut egui::Ui, an: Option<bool>, titel: &str, an_txt: &str, aus_txt: &str| {
            ui.horizontal(|ui| {
                let s = match an {
                    Some(true) => State::Warn,
                    Some(false) => State::Ok,
                    None => State::Unknown,
                };
                state_dot(ui, s);
                ui.label(egui::RichText::new(titel).strong().size(14.0));
                stil::rechts(ui, |ui| {
                    let txt = match an {
                        Some(true) => an_txt,
                        Some(false) => aus_txt,
                        None => t("Unbekannt"),
                    };
                    ui.label(egui::RichText::new(txt).color(state_color(s)).strong());
                });
            });
        };
        if cfg!(windows) {
            card(ui, |ui| {
                zeile(ui, b.rdp, t("Remotedesktop"), t("Eingeschaltet"), t("Ausgeschaltet"));
                if b.rdp == Some(true) {
                    ui.add_space(6.0);
                    erklaerung(
                        ui,
                        t("Andere Rechner können sich mit Benutzername und Passwort anmelden und den Bildschirm übernehmen. Für Angreifer ein beliebtes Ziel, besonders wenn der Rechner aus dem Internet erreichbar ist."),
                        t("Brauchst du es nicht, schalte es in den Einstellungen aus. Schutzwerk ändert das nicht selbst."),
                    );
                }
                ui.add_space(6.0);
                if stil::knopf(ui, Sym::Extern, t("Einstellungen öffnen")).clicked() {
                    sys::oeffnen("ms-settings:remotedesktop");
                }
            });
            card(ui, |ui| {
                zeile(ui, b.ra, t("Remoteunterstützung"), t("Erlaubt"), t("Nicht erlaubt"));
                if b.ra == Some(true) {
                    ui.add_space(6.0);
                    erklaerung(
                        ui,
                        t("Über eine Einladung kann jemand deinen Bildschirm sehen und bedienen. Ohne Einladung geht es nicht – aber Betrüger am Telefon bitten gern darum."),
                        t("Brauchst du es nicht: Systemeigenschaften → Remote → Häkchen bei «Remoteunterstützungsverbindungen … zulassen» entfernen."),
                    );
                }
                ui.add_space(6.0);
                if stil::knopf(ui, Sym::Extern, t("Systemeigenschaften öffnen")).clicked() {
                    sys::starten("SystemPropertiesRemote.exe");
                }
            });
        } else {
            card(ui, |ui| {
                zeile(ui, b.ssh, t("SSH-Server"), t("Läuft"), t("Läuft nicht"));
                if b.ssh == Some(true) {
                    ui.add_space(6.0);
                    erklaerung(
                        ui,
                        t("Über SSH kann sich jemand mit Passwort oder Schlüssel an diesem Rechner anmelden und Befehle ausführen."),
                        if cfg!(target_os = "macos") {
                            t("Brauchst du es nicht: Systemeinstellungen → Allgemein → Teilen → «Entfernte Anmeldung» ausschalten.")
                        } else {
                            t("Brauchst du es nicht: im Terminal «sudo systemctl disable --now ssh» (bzw. sshd) ausführen.")
                        },
                    );
                }
            });
        }
        let mut zeige: Option<String> = None;
        card(ui, |ui| {
            abschnitt(ui, t("FERNWARTUNGS-PROGRAMME"));
            if b.tools.is_empty() {
                ui.colored_label(grn(), t("Keine bekannten Fernwartungs-Programme gefunden."));
            }
            for x in &b.tools {
                ui.horizontal(|ui| {
                    state_dot(ui, if x.laeuft || x.dienst_auto { State::Warn } else { State::Ok });
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(x.produkt).strong());
                            if x.laeuft {
                                stil::marke(ui, t("LÄUFT"), orange(), 0.0);
                            }
                            if x.dienst_auto {
                                stil::marke(ui, t("STARTET AUTOMATISCH"), amber(), 0.0);
                            }
                            if !x.laeuft && !x.dienst_auto {
                                stil::marke(ui, t("INSTALLIERT"), thema::muted(), 0.0);
                            }
                        });
                        weak_trunc(ui, if x.pfad.is_empty() { &x.quelle } else { &x.pfad });
                    });
                    if !x.pfad.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if stil::leise(ui, true, Sym::Ordner, "", t("Im Ordner zeigen")).clicked() {
                                zeige = Some(x.pfad.clone());
                            }
                        });
                    }
                });
                ui.add_space(4.0);
            }
            ui.add_space(6.0);
            erklaerung(
                ui,
                t("Fernwartungs-Programme sind nützlich, etwa für Hilfe aus der Familie oder vom Support. Betrüger am Telefon («Microsoft-Support», «Ihre Bank») lassen sie aber gern installieren."),
                t("Kennst du ein Programm nicht oder hast es nicht selbst installiert: deinstallieren (Einstellungen → Apps) und danach wichtige Passwörter ändern."),
            );
        });
        if let Some(p) = zeige {
            sys::im_ordner_zeigen(&p);
        }
    }
}
