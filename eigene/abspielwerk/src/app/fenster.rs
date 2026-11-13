//! Fenster: Netzwerkstream, Zu Zeit springen, Effekte und Filter (Equalizer, Video-
//! Anpassungen), Medieninformationen, Einstellungen, Tastenkürzel, Über, Rückfrage Papierkorb.

use super::*;

use egui::{RichText, Ui};

use crate::stil::Sym;

/// Tastenkürzel für Hilfe › Tastenkürzel (Taste, Wirkung).
fn tastenliste() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Leertaste", t("Abspielen / Pause (Bilder: Diashow)")),
        ("S", t("Stopp")),
        ("N / P", t("Nächstes / Vorheriges")),
        ("F, Doppelklick", t("Vollbild")),
        ("Esc", t("Vollbild beenden")),
        ("← / →", t("10 s zurück / vor (Bilder: blättern)")),
        ("Alt+← / →", t("10 s zurück / vor (kurzer Sprung)")),
        ("Umschalt+← / →", t("3 s zurück / vor (sehr kurzer Sprung)")),
        ("Strg+← / →", t("1 min zurück / vor (mittlerer Sprung)")),
        ("Strg+Alt+← / →", t("5 min zurück / vor (langer Sprung)")),
        ("↑ / ↓, Strg+↑ / ↓", t("Lauter / leiser (bis 125 %)")),
        ("Mausrad", t("Lauter / leiser (Bilder: zoomen)")),
        ("M", t("Stumm")),
        ("+ / - / =", t("Schneller / langsamer / normale Geschwindigkeit")),
        ("E", t("Bild für Bild")),
        ("T", t("Zeit anzeigen")),
        ("Umschalt+S", t("Bildschirmfoto")),
        ("B", t("Tonspur wechseln")),
        ("V", t("Untertitel wechseln")),
        ("G / H", t("Untertitel-Verzögerung −50 / +50 ms")),
        ("A", t("Seitenverhältnis wechseln")),
        ("C", t("Zuschneiden wechseln")),
        ("D", t("Deinterlace wechseln")),
        ("L", t("Wiederholen wechseln")),
        ("R", t("Zufall ein / aus")),
        ("Entf", t("Gewählten Eintrag aus der Liste nehmen (Bilder: In Papierkorb)")),
        ("Strg+O", t("Datei öffnen")),
        ("Strg+Umschalt+O", t("Mehrere Dateien öffnen")),
        ("Strg+F", t("Ordner öffnen")),
        ("Strg+N", t("Netzwerkstream öffnen")),
        ("Strg+Y", t("Wiedergabeliste speichern")),
        ("Strg+L", t("Wiedergabeliste ein / aus")),
        ("Strg+T", t("Zu Zeit springen")),
        ("Strg+H", t("Minimale Ansicht")),
        ("Strg+E", t("Effekte und Filter")),
        ("Strg+I", t("Medieninformationen")),
        ("Strg+P", t("Einstellungen")),
        ("Strg+Q", t("Beenden")),
        ("F1", t("Tastenkürzel")),
    ]
}

fn zeile(ui: &mut Ui, k: &str, v: &str) {
    ui.label(RichText::new(k).color(thema::muted()));
    ui.add(egui::Label::new(v).wrap());
    ui.end_row();
}

fn khz(f: f32) -> String {
    if f >= 1000.0 {
        format!("{} kHz", (f / 100.0).round() / 10.0)
    } else {
        format!("{f:.0} Hz")
    }
}

impl App {
    pub(super) fn fenster_zeigen(&mut self, ctx: &egui::Context, a: &mut Vec<Aktion>) {
        self.papierkorb_frage(ctx);
        self.netzwerk_fenster(ctx);
        self.zeit_fenster(ctx, a);
        self.effekte_fenster(ctx);
        self.info_fenster(ctx);
        self.einstellungen_fenster(ctx);
        self.tasten_fenster(ctx);
        self.ueber_fenster(ctx);
    }

