//! Menüleiste wie VLC: Medien, Wiedergabe, Audio, Video, Untertitel, Werkzeuge, Ansicht,
//! Hilfe. Dieselben Menüs gibt es mit einem Rechtsklick ins Bild.

use super::*;

use egui::{Sense, Stroke, Ui};

use crate::stil::Sym;

/// Menüeintrag: optional Häkchen, Text, rechts das Tastenkürzel. Liefert true beim Klick
/// (und schliesst das Menü).
fn eintrag(ui: &mut Ui, an: bool, haken: Option<bool>, text: &str, kurz: &str) -> bool {
    let kurz = kurz_text(kurz);
    let fg = if an { thema::text() } else { thema::muted() };
    let g = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(13.5), fg);
    let gk = ui.painter().layout_no_wrap(kurz, egui::FontId::proportional(12.0), thema::muted());
    // Alle Einträge eines Menüs gleich breit: die grösste Breite merken.
    let key = ui.id().with("abspiel-menue-breite");
    let bisher = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(0.0);
    let w = (g.size().x + gk.size().x + 72.0).max(bisher).max(ui.available_width()).max(210.0);
    if w > bisher {
        ui.data_mut(|d| d.insert_temp(key, w));
    }
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 26.0), if an { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    if an && resp.hovered() {
        p.rect_filled(rect, 5.0, thema::hover());
    }
    if haken == Some(true) {
        stil::sym(p, Rect::from_center_size(Pos2::new(rect.left() + 13.0, rect.center().y), Vec2::splat(14.0)), Sym::Haken, if an { stil::akzent() } else { thema::muted() });
    }
    p.galley(Pos2::new(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, fg);
    if gk.size().x > 0.0 {
        p.galley(Pos2::new(rect.right() - 8.0 - gk.size().x, rect.center().y - gk.size().y / 2.0), gk, thema::muted());
    }
    if resp.clicked() {
        ui.close_menu();
        true
    } else {
        false
    }
}

fn knopf(ui: &mut Ui, an: bool, text: &str, kurz: &str) -> bool {
    eintrag(ui, an, None, text, kurz)
}

fn wahl(ui: &mut Ui, an: bool, gewaehlt: bool, text: &str, kurz: &str) -> bool {
    eintrag(ui, an, Some(gewaehlt), text, kurz)
}

/// Untermenü (ausgegraut, wenn `an` falsch ist).
fn unter(ui: &mut Ui, an: bool, text: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_enabled_ui(an, |ui| {
        ui.menu_button(text, |ui| {
            ui.set_min_width(200.0);
            add(ui);
        });
    });
}

fn leer_hinweis(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.5).color(thema::muted()));
}

