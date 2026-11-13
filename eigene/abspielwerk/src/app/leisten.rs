//! Bedienleiste unten (wie VLC), Wiedergabeliste rechts und die Anzeige in der Mitte.

use super::*;

use egui::{Align, RichText, Sense, Stroke, Ui};

use crate::stil::Sym;

/// Text auf eine Breite gekürzt (mit «…»).
fn gekuerzt(ui: &Ui, text: &str, gr: f32, farbe: Color32, breite: f32) -> std::sync::Arc<egui::Galley> {
    let font = egui::FontId::proportional(gr);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, farbe);
    job.wrap = egui::text::TextWrapping::truncate_at_width(breite.max(8.0));
    ui.fonts(|f| f.layout_job(job))
}

/// Schaltknopf mit Symbol (z. B. Wiederholen, Zufall): getönt, wenn an.
fn umschalter(ui: &mut Ui, an: bool, s: Sym, hinweis: &str, zusatz: Option<&str>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(32.0, 30.0), Sense::click());
    let p = ui.painter();
    if an {
        p.rect_filled(rect, 8.0, stil::toenung());
    } else if resp.hovered() {
        p.rect_filled(rect, 8.0, thema::hover());
    }
    let c = if an { stil::akzent() } else { thema::muted() };
    stil::sym(p, Rect::from_center_size(rect.center(), Vec2::splat(17.0)), s, c);
    if let Some(z) = zusatz {
        let g = p.layout_no_wrap(z.to_string(), egui::FontId::proportional(9.5), c);
        p.galley(Pos2::new(rect.right() - g.size().x - 3.0, rect.bottom() - g.size().y - 1.0), g, c);
    }
    resp.on_hover_text(hinweis).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Lautstärkeregler wie in VLC (0–125 %, Marke bei 100 %); Mausrad darüber ändert ebenfalls.
fn lautstaerke_regler(ui: &mut Ui, wert: &mut f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(112.0, 24.0), Sense::click_and_drag());
    let bahn = Rect::from_center_size(rect.center(), egui::vec2(rect.width() - 8.0, 6.0));
    let frac = (*wert / MAX_LAUTSTAERKE).clamp(0.0, 1.0);
    let p = ui.painter();
    p.rect_filled(bahn, 3.0, thema::border());
    let x = bahn.left() + bahn.width() * frac;
    let hundert = bahn.left() + bahn.width() * (100.0 / MAX_LAUTSTAERKE);
    p.rect_filled(Rect::from_min_max(bahn.min, Pos2::new(x.min(hundert), bahn.max.y)), 3.0, stil::farbe());
    if x > hundert {
        p.rect_filled(Rect::from_min_max(Pos2::new(hundert, bahn.min.y), Pos2::new(x, bahn.max.y)), 3.0, stil::orange());
    }
    p.vline(hundert, Rect::from_center_size(bahn.center(), egui::vec2(1.0, 14.0)).y_range(), Stroke::new(1.0, thema::muted()));
    let aktiv = resp.hovered() || resp.dragged();
    p.circle(Pos2::new(x, bahn.center().y), if aktiv { 6.5 } else { 5.5 }, stil::farbe(), Stroke::new(2.0, thema::panel()));
    let mut neu = *wert;
    if resp.dragged() || resp.clicked() {
        if let Some(pp) = resp.interact_pointer_pos() {
            neu = (((pp.x - bahn.left()) / bahn.width().max(1.0)).clamp(0.0, 1.0) * MAX_LAUTSTAERKE).round();
        }
    }
    if resp.hovered() {
        let d = ui.input(|i| i.raw_scroll_delta.y);
        if d.is_finite() && d.abs() > 0.5 {
            neu = (neu + d.signum() * LAUT_SCHRITT).clamp(0.0, MAX_LAUTSTAERKE).round();
        }
    }
    let _ = resp.on_hover_text(t("Lautstärke (Mausrad, ↑ / ↓)"));
    if (neu - *wert).abs() > 0.01 {
        *wert = neu;
        true
    } else {
        false
    }
}

impl App {
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