    fn papierkorb_frage(&mut self, ctx: &egui::Context) {
        let Some(i) = self.frage_papierkorb else { return };
        let Some(e) = self.liste.get(i) else {
            self.frage_papierkorb = None;
            return;
        };
        let name = e.name.clone();
        let (mut ja, mut nein) = (false, false);
        egui::Window::new(t("In Papierkorb verschieben?")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
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

    fn netzwerk_fenster(&mut self, ctx: &egui::Context) {
        if !self.fenster.netzwerk {
            return;
        }
        let mut offen = true;
        let mut los = false;
        let mut zu = false;
        egui::Window::new(t("Netzwerkstream öffnen")).open(&mut offen).collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
            stil::feld(ui, t("Netzwerkadresse"), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.netz_url).hint_text("https://… · rtsp://… · udp://@:1234").desired_width(420.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    los = true;
                }
                r.request_focus();
            });
            stil::hinweis(ui, t("z. B. http://, https://, rtsp://, rtp://, udp://, mms:// oder eine Adresse zu einer Video- oder Musikdatei."));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if stil::haupt(ui, !self.netz_url.trim().is_empty(), Sym::Play, t("Abspielen")).clicked() {
                    los = true;
                }
                if stil::knopf_text(ui, true, t("Abbrechen")).clicked() {
                    zu = true;
                }
            });
        });
        if los {
            let url = self.netz_url.clone();
            if self.adresse_oeffnen(ctx, &url) {
                zu = true;
            }
        }
        if !offen || zu || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.fenster.netzwerk = false;
        }
    }

    fn zeit_fenster(&mut self, ctx: &egui::Context, a: &mut Vec<Aktion>) {
        if !self.fenster.zeit {
            return;
        }
        let mut offen = true;
        let mut los = false;
        let mut zu = false;
        let laenge = self.laenge;
        egui::Window::new(t("Zu Zeit springen")).open(&mut offen).collapsible(false).resizable(false).show(ctx, |ui| {
            stil::feld(ui, t("Zeit (h:mm:ss, mm:ss oder Sekunden)"), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.zeit_eingabe).desired_width(160.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    los = true;
                }
                r.request_focus();
            });
            if laenge > 0 {
                stil::hinweis(ui, &tf("Länge: {t}", &[("t", &medien::zeit_text(laenge))]));
            }
            if !self.zeit_eingabe.trim().is_empty() && medien::zeit_parsen(&self.zeit_eingabe).is_none() {
                ui.label(RichText::new(t("Unbekanntes Zeitformat")).color(stil::rot()).size(12.0));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if stil::haupt(ui, medien::zeit_parsen(&self.zeit_eingabe).is_some(), Sym::Ende, t("Springen")).clicked() {
                    los = true;
                }
                if stil::knopf_text(ui, true, t("Abbrechen")).clicked() {
                    zu = true;
                }
            });
        });
        if los {
            if let Some(ms) = medien::zeit_parsen(&self.zeit_eingabe) {
                let ms = if laenge > 0 { ms.min(laenge) } else { ms };
                a.push(Aktion::Springen(ms));
                zu = true;
            }
        }
        if !offen || zu || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.fenster.zeit = false;
        }
    }

    fn effekte_fenster(&mut self, ctx: &egui::Context) {
        if !self.fenster.effekte {
            return;
        }
        let mut offen = true;
        let (kann_eq, kann_anp) = self.player().map(|p| (p.kann_equalizer(), p.kann_anpassen())).unwrap_or((false, false));
        let eq_alt = self.eq.clone();
        let anp_alt = self.anpassung.clone();
        let mut vorlage_neu: Option<usize> = None;
        egui::Window::new(t("Effekte und Filter")).open(&mut offen).collapsible(false).resizable(false).default_width(560.0).show(ctx, |ui| {
            let mut reiter = self.fenster.effekte_reiter;
            stil::wahl(ui, &mut reiter, &[(0u8, t("Audio-Equalizer")), (1u8, t("Video-Anpassungen"))]);
            self.fenster.effekte_reiter = reiter;
            ui.add_space(8.0);
            if reiter == 0 {
                if !kann_eq {
                    stil::hinweis(ui, t("Dieses libVLC hat keinen Equalizer (oder es ist noch nicht geladen)."));
                }
                ui.add_enabled_ui(kann_eq, |ui| {
                    stil::schalter(ui, &mut self.eq.an, t("Equalizer einschalten"));
                    ui.add_enabled_ui(self.eq.an, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(t("Vorlage"));
                            let text = self.eq.vorlage.and_then(|i| self.eq_vorlagen.get(i)).cloned().unwrap_or_else(|| t("Eigene").to_string());
                            egui::ComboBox::from_id_salt("abspiel-eq-vorlage").selected_text(text).width(200.0).show_ui(ui, |ui| {
                                for (i, n) in self.eq_vorlagen.iter().enumerate() {
                                    if ui.selectable_label(self.eq.vorlage == Some(i), n.as_str()).clicked() {
                                        vorlage_neu = Some(i);
                                    }
                                }
                            });
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().slider_width = 150.0;
                            ui.vertical(|ui| {
                                ui.add(egui::Slider::new(&mut self.eq.vor, -20.0..=20.0).vertical().fixed_decimals(1).show_value(false));
                                ui.label(RichText::new(t("Vorverst.")).size(11.0).color(thema::muted()));
                                ui.label(RichText::new(format!("{:+.1}", self.eq.vor)).size(11.0).monospace());
                            });
                            ui.add_space(8.0);
                            let n = self.eq.baender.len().min(self.eq_frequenzen.len());
                            for k in 0..n {
                                let f = self.eq_frequenzen.get(k).copied().unwrap_or(0.0);
                                if let Some(b) = self.eq.baender.get_mut(k) {
                                    ui.vertical(|ui| {
                                        ui.add(egui::Slider::new(b, -20.0..=20.0).vertical().fixed_decimals(1).show_value(false));
                                        ui.label(RichText::new(khz(f)).size(11.0).color(thema::muted()));
                                        ui.label(RichText::new(format!("{:+.1}", *b)).size(11.0).monospace());
                                    });
                                }
                            }
                        });
                        stil::hinweis(ui, t("Werte in dB. Gilt sofort und für alle folgenden Stücke."));
                    });
                });
            } else {
                if !kann_anp {
                    stil::hinweis(ui, t("Dieses libVLC kann das Bild nicht anpassen (oder es ist noch nicht geladen)."));
                }
                ui.add_enabled_ui(kann_anp, |ui| {
                    stil::schalter(ui, &mut self.anpassung.an, t("Bild anpassen"));
                    ui.add_enabled_ui(self.anpassung.an, |ui| {
                        let namen = [t("Helligkeit"), t("Kontrast"), t("Sättigung"), t("Farbton"), t("Gamma")];
                        let bereiche = [0.0..=2.0, 0.0..=2.0, 0.0..=3.0, -180.0..=180.0, 0.01..=10.0];
                        // Reihenfolge der Werte: Helligkeit, Kontrast, Sättigung, Farbton, Gamma
                        for (k, (name, bereich)) in namen.iter().zip(bereiche).enumerate() {
                            if let Some(w) = self.anpassung.werte.get_mut(k) {
                                stil::feld(ui, name, |ui| {
                                    stil::schieber(ui, w, bereich, if k == 3 { "°" } else { "" }, if k == 3 { 0 } else { 2 });
                                });
                            }
                        }
                        if stil::knopf_text(ui, self.anpassung.werte != ANPASSUNG_STD, t("Zurücksetzen")).clicked() {
                            self.anpassung.werte = ANPASSUNG_STD;
                        }
                    });
                });
                ui.add_space(4.0);
                stil::hinweis(ui, t("Seitenverhältnis, Zuschneiden und Deinterlace stehen im Menü Video."));
            }
        });
        if let Some(i) = vorlage_neu {
            if let Some((vor, baender)) = self.player().and_then(|p| p.equalizer_vorlage(i)) {
                self.eq.vor = vor;
                self.eq.baender = baender;
                self.eq.vorlage = Some(i);
                self.eq.an = true;
            }
        } else if self.eq.vor != eq_alt.vor || self.eq.baender != eq_alt.baender {
            // Von Hand geändert: keine Vorlage mehr
            self.eq.vorlage = None;
        }
        if self.eq != eq_alt || self.anpassung != anp_alt {
            self.effekte_anwenden();
        }
        if !offen {
            self.fenster.effekte = false;
        }
    }

    fn info_lesen(&mut self) {
        let mut info = Info::default();
        let e = self.aktuell.and_then(|i| self.liste.get(i));
        if let Some(e) = e {
            info.meta.push((t("Datei"), e.url.clone().unwrap_or_else(|| e.pfad.display().to_string())));
            if e.url.is_none() {
                if let Ok(m) = std::fs::metadata(&e.pfad) {
                    info.meta.push((t("Grösse"), groesse_text(m.len())));
                }
            }
            if e.art == Art::Bild {
                if let Some(b) = self.bilder.iter().find(|b| b.pfad == e.pfad) {
                    info.meta.push((t("Bildgrösse"), format!("{} × {}", b.size[0], b.size[1])));
                }
            }
        }
        if let Some(p) = self.medium_player() {
            for (k, name) in [
                (meta::TITEL, t("Titel")),
                (meta::KUENSTLER, t("Künstler")),
                (meta::ALBUM, t("Album")),
                (meta::ALBUM_KUENSTLER, t("Album-Künstler")),
                (meta::NUMMER, t("Titelnummer")),
                (meta::GENRE, t("Genre")),
                (meta::DATUM, t("Datum")),
                (meta::REGIE, t("Regie")),
                (meta::BESCHREIBUNG, t("Beschreibung")),
                (meta::COPYRIGHT, t("Copyright")),
                (meta::HERAUSGEBER, t("Herausgeber")),
                (meta::KODIERT_VON, t("Kodiert von")),
                (meta::SPRACHE, t("Sprache")),
                (meta::LAEUFT, t("Läuft gerade")),
                (meta::URL, t("Adresse")),
            ] {
                if let Some(v) = p.meta(k) {
                    info.meta.push((name, v));
                }
            }
            if self.laenge > 0 {
                info.meta.push((t("Dauer"), medien::zeit_text(self.laenge)));
            }
            info.spuren = p.spuren_info();
            info.statistik = p.statistik();
        }
        self.info = info;
    }

    fn info_fenster(&mut self, ctx: &egui::Context) {
        if !self.fenster.info {
            return;
        }
        if self.info_seit.elapsed() > Duration::from_secs(1) {
            self.info_seit = Instant::now();
            self.info_lesen();
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        let mut offen = true;
        egui::Window::new(t("Medieninformationen")).open(&mut offen).collapsible(false).default_width(520.0).default_height(420.0).show(ctx, |ui| {
            let mut reiter = self.fenster.info_reiter;
            stil::wahl(ui, &mut reiter, &[(0u8, t("Allgemein")), (1u8, t("Codec")), (2u8, t("Statistik"))]);
            self.fenster.info_reiter = reiter;
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| match reiter {
                0 => {
                    if self.info.meta.is_empty() {
                        stil::hinweis(ui, t("Keine Angaben."));
                    }
                    egui::Grid::new("abspiel-info-allg").num_columns(2).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
                        for (k, v) in &self.info.meta {
                            zeile(ui, k, v);
                        }
                    });
                }
                1 => {
                    if self.info.spuren.is_empty() {
                        stil::hinweis(ui, t("Keine Angaben (erst während der Wiedergabe)."));
                    }
                    for (n, s) in self.info.spuren.iter().enumerate() {
                        let art = match s.art {
                            0 => t("Audio"),
                            1 => t("Video"),
                            2 => t("Untertitel"),
                            _ => t("Unbekannt"),
                        };
                        ui.label(RichText::new(tf("Spur {n}: {art}", &[("n", &(n + 1)), ("art", &art)])).strong());
                        egui::Grid::new(("abspiel-info-spur", n)).num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
                            let codec = if s.codec.is_empty() { s.fourcc.clone() } else { format!("{} ({})", s.codec, s.fourcc.trim()) };
                            zeile(ui, t("Codec"), &codec);
                            if !s.sprache.is_empty() {
                                zeile(ui, t("Sprache"), &s.sprache);
                            }
                            if !s.beschreibung.is_empty() {
                                zeile(ui, t("Beschreibung"), &s.beschreibung);
                            }
                            if s.breite > 0 && s.hoehe > 0 {
                                zeile(ui, t("Auflösung"), &format!("{} × {}", s.breite, s.hoehe));
                            }
                            if s.fps > 0.0 && s.fps.is_finite() {
                                zeile(ui, t("Bildrate"), &format!("{:.3}", s.fps));
                            }
                            if s.kanaele > 0 {
                                zeile(ui, t("Kanäle"), &s.kanaele.to_string());
                            }
                            if s.rate > 0 {
                                zeile(ui, t("Abtastrate"), &format!("{} Hz", s.rate));
                            }
                            if s.bitrate > 0 {
                                zeile(ui, t("Bitrate"), &format!("{} kb/s", s.bitrate / 1000));
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
                _ => match self.info.statistik {
                    None => stil::hinweis(ui, t("Keine Angaben (erst während der Wiedergabe).")),
                    Some(st) => {
                        egui::Grid::new("abspiel-info-stat").num_columns(2).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
                            let kb = |b: i32| format!("{} KiB", (b.max(0) as i64) / 1024);
                            let rate = |r: f32| if r.is_finite() { format!("{:.0} kb/s", (r * 8000.0).max(0.0)) } else { "–".to_string() };
                            zeile(ui, t("Gelesen"), &kb(st.gelesen_bytes));
                            zeile(ui, t("Eingangs-Bitrate"), &rate(st.eingang_bitrate));
                            zeile(ui, t("Demux gelesen"), &kb(st.demux_bytes));
                            zeile(ui, t("Inhalts-Bitrate"), &rate(st.demux_bitrate));
                            zeile(ui, t("Fehlerhaft"), &st.demux_kaputt.max(0).to_string());
                            zeile(ui, t("Unterbrechungen"), &st.demux_unterbrochen.max(0).to_string());
                            zeile(ui, t("Dekodierte Videobilder"), &st.dekodiert_video.max(0).to_string());
                            zeile(ui, t("Angezeigte Bilder"), &st.angezeigt.max(0).to_string());
                            zeile(ui, t("Verlorene Bilder"), &st.verloren.max(0).to_string());
                            zeile(ui, t("Dekodierte Audioblöcke"), &st.dekodiert_audio.max(0).to_string());
                            zeile(ui, t("Gespielte Audiopuffer"), &st.audio_gespielt.max(0).to_string());
                            zeile(ui, t("Verlorene Audiopuffer"), &st.audio_verloren.max(0).to_string());
                        });
                    }
                },
            });
        });
        if !offen {
            self.fenster.info = false;
        }
    }

    fn einstellungen_fenster(&mut self, ctx: &egui::Context) {
        if !self.fenster.einstellungen {
            return;
        }
        let mut offen = true;
        let mut ordner_waehlen = false;
        egui::Window::new(t("Einstellungen")).open(&mut offen).collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            stil::feld(ui, t("Wiedergabe fortsetzen, wo sie aufgehört hat"), |ui| {
                stil::wahl(ui, &mut self.fortsetzen, &[(Fortsetzen::Fragen, t("Fragen")), (Fortsetzen::Immer, t("Immer")), (Fortsetzen::Nie, t("Nie"))]);
            });
            stil::hinweis(ui, t("Für Videos ab 1 Minute und Musik ab 20 Minuten."));
            stil::schalter(ui, &mut self.rest_zeit, t("Verbleibende statt gesamte Zeit anzeigen"));
            stil::feld(ui, t("Diashow: Sekunden pro Bild"), |ui| {
                stil::wahl(ui, &mut self.diashow_s, &[(2.0, "2"), (3.0, "3"), (5.0, "5"), (10.0, "10"), (20.0, "20")]);
            });
            stil::feld(ui, t("Bildschirmfotos speichern in"), |ui| {
                let ordner = self.snap_ordner.clone().or_else(standard_bilder_ordner).map(|p| p.display().to_string()).unwrap_or_default();
                ui.add(egui::Label::new(RichText::new(ordner).monospace().size(12.0)).truncate());
                ui.horizontal(|ui| {
                    if stil::knopf(ui, Sym::OrdnerAuf, t("Ordner wählen …")).clicked() {
                        ordner_waehlen = true;
                    }
                    if stil::knopf_text(ui, self.snap_ordner.is_some(), t("Standard")).clicked() {
                        self.snap_ordner = None;
                    }
                });
            });
            if stil::knopf_text(ui, self.positionen.len() > 0, t("Gemerkte Positionen vergessen")).clicked() {
                self.positionen = Positionen::default();
            }
        });
        if ordner_waehlen {
            if let Some(p) = rfd::FileDialog::new().set_title(t("Bildschirmfotos speichern in")).pick_folder() {
                self.snap_ordner = Some(p);
            }
        }
        if !offen {
            self.fenster.einstellungen = false;
        }
    }

    fn tasten_fenster(&mut self, ctx: &egui::Context) {
        if !self.fenster.tasten {
            return;
        }
        let mut offen = true;
        egui::Window::new(t("Tastenkürzel")).open(&mut offen).collapsible(false).default_width(520.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                egui::Grid::new("abspiel-tasten").num_columns(2).spacing([18.0, 5.0]).striped(true).show(ui, |ui| {
                    for (k, v) in tastenliste() {
                        ui.label(RichText::new(kurz_text(k)).monospace().strong());
                        ui.label(v);
                        ui.end_row();
                    }
                });
            });
            ui.add_space(4.0);
            stil::hinweis(ui, t("Wie in VLC. Ein Doppelklick ins Bild schaltet Vollbild, ein Rechtsklick zeigt alle Menüs."));
        });
        if !offen {
            self.fenster.tasten = false;
        }
    }

    fn ueber_fenster(&mut self, ctx: &egui::Context) {
        if !self.fenster.ueber {
            return;
        }
        let mut offen = true;
        let vlc = match &self.engine {
            Engine::Bereit(p) => format!("libVLC {}", p.version()),
            Engine::Laedt(_) => t("libVLC wird geladen …").to_string(),
            Engine::Fehlt(_) => vlc_fehlt_text().to_string(),
        };
        egui::Window::new(t("Über Abspielwerk")).open(&mut offen).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                stil::titel(ui, "Abspielwerk");
                stil::grau(ui, env!("CARGO_PKG_VERSION"));
            });
            ui.label(t("Videos, Musik und Bilder ansehen – Teil der Werkbank."));
            ui.add_space(4.0);
            ui.label(RichText::new(vlc).color(thema::muted()));
            stil::hinweis(ui, t("Videos und Musik spielt libVLC von VideoLAN (LGPL 2.1 oder später). Abspielwerk ist kein Produkt von VideoLAN."));
        });
        if !offen {
            self.fenster.ueber = false;
        }
    }
}

fn groesse_text(b: u64) -> String {
    let b = b as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GiB", b / 1024.0 / 1024.0 / 1024.0)
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MiB", b / 1024.0 / 1024.0)
    } else {
        format!("{:.0} KiB", b / 1024.0)
    }
}