impl App {
    pub(super) fn menueleiste(&mut self, ctx: &egui::Context, aktionen: &mut Vec<Aktion>) {
        let info = match &self.engine {
            Engine::Bereit(p) => format!("libVLC {}", p.version()),
            Engine::Laedt(_) => t("libVLC wird geladen …").to_string(),
            Engine::Fehlt(_) => vlc_fehlt_text().to_string(),
        };
        egui::TopBottomPanel::top("abspiel-menue")
            .frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::symmetric(10.0, 4.0)).stroke(Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    stil::titel(ui, "Abspielwerk");
                    self.menues(ui, aktionen);
                    stil::rechts(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(info).size(12.0).color(thema::muted())).truncate().selectable(false));
                    });
                });
            });
    }

    fn menues(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.menu_button(t("Medien"), |ui| self.menue_medien(ui, a));
        ui.menu_button(t("Wiedergabe"), |ui| self.menue_wiedergabe(ui, a));
        ui.menu_button(t("Audio"), |ui| self.menue_audio(ui, a));
        ui.menu_button(t("Video"), |ui| self.menue_video(ui, a));
        ui.menu_button(t("Untertitel"), |ui| self.menue_untertitel(ui, a));
        ui.menu_button(t("Werkzeuge"), |ui| self.menue_werkzeuge(ui, a));
        ui.menu_button(t("Ansicht"), |ui| self.menue_ansicht(ui, a));
        ui.menu_button(t("Hilfe"), |ui| self.menue_hilfe(ui, a));
    }

    /// Rechtsklick ins Bild: das Wichtigste direkt, der Rest in Untermenüs.
    pub(super) fn kontextmenue(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(220.0);
        let medium = self.medium_player().is_some();
        let spielt = medium && matches!(self.zustand, Zustand::Spielt | Zustand::Puffert | Zustand::Oeffnet);
        if knopf(ui, !self.liste.is_empty(), if spielt { t("Pause") } else { t("Abspielen") }, "Leertaste") {
            a.push(Aktion::PlayPause);
        }
        if knopf(ui, medium, t("Stopp"), "S") {
            a.push(Aktion::Stopp);
        }
        if knopf(ui, !self.liste.is_empty(), t("Vorheriges"), "P") {
            a.push(Aktion::Zurueck);
        }
        if knopf(ui, !self.liste.is_empty(), t("Nächstes"), "N") {
            a.push(Aktion::Weiter);
        }
        ui.separator();
        if wahl(ui, true, self.vollbild, t("Vollbild"), "F") {
            a.push(Aktion::Vollbild);
        }
        if wahl(ui, true, self.minimal, t("Minimale Ansicht"), "Strg+H") {
            a.push(Aktion::Minimal);
        }
        ui.separator();
        unter(ui, true, t("Medien"), |ui| self.menue_medien(ui, a));
        unter(ui, true, t("Wiedergabe"), |ui| self.menue_wiedergabe(ui, a));
        unter(ui, true, t("Audio"), |ui| self.menue_audio(ui, a));
        unter(ui, true, t("Video"), |ui| self.menue_video(ui, a));
        unter(ui, true, t("Untertitel"), |ui| self.menue_untertitel(ui, a));
        unter(ui, true, t("Werkzeuge"), |ui| self.menue_werkzeuge(ui, a));
        unter(ui, true, t("Ansicht"), |ui| self.menue_ansicht(ui, a));
        unter(ui, true, t("Hilfe"), |ui| self.menue_hilfe(ui, a));
    }

    fn menue_medien(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(260.0);
        if knopf(ui, true, t("Datei öffnen …"), "Strg+O") {
            a.push(Aktion::Oeffnen);
        }
        if knopf(ui, true, t("Mehrere Dateien öffnen …"), "Strg+Umschalt+O") {
            a.push(Aktion::MehrereOeffnen);
        }
        if knopf(ui, true, t("Ordner öffnen …"), "Strg+F") {
            a.push(Aktion::OrdnerOeffnen);
        }
        let netz = self.player().is_some_and(|p| p.kann_adresse());
        if knopf(ui, netz, t("Netzwerkstream öffnen …"), "Strg+N") {
            a.push(Aktion::NetzwerkDialog);
        }
        unter(ui, !self.zuletzt.v.is_empty(), t("Zuletzt geöffnet"), |ui| {
            for (i, s) in self.zuletzt.v.iter().enumerate() {
                let name = if merken::ist_url(s) { s.clone() } else { Path::new(s).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.clone()) };
                let r = ui.add(egui::Button::new(format!("{}  {name}", i + 1)).wrap_mode(egui::TextWrapMode::Truncate)).on_hover_text(s.as_str());
                if r.clicked() {
                    a.push(Aktion::ZuletztOeffnen(i));
                    ui.close_menu();
                }
            }
            ui.separator();
            if knopf(ui, true, t("Liste leeren"), "") {
                a.push(Aktion::ZuletztLeeren);
            }
        });
        ui.separator();
        if knopf(ui, !self.liste.is_empty(), t("Wiedergabeliste speichern …"), "Strg+Y") {
            a.push(Aktion::ListeSpeichern);
        }
        if knopf(ui, !self.liste.is_empty(), t("Wiedergabeliste leeren"), "") {
            a.push(Aktion::Leeren);
        }
        let datei = self.aktuell.and_then(|i| self.liste.get(i)).is_some_and(|e| e.url.is_none());
        if knopf(ui, datei, t("In Papierkorb …"), "") {
            a.push(Aktion::Papierkorb);
        }
        ui.separator();
        if knopf(ui, true, t("Beenden"), "Strg+Q") {
            a.push(Aktion::Beenden);
        }
    }

    fn menue_wiedergabe(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(260.0);
        let p = self.medium_player();
        let medium = p.is_some();
        let bild = self.aktuelle_art() == Some(Art::Bild);
        if let Some(p) = p {
            let (n, akt) = p.titel_info();
            if n > 1 {
                unter(ui, true, t("Titel"), |ui| {
                    for i in 0..n.min(200) {
                        if wahl(ui, true, i == akt, &tf("Titel {n}", &[("n", &(i + 1))]), "") {
                            a.push(Aktion::Titel(i));
                        }
                    }
                });
            }
            let (n, akt) = p.kapitel_info();
            if n > 1 {
                unter(ui, true, t("Kapitel"), |ui| {
                    for i in 0..n.min(500) {
                        if wahl(ui, true, i == akt, &tf("Kapitel {n}", &[("n", &(i + 1))]), "") {
                            a.push(Aktion::Kapitel(i));
                        }
                    }
                });
            }
        }
        unter(ui, true, t("Geschwindigkeit"), |ui| {
            if knopf(ui, true, t("Schneller"), "+") {
                a.push(Aktion::Schneller);
            }
            if knopf(ui, true, t("Normale Geschwindigkeit"), "=") {
                a.push(Aktion::TempoNormal);
            }
            if knopf(ui, true, t("Langsamer"), "-") {
                a.push(Aktion::Langsamer);
            }
            ui.separator();
            for r in befehl::TEMPI {
                if wahl(ui, true, (r - self.tempo).abs() < 1e-3, &befehl::tempo_text(r), "") {
                    a.push(Aktion::Tempo(r));
                }
            }
        });
        ui.separator();
        if knopf(ui, medium, t("Sprung vorwärts"), "→") {
            a.push(Aktion::Spulen(befehl::SPRUNG_KURZ));
        }
        if knopf(ui, medium, t("Sprung rückwärts"), "←") {
            a.push(Aktion::Spulen(-befehl::SPRUNG_KURZ));
        }
        if knopf(ui, medium, t("Zu Zeit springen …"), "Strg+T") {
            a.push(Aktion::ZeitDialog);
        }
        ui.separator();
        let spielt = medium && matches!(self.zustand, Zustand::Spielt | Zustand::Puffert | Zustand::Oeffnet);
        let txt = if bild {
            if self.diashow {
                t("Diashow beenden")
            } else {
                t("Diashow starten")
            }
        } else if spielt {
            t("Pause")
        } else {
            t("Abspielen")
        };
        if knopf(ui, !self.liste.is_empty(), txt, "Leertaste") {
            a.push(Aktion::PlayPause);
        }
        if knopf(ui, medium || self.diashow, t("Stopp"), "S") {
            a.push(Aktion::Stopp);
        }
        if knopf(ui, !self.liste.is_empty(), t("Vorheriges"), "P") {
            a.push(Aktion::Zurueck);
        }
        if knopf(ui, !self.liste.is_empty(), t("Nächstes"), "N") {
            a.push(Aktion::Weiter);
        }
        if knopf(ui, p.is_some_and(|p| p.kann_bild_fuer_bild()) && self.aktuelle_art() == Some(Art::Video), t("Bild für Bild"), "E") {
            a.push(Aktion::BildFuerBild);
        }
        ui.separator();
        unter(ui, true, t("Wiederholen"), |ui| {
            for (w, k, txt) in [(Wdh::Aus, 0u8, t("Nicht wiederholen")), (Wdh::Alle, 1, t("Ganze Liste wiederholen")), (Wdh::Eins, 2, t("Aktuelles Stück wiederholen"))] {
                if wahl(ui, true, self.wdh == w, txt, "") {
                    a.push(Aktion::Wdh(k));
                }
            }
        });
        if wahl(ui, true, self.zufall, t("Zufällige Reihenfolge"), "R") {
            a.push(Aktion::ZufallUmschalten);
        }
    }

    fn menue_audio(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(240.0);
        let p = self.medium_player();
        unter(ui, p.is_some(), t("Tonspur"), |ui| {
            let Some(p) = p else { return };
            let aktiv = p.tonspur();
            let spuren = p.tonspuren();
            if spuren.is_empty() {
                leer_hinweis(ui, t("Keine Tonspur"));
            }
            for (id, name) in spuren {
                if wahl(ui, true, id == aktiv, &spur_name(id, &name), "") {
                    a.push(Aktion::Tonspur(id));
                }
            }
        });
        let geraete = self.player().filter(|p| p.kann_audiogeraet());
        unter(ui, geraete.is_some(), t("Audiogerät"), |ui| {
            let Some(p) = geraete else { return };
            let aktiv = p.audiogeraet();
            let liste = p.audiogeraete();
            if liste.is_empty() {
                leer_hinweis(ui, t("Keine Audiogeräte gemeldet (erst während der Wiedergabe)."));
            }
            for (id, name) in liste {
                let txt = if name.is_empty() { id.clone() } else { name };
                if wahl(ui, true, aktiv.as_deref() == Some(id.as_str()), &txt, "") {
                    a.push(Aktion::Audiogeraet(id));
                }
            }
        });
        let stereo = p.filter(|p| p.kann_stereo());
        unter(ui, stereo.is_some(), t("Stereo-Modus"), |ui| {
            let Some(p) = stereo else { return };
            let aktiv = p.stereo();
            for (m, txt) in [(1, t("Stereo")), (2, t("Umgekehrtes Stereo")), (3, t("Links")), (4, t("Rechts")), (5, t("Dolby Surround"))] {
                if wahl(ui, true, aktiv == m, txt, "") {
                    a.push(Aktion::StereoModus(m));
                }
            }
        });
        ui.separator();
        if knopf(ui, self.lautstaerke < MAX_LAUTSTAERKE, t("Lauter"), "Strg+↑") {
            a.push(Aktion::Lauter);
        }
        if knopf(ui, self.lautstaerke > 0.0, t("Leiser"), "Strg+↓") {
            a.push(Aktion::Leiser);
        }
        if wahl(ui, true, self.stumm, t("Stumm"), "M") {
            a.push(Aktion::Stumm);
        }
    }

    fn menue_video(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(240.0);
        let video = self.aktuelle_art() == Some(Art::Video);
        let bild = self.aktuelle_art() == Some(Art::Bild);
        if wahl(ui, true, self.vollbild, t("Vollbild"), "F") {
            a.push(Aktion::Vollbild);
        }
        ui.separator();
        unter(ui, true, t("Seitenverhältnis"), |ui| {
            for i in 0..befehl::SEITEN.len() {
                let k = if i == 0 { "A" } else { "" };
                if wahl(ui, true, self.seiten == i, &seiten_name(i), k) {
                    a.push(Aktion::Seitenverhaeltnis(i));
                }
            }
        });
        unter(ui, true, t("Zuschneiden"), |ui| {
            for i in 0..befehl::ZUSCHNITTE.len() {
                let k = if i == 0 { "C" } else { "" };
                if wahl(ui, true, self.zuschnitt == i, &zuschnitt_name(i), k) {
                    a.push(Aktion::Zuschnitt(i));
                }
            }
        });
        let dei = self.player().is_some_and(|p| p.kann_deinterlace());
        unter(ui, dei, t("Deinterlace"), |ui| {
            for i in 0..befehl::DEINTERLACE.len() {
                let k = if i == 0 { "D" } else { "" };
                if wahl(ui, true, self.deinterlace == i, &deinterlace_name(i), k) {
                    a.push(Aktion::Deinterlace(i));
                }
            }
        });
        ui.separator();
        if knopf(ui, video && self.video_tex.is_some(), t("Bildschirmfoto"), "Umschalt+S") {
            a.push(Aktion::Schnappschuss);
        }
        ui.separator();
        if knopf(ui, bild, t("Nach links drehen"), "") {
            a.push(Aktion::Drehen(3));
        }
        if knopf(ui, bild, t("Nach rechts drehen"), "") {
            a.push(Aktion::Drehen(1));
        }
        if knopf(ui, bild, t("Einpassen"), "") {
            a.push(Aktion::Einpassen);
        }
        let hat_bilder = self.liste.iter().any(|e| e.art == Art::Bild);
        let txt = if self.diashow { t("Diashow beenden") } else { t("Diashow starten") };
        if knopf(ui, hat_bilder, txt, "") {
            a.push(Aktion::Diashow);
        }
    }

    fn menue_untertitel(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(260.0);
        let p = self.medium_player().filter(|_| self.aktuelle_art() == Some(Art::Video));
        if knopf(ui, p.is_some_and(|p| p.kann_untertitel_datei()), t("Untertiteldatei hinzufügen …"), "") {
            a.push(Aktion::UntertitelDatei);
        }
        unter(ui, p.is_some(), t("Untertitelspur"), |ui| {
            let Some(p) = p else { return };
            let aktiv = p.untertitel();
            let spuren = p.untertitel_liste();
            if spuren.is_empty() {
                leer_hinweis(ui, t("Keine Untertitel"));
            }
            for (id, name) in spuren {
                if wahl(ui, true, id == aktiv, &spur_name(id, &name), "") {
                    a.push(Aktion::Untertitel(id));
                }
            }
        });
        ui.separator();
        let v = p.is_some_and(|p| p.kann_untertitel_verzoegerung());
        leer_hinweis(ui, &tf("Verzögerung: {ms} ms", &[("ms", &self.spu_verzoegerung)]));
        if knopf(ui, v, t("Verzögerung +50 ms"), "H") {
            a.push(Aktion::UntertitelVerzoegerung(befehl::SPU_SCHRITT_MS));
        }
        if knopf(ui, v, t("Verzögerung −50 ms"), "G") {
            a.push(Aktion::UntertitelVerzoegerung(-befehl::SPU_SCHRITT_MS));
        }
        if knopf(ui, v && self.spu_verzoegerung != 0, t("Verzögerung zurücksetzen"), "") {
            a.push(Aktion::UntertitelVerzoegerungNull);
        }
    }

    fn menue_werkzeuge(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(260.0);
        if wahl(ui, true, self.fenster.effekte, t("Effekte und Filter"), "Strg+E") {
            a.push(Aktion::Effekte);
        }
        if wahl(ui, self.aktuell.is_some(), self.fenster.info, t("Medieninformationen"), "Strg+I") {
            a.push(Aktion::Medieninfo);
        }
        ui.separator();
        if wahl(ui, true, self.fenster.einstellungen, t("Einstellungen"), "Strg+P") {
            a.push(Aktion::Einstellungen);
        }
    }

    fn menue_ansicht(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(260.0);
        if wahl(ui, true, self.liste_sichtbar, t("Wiedergabeliste"), "Strg+L") {
            a.push(Aktion::ListeUmschalten);
        }
        if wahl(ui, true, self.minimal, t("Minimale Ansicht"), "Strg+H") {
            a.push(Aktion::Minimal);
        }
        if wahl(ui, true, self.oben, t("Immer im Vordergrund"), "") {
            a.push(Aktion::ImmerOben);
        }
        if wahl(ui, true, self.vollbild, t("Vollbild"), "F") {
            a.push(Aktion::Vollbild);
        }
    }

    fn menue_hilfe(&self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.set_min_width(220.0);
        if knopf(ui, true, t("Tastenkürzel"), "F1") {
            a.push(Aktion::Tastenkuerzel);
        }
        if knopf(ui, true, t("Über Abspielwerk"), "") {
            a.push(Aktion::Ueber);
        }
    }
}