    /// Bedienleiste wie in VLC: oben Zeit – Suchleiste – Gesamtzeit (Klick: verbleibend),
    /// darunter Abspielen, Vorheriges/Stopp/Nächstes, Vollbild/Erweitert/Liste,
    /// Wiederholen/Zufall und rechts die Lautstärke.
    pub(super) fn fuss(&mut self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        if self.aktuelle_art() == Some(Art::Bild) {
            self.fuss_bild(ui, a);
            return;
        }
        let medium = self.medium_player().is_some();
        let hat_liste = !self.liste.is_empty();
        // Zeile 1: Zeit, Suchleiste, Gesamtzeit
        ui.horizontal(|ui| {
            let pos = self.scrub.unwrap_or(self.zeit);
            ui.label(RichText::new(medien::zeit_text(pos)).monospace().size(13.0).color(thema::muted()));
            let rechts = if self.rest_zeit && self.laenge > 0 { format!("-{}", medien::zeit_text((self.laenge - pos).max(0))) } else { medien::zeit_text(self.laenge) };
            let w = ui.available_width() - 76.0;
            if let Some(z) = self.suchleiste(ui, w) {
                a.push(Aktion::Springen(z));
            }
            let r = ui.add(egui::Label::new(RichText::new(rechts).monospace().size(13.0).color(thema::muted())).sense(Sense::click()).selectable(false));
            if r.on_hover_text(t("Klick: Gesamtzeit oder verbleibende Zeit")).clicked() {
                self.rest_zeit = !self.rest_zeit;
            }
        });
        ui.add_space(2.0);
        // Zeile 2: Knöpfe
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let spielt = medium && matches!(self.zustand, Zustand::Spielt | Zustand::Oeffnet | Zustand::Puffert);
            let h = if spielt { t("Pause (Leertaste)") } else { t("Abspielen (Leertaste)") };
            if stil::haupt(ui, hat_liste, if spielt { Sym::Pause } else { Sym::Play }, "").on_hover_text(h).clicked() {
                a.push(Aktion::PlayPause);
            }
            ui.add_space(6.0);
            if stil::leise(ui, hat_liste, Sym::Anfang, "", t("Vorheriges (P)")).clicked() {
                a.push(Aktion::Zurueck);
            }
            if stil::leise(ui, medium, Sym::Stopp, "", t("Stopp (S)")).clicked() {
                a.push(Aktion::Stopp);
            }
            if stil::leise(ui, hat_liste, Sym::Ende, "", t("Nächstes (N)")).clicked() {
                a.push(Aktion::Weiter);
            }
            stil::trenner(ui);
            let hv = if self.vollbild { t("Vollbild beenden (Esc)") } else { t("Vollbild (F)") };
            if stil::leise(ui, true, Sym::Panorama, "", hv).clicked() {
                a.push(Aktion::Vollbild);
            }
            if umschalter(ui, self.fenster.effekte, Sym::Diagramm, t("Erweiterte Einstellungen: Effekte und Filter (Strg+E)"), None).clicked() {
                a.push(Aktion::Effekte);
            }
            if !self.vollbild && umschalter(ui, self.liste_sichtbar, Sym::Ebenen, t("Wiedergabeliste (Strg+L)"), None).clicked() {
                a.push(Aktion::ListeUmschalten);
            }
            stil::trenner(ui);
            let (wdh_an, zusatz) = match self.wdh {
                Wdh::Aus => (false, None),
                Wdh::Alle => (true, None),
                Wdh::Eins => (true, Some("1")),
            };
            let hw = format!("{} (L)", wdh_text(self.wdh));
            if umschalter(ui, wdh_an, Sym::Aktualisieren, &hw, zusatz).clicked() {
                a.push(Aktion::WdhWechseln);
            }
            if umschalter(ui, self.zufall, Sym::Mischen, t("Zufällige Reihenfolge (R)"), None).clicked() {
                a.push(Aktion::ZufallUmschalten);
            }
            stil::rechts(ui, |ui| {
                let v = self.lautstaerke;
                ui.label(RichText::new(format!("{v:.0} %")).monospace().size(12.5).color(if v > 100.0 { stil::orange() } else { thema::muted() }));
                let mut l = self.lautstaerke;
                if lautstaerke_regler(ui, &mut l) {
                    self.lautstaerke = l;
                    if l > 0.0 {
                        self.stumm = false;
                    }
                }
                let hs = if self.stumm { t("Ton einschalten (M)") } else { t("Stumm (M)") };
                if stil::leise(ui, true, if self.stumm { Sym::Kreuz } else { Sym::Musik }, "", hs).clicked() {
                    a.push(Aktion::Stumm);
                }
                if (self.tempo - 1.0).abs() > 1e-3 {
                    let r = ui.add(egui::Label::new(RichText::new(befehl::tempo_text(self.tempo)).size(12.5).color(stil::akzent())).sense(Sense::click()).selectable(false));
                    if r.on_hover_text(t("Normale Geschwindigkeit (=)")).clicked() {
                        a.push(Aktion::TempoNormal);
                    }
                }
            });
        });
    }

    fn fuss_bild(&mut self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let h = if self.diashow { t("Diashow anhalten (Leertaste)") } else { t("Diashow starten (Leertaste)") };
            if stil::haupt(ui, true, if self.diashow { Sym::Pause } else { Sym::Play }, "").on_hover_text(h).clicked() {
                a.push(Aktion::Diashow);
            }
            ui.add_space(6.0);
            if stil::leise(ui, true, Sym::Anfang, "", t("Vorheriges (←)")).clicked() {
                a.push(Aktion::Zurueck);
            }
            if stil::leise(ui, true, Sym::Ende, "", t("Nächstes (→)")).clicked() {
                a.push(Aktion::Weiter);
            }
            stil::trenner(ui);
            if stil::leise(ui, true, Sym::Rueckgaengig, "", t("Nach links drehen")).clicked() {
                a.push(Aktion::Drehen(3));
            }
            if stil::leise(ui, true, Sym::Aktualisieren, "", t("Nach rechts drehen")).clicked() {
                a.push(Aktion::Drehen(1));
            }
            if stil::knopf_text(ui, self.zoom != 1.0 || self.pan != Vec2::ZERO, t("Einpassen")).clicked() {
                a.push(Aktion::Einpassen);
            }
            stil::grau(ui, &format!("{:.0} %", self.zoom_prozent()));
            stil::trenner(ui);
            let hv = if self.vollbild { t("Vollbild beenden (Esc)") } else { t("Vollbild (F)") };
            if stil::leise(ui, true, Sym::Panorama, "", hv).clicked() {
                a.push(Aktion::Vollbild);
            }
            if !self.vollbild && umschalter(ui, self.liste_sichtbar, Sym::Ebenen, t("Wiedergabeliste (Strg+L)"), None).clicked() {
                a.push(Aktion::ListeUmschalten);
            }
            if self.diashow {
                stil::grau(ui, &tf("Diashow: {s} s pro Bild", &[("s", &self.diashow_s)]));
            }
            stil::rechts(ui, |ui| {
                if stil::leise(ui, true, Sym::Papierkorb, "", t("In Papierkorb … (Entf)")).clicked() {
                    a.push(Aktion::Papierkorb);
                }
                stil::trenner(ui);
                if let Some(e) = self.aktuell.and_then(|i| self.liste.get(i)) {
                    let groesse = self.bilder.iter().find(|b| b.pfad == e.pfad).map(|b| format!("  ·  {} × {}", b.size[0], b.size[1])).unwrap_or_default();
                    ui.add(egui::Label::new(RichText::new(format!("{}{}", e.name, groesse)).size(13.0).color(thema::muted())).truncate().selectable(false));
                }
            });
        });
    }

    /// Zoom in Prozent der Originalgrösse (für die Anzeige), ungefähr aus dem letzten Bild.
    fn zoom_prozent(&self) -> f32 {
        (self.basis * self.zoom * 100.0).clamp(0.0, 100_000.0)
    }

    /// Bedienleiste im Vollbild: schwebt unten über dem Bild (erscheint bei Mausbewegung).
    pub(super) fn vollbild_bedienung(&mut self, ctx: &egui::Context, a: &mut Vec<Aktion>) {
        let breite = (ctx.screen_rect().width() - 60.0).clamp(300.0, 1100.0);
        let r = egui::Area::new(egui::Id::new("abspiel-vollbild-bedienung"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -18.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(thema::panel())
                    .rounding(12.0)
                    .stroke(Stroke::new(1.0, thema::border()))
                    .inner_margin(egui::Margin::symmetric(14.0, 9.0))
                    .show(ui, |ui| {
                        ui.set_width(breite);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        self.fuss(ui, a);
                    });
            });
        self.overlay = Some(r.response.rect);
    }

    // ------------------------------------------------------------------ Wiedergabeliste

    pub(super) fn liste_panel(&mut self, ctx: &egui::Context, a: &mut Vec<Aktion>) {
        let max = (ctx.screen_rect().width() * 0.5).max(300.0);
        egui::SidePanel::right("abspiel-liste")
            .resizable(true)
            .default_width(380.0)
            .min_width(240.0)
            .max_width(max)
            .frame(egui::Frame::none().fill(thema::bg()).inner_margin(egui::Margin::same(10.0)).stroke(Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| self.seite(ui, a));
    }

    fn seite(&mut self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        ui.horizontal(|ui| {
            stil::symbol(ui, Sym::Ebenen, 18.0, stil::akzent());
            ui.label(RichText::new(t("Wiedergabeliste")).size(14.5).strong());
            let n = self.liste.len();
            let summe: i64 = self.liste.iter().filter_map(|e| e.dauer).sum();
            stil::rechts(ui, |ui| {
                let txt = if summe > 0 { format!("{n}  ·  {}", medien::zeit_text(summe)) } else { n.to_string() };
                stil::grau(ui, &txt);
            });
        });
        if self.liste.is_empty() {
            stil::hinweis(ui, t("Dateien hineinziehen oder einen Ordner öffnen."));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if stil::knopf(ui, Sym::Datei, t("Datei öffnen …")).clicked() {
                    a.push(Aktion::Oeffnen);
                }
                if stil::knopf(ui, Sym::OrdnerAuf, t("Ordner öffnen …")).clicked() {
                    a.push(Aktion::OrdnerOeffnen);
                }
            });
            return;
        }
        ui.add_space(2.0);
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(t("Suchen (Titel, Album) …")).desired_width(f32::INFINITY));
        ui.add_space(2.0);

        let breit = ui.available_width();
        let mit_album = breit >= 320.0;
        let dauer_w = 52.0;
        let album_w = if mit_album { (breit * 0.3).clamp(70.0, 220.0) } else { 0.0 };
        let titel_x = 26.0;
        let titel_w = (breit - titel_x - dauer_w - album_w - 14.0).max(30.0);
        // Spaltenköpfe
        {
            let (r, _) = ui.allocate_exact_size(egui::vec2(breit, 18.0), Sense::hover());
            let p = ui.painter();
            let f = egui::FontId::proportional(11.5);
            p.text(Pos2::new(r.left() + titel_x, r.center().y), egui::Align2::LEFT_CENTER, t("Titel"), f.clone(), thema::muted());
            p.text(Pos2::new(r.left() + titel_x + titel_w + dauer_w, r.center().y), egui::Align2::RIGHT_CENTER, t("Dauer"), f.clone(), thema::muted());
            if mit_album {
                p.text(Pos2::new(r.right() - album_w, r.center().y), egui::Align2::LEFT_CENTER, t("Album"), f, thema::muted());
            }
            p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, thema::border()));
        }

        let filter = self.filter.trim().to_lowercase();
        let sicht: Vec<usize> = if filter.is_empty() {
            (0..self.liste.len()).collect()
        } else {
            self.liste.iter().enumerate().filter(|(_, e)| e.anzeige().to_lowercase().contains(&filter) || e.album.to_lowercase().contains(&filter) || e.name.to_lowercase().contains(&filter)).map(|(i, _)| i).collect()
        };
        if sicht.is_empty() {
            stil::hinweis(ui, t("Nichts gefunden."));
            return;
        }
        let zeile_h = 26.0;
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if self.scroll_zu {
            if let Some(pos) = self.aktuell.and_then(|a| sicht.iter().position(|i| *i == a)) {
                let hoehe = ui.available_height();
                let off = (pos as f32 * (zeile_h + 2.0) - hoehe / 2.0 + zeile_h).max(0.0);
                sa = sa.vertical_scroll_offset(off);
            }
            self.scroll_zu = false;
        }
        let darf_ziehen = filter.is_empty();
        let mut ziel_vor: Option<usize> = None;
        let zeiger = ui.input(|i| i.pointer.interact_pos());
        sa.show_rows(ui, zeile_h, sicht.len(), |ui, bereich| {
            for k in bereich {
                let Some(&i) = sicht.get(k) else { continue };
                let Some(e) = self.liste.get(i) else { continue };
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(breit, zeile_h), Sense::click_and_drag());
                let an = self.aktuell == Some(i);
                let gewaehlt = self.auswahl == Some(i);
                if ui.is_rect_visible(rect) {
                    let p = ui.painter();
                    if an {
                        p.rect_filled(rect, 6.0, stil::toenung());
                    } else if gewaehlt {
                        p.rect_filled(rect, 6.0, thema::hover());
                    } else if resp.hovered() {
                        p.rect_filled(rect, 6.0, stil::mix(thema::hover(), thema::bg(), 0.6));
                    }
                    if gewaehlt && self.liste_fokus {
                        p.rect_stroke(rect.shrink(0.5), 6.0, Stroke::new(1.0, stil::mix(stil::farbe(), thema::border(), 0.5)));
                    }
                    let s = match e.art {
                        Art::Video => Sym::Film,
                        Art::Musik => Sym::Musik,
                        Art::Bild => Sym::Bild,
                    };
                    let sym_c = if an { stil::akzent() } else { thema::muted() };
                    let s = if an && matches!(self.zustand, Zustand::Spielt) { Sym::Play } else { s };
                    stil::sym(p, Rect::from_center_size(Pos2::new(rect.left() + 13.0, rect.center().y), Vec2::splat(14.0)), s, sym_c);
                    let farbe = if an { thema::text() } else { stil::mix(thema::text(), thema::muted(), 0.35) };
                    let g = gekuerzt(ui, e.anzeige(), 13.0, farbe, titel_w);
                    p.galley(Pos2::new(rect.left() + titel_x, rect.center().y - g.size().y / 2.0), g, farbe);
                    let dauer = match (e.art, e.dauer) {
                        (Art::Bild, _) => String::new(),
                        (_, Some(d)) => medien::zeit_text(d),
                        (_, None) => "–".to_string(),
                    };
                    p.text(Pos2::new(rect.left() + titel_x + titel_w + dauer_w, rect.center().y), egui::Align2::RIGHT_CENTER, dauer, egui::FontId::monospace(12.0), thema::muted());
                    if mit_album && !e.album.is_empty() {
                        let g = gekuerzt(ui, &e.album, 12.0, thema::muted(), album_w - 6.0);
                        p.galley(Pos2::new(rect.right() - album_w, rect.center().y - g.size().y / 2.0), g, thema::muted());
                    }
                }
                // Ziehen zum Umordnen: Einfügemarke zeigen
                if self.ziehen.is_some() {
                    if let Some(z) = zeiger.filter(|z| z.y >= rect.top() - 1.0 && z.y < rect.bottom() + 1.0) {
                        let vor = if z.y < rect.center().y { i } else { i + 1 };
                        ziel_vor = Some(vor);
                        let y = if vor == i { rect.top() } else { rect.bottom() };
                        ui.painter().hline(rect.x_range(), y, Stroke::new(2.0, stil::farbe()));
                    }
                }
                if resp.drag_started() && darf_ziehen {
                    self.ziehen = Some(i);
                }
                if resp.clicked() {
                    self.auswahl = Some(i);
                    self.liste_fokus = true;
                }
                if resp.double_clicked() {
                    a.push(Aktion::Zeigen(i));
                }
                let pfad = e.url.clone().unwrap_or_else(|| e.pfad.display().to_string());
                let resp = if resp.hovered() && self.ziehen.is_none() { resp.on_hover_text(pfad) } else { resp };
                resp.context_menu(|ui| {
                    if stil::eintrag(ui, true, Sym::Play, t("Abspielen")).clicked() {
                        a.push(Aktion::Zeigen(i));
                        ui.close_menu();
                    }
                    if stil::eintrag(ui, true, Sym::Kreuz, t("Aus der Liste nehmen")).clicked() {
                        a.push(Aktion::Entfernen(i));
                        ui.close_menu();
                    }
                    ui.separator();
                    if stil::eintrag(ui, true, Sym::Papierkorb, t("Liste leeren")).clicked() {
                        a.push(Aktion::Leeren);
                        ui.close_menu();
                    }
                });
            }
        });
        if ziel_vor.is_some() {
            self.ziehen_vor = ziel_vor;
        }
        if let Some(von) = self.ziehen {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if !ui.input(|i| i.pointer.any_down()) {
                if let Some(vor) = self.ziehen_vor.take() {
                    a.push(Aktion::Verschieben(von, vor));
                }
                self.ziehen = None;
            }
        }
    }

    // ------------------------------------------------------------------ Anzeige

    pub(super) fn anzeige(&mut self, ui: &mut Ui, a: &mut Vec<Aktion>) {
        let Some((pfad, art, name)) = self.aktuell.and_then(|i| self.liste.get(i)).map(|e| (e.pfad.clone(), e.art, e.anzeige().to_string())) else {
            stil::leer(ui, Sym::Film, t("Videos, Musik und Bilder"), t("Dateien hineinziehen oder über Medien › Datei öffnen …"));
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
            return;
        };
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        if resp.double_clicked() {
            a.push(Aktion::Vollbild);
        }
        if resp.clicked() {
            self.liste_fokus = false;
        }
        let rund = if self.vollbild { 0.0 } else { 10.0 };
        // Mausrad über Video und Musik = Lautstärke (wie VLC); bei Bildern zoomt es.
        if art != Art::Bild && resp.hovered() {
            let d = ui.input(|i| i.raw_scroll_delta.y);
            if d.is_finite() && d.abs() > 0.0 {
                self.laut_rest += d;
                // 50 Punkte ≈ eine Raste
                let schritte = (self.laut_rest / 50.0).trunc();
                if schritte != 0.0 {
                    self.laut_rest -= schritte * 50.0;
                    self.lautstaerke_aendern(schritte.clamp(-5.0, 5.0) * LAUT_SCHRITT);
                }
            }
        }
        match art {
            Art::Video => {
                ui.painter().rect_filled(rect, rund, Color32::BLACK);
                match &self.video_tex {
                    Some(tex) => {
                        let sar = self.sar.map(|(n, d)| n as f64 / d.max(1) as f64).unwrap_or(1.0);
                        let seiten = befehl::SEITEN.get(self.seiten).and_then(|s| befehl::verhaeltnis(s));
                        let zuschnitt = befehl::ZUSCHNITTE.get(self.zuschnitt).and_then(|s| befehl::verhaeltnis(s));
                        let (dar, uv) = befehl::geometrie(self.video_size[0], self.video_size[1], sar, seiten, zuschnitt);
                        let r = einpassen_verhaeltnis(rect, dar);
                        ui.painter().image(tex.id(), r, Rect::from_min_max(Pos2::new(uv[0], uv[1]), Pos2::new(uv[2], uv[3])), Color32::WHITE);
                    }
                    None => {
                        let txt = match &self.engine {
                            Engine::Fehlt(_) => vlc_fehlt_text().to_string(),
                            _ if matches!(self.zustand, Zustand::Fehler) => tf("{name} lässt sich nicht abspielen.", &[("name", &name)]),
                            _ if matches!(self.zustand, Zustand::Gestoppt | Zustand::Nichts) => String::new(),
                            _ if matches!(self.zustand, Zustand::Spielt) => String::new(),
                            _ => t("Wird geöffnet …").to_string(),
                        };
                        mittig_text(ui, rect, &txt, 15.0, Color32::from_gray(200));
                        if matches!(self.zustand, Zustand::Gestoppt) {
                            stil::sym(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(64.0)), Sym::Film, Color32::from_gray(90));
                        }
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
        // Rechtsklick ins Bild: alle Menüs (wie in VLC; auch in der minimalen Ansicht).
        resp.context_menu(|ui| self.kontextmenue(ui, a));
        self.fortsetzen_malen(ui, rect, a);
        self.osd_malen(ui, rect);
        self.meldung_malen(ui, rect);
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
            let g = gekuerzt(ui, &text, gr, farbe, breite);
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

    /// Leiste «Fortsetzen ab 12:34?» oben im Bild (wie VLC).
    fn fortsetzen_malen(&mut self, ui: &mut Ui, rect: Rect, a: &mut Vec<Aktion>) {
        let Some((ms, seit)) = self.frage_fortsetzen else { return };
        let alter = seit.elapsed();
        if alter > FRAGE_DAUER || self.medium_player().is_none() {
            self.frage_fortsetzen = None;
            return;
        }
        ui.ctx().request_repaint_after(FRAGE_DAUER - alter);
        let w = (rect.width() - 24.0).clamp(200.0, 560.0);
        let r = Rect::from_min_size(Pos2::new(rect.center().x - w / 2.0, rect.top() + 12.0), egui::vec2(w, 40.0));
        ui.painter().rect(r, 10.0, thema::panel(), Stroke::new(1.0, stil::mix(stil::farbe(), thema::border(), 0.5)));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(egui::vec2(12.0, 5.0))).layout(egui::Layout::left_to_right(Align::Center)));
        let zeit = medien::zeit_text(ms);
        c.label(RichText::new(tf("Fortsetzen ab {t}?", &[("t", &zeit)])).size(13.5).color(thema::text()));
        let mut zu = false;
        stil::rechts(&mut c, |ui| {
            if stil::leise(ui, true, Sym::Kreuz, "", t("Von vorne")).clicked() {
                zu = true;
            }
            if stil::knopf(ui, Sym::Play, t("Fortsetzen")).clicked() {
                a.push(Aktion::Springen(ms));
                zu = true;
            }
        });
        if zu {
            self.frage_fortsetzen = None;
        }
    }

    fn osd_malen(&mut self, ui: &mut Ui, rect: Rect) {
        let Some((text, seit)) = &self.osd else { return };
        let dauer = Duration::from_millis(1800);
        let alter = seit.elapsed();
        if alter > dauer {
            self.osd = None;
            return;
        }
        ui.ctx().request_repaint_after(dauer - alter);
        let font = egui::FontId::proportional((rect.height() / 22.0).clamp(15.0, 30.0));
        let g = ui.painter().layout_no_wrap(text.clone(), font, Color32::WHITE);
        let pos = Pos2::new(rect.right() - g.size().x - 22.0, rect.top() + 18.0);
        let hinter = Rect::from_min_size(pos - egui::vec2(10.0, 6.0), g.size() + egui::vec2(20.0, 12.0));
        ui.painter().rect_filled(hinter, 8.0, Color32::from_black_alpha(150));
        ui.painter().galley(pos, g, Color32::WHITE);
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
        let oben = if self.frage_fortsetzen.is_some() { 60.0 } else { 20.0 };
        let box_r = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + oben + g.size().y / 2.0 + 10.0), g.size() + egui::vec2(28.0, 20.0));
        ui.painter().rect(box_r, 10.0, thema::panel(), Stroke::new(1.0, stil::mix(stil::orange(), thema::border(), 0.6)));
        ui.painter().galley(box_r.min + egui::vec2(14.0, 10.0), g, thema::text());
    }
}
