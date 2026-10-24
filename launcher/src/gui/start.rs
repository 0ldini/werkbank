//! Startseite: oben die schmale Widget-Leiste (ausklappbar zum Raster), darunter der
//! Schnellzugriff mit grossen Programm-Symbolen und der Kachel «Alle Programme».
//! Knöpfe zeigen farbige Symbole statt Wörter; der Name steht im Hinweis.

use super::*;

/// Vorgabe für den Schnellzugriff (nur installierte erscheinen).
pub(super) const QUICK_DEFAULT: [&str; 10] =
    ["photocraft", "wordcraft", "gridcraft", "diawerk", "filmcraft", "mailwerk", "lightcraft", "wandelwerk", "pdfcraft", "codewerk"];

/// Farbe des Symbols je Widget.
pub(super) fn widget_color(kind: &str) -> Color32 {
    rgb(match kind {
        "uhr" => 0x2F6FDE,
        "wetter" => 0xE39B0B,
        "mail" => 0x2F6FDE,
        "termine" => 0xD9622B,
        "nachrichten" => 0xC8102E,
        "timer" => 0x7B4FD6,
        "computer" => 0x0F8B8D,
        "notizen" => 0xC28A00,
        "ordner" => 0xB7791F,
        "zuletzt" => 0x2E9E5B,
        _ => 0x5B616E,
    })
}

pub(super) fn widget_glyph(kind: &str) -> Glyph {
    match kind {
        "uhr" => Glyph::Clock,
        "wetter" => Glyph::Cloud,
        "mail" => Glyph::Mail,
        "termine" => Glyph::Calendar,
        "nachrichten" => Glyph::News,
        "timer" => Glyph::Timer,
        "computer" => Glyph::Cpu,
        "notizen" => Glyph::Note,
        "ordner" => Glyph::Folder,
        "zuletzt" => Glyph::History,
        _ => Glyph::Layout,
    }
}

/// Kräftigere Farbe im dunklen Design, damit Symbole auf dunklem Grund leuchten.
pub(super) fn vivid(c: Color32) -> Color32 {
    if dark() {
        mix(c, Color32::WHITE, 0.72)
    } else {
        c
    }
}

/// Farbiges Symbol-Knöpfchen: getönter Grund, Symbol in der Farbe, Name als Hinweis.
pub(super) fn color_button(ui: &mut egui::Ui, g: Glyph, hint: &str, color: Color32, size: f32, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let c = vivid(color);
    let a = if on || resp.hovered() {
        0.26
    } else if dark() {
        0.16
    } else {
        0.11
    };
    let stroke = if on { Stroke::new(1.5, c) } else { Stroke::NONE };
    ui.painter().rect(rect, size * 0.3, mix(c, panel(), a), stroke, egui::StrokeKind::Inside);
    glyph(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(size * 0.54)), g, c);
    resp.on_hover_text(hint)
}

/// Runder gelber Hauptknopf mit Symbol.
pub(super) fn accent_round(ui: &mut egui::Ui, g: Glyph, hint: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::click());
    let fill = if resp.hovered() { mix(ACCENT, INK, 0.9) } else { ACCENT };
    ui.painter().circle_filled(rect.center(), 21.0, fill);
    glyph(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(22.0)), g, INK);
    resp.on_hover_text(hint)
}

/// Kopfzeile eines Widgets: farbiges Symbol, optional Text, rechts ein Symbol-Knopf.
pub(super) fn icon_row(ui: &mut egui::Ui, g: Glyph, color: Color32, label: Option<&str>, action: Option<(Glyph, &str)>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
        glyph(ui.painter(), r, g, vivid(color));
        if let Some(l) = label {
            ui.label(egui::RichText::new(l).size(14.0).color(muted()));
        }
        if let Some((ag, hint)) = action {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked = color_button(ui, ag, hint, color, 28.0, false).clicked();
            });
        }
    });
    clicked
}

/// Ein Eintrag der eingeklappten Leiste: Symbol, Wert, grauer Zusatz, Trennstrich.
fn chip_item(ui: &mut egui::Ui, icon: &dyn Fn(&egui::Painter, Rect), value: &str, rest: &str, max_w: f32, sep: bool, hint: &str) -> bool {
    let fmt = |size: f32, c: Color32| egui::TextFormat { font_id: egui::FontId::proportional(size), color: c, ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    if !value.is_empty() {
        job.append(value, 0.0, fmt(14.5, text()));
    }
    if !rest.is_empty() {
        job.append(rest, if value.is_empty() { 0.0 } else { 6.0 }, fmt(13.5, muted()));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width((max_w - 54.0).max(20.0));
    let galley = ui.painter().layout_job(job);
    let w = if value.is_empty() && rest.is_empty() { 46.0 } else { (galley.size().x + 54.0).min(max_w).max(46.0) };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 44.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink(3.0), 10.0, hover());
    }
    icon(ui.painter(), Rect::from_center_size(Pos2::new(rect.left() + 23.0, rect.center().y), Vec2::splat(18.0)));
    ui.painter().galley(Pos2::new(rect.left() + 40.0, rect.center().y - galley.size().y / 2.0), galley, text());
    if sep {
        ui.painter().line_segment([Pos2::new(rect.right(), rect.top() + 11.0), Pos2::new(rect.right(), rect.bottom() - 11.0)], Stroke::new(1.0, border()));
    }
    resp.on_hover_text(hint).clicked()
}

/// Wahlknopf (aktiv gelb); liefert true, wenn er angeklickt wurde und noch nicht aktiv war.
fn pill(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let b = egui::Button::new(egui::RichText::new(label).size(13.0).color(if active { INK } else { soft() }))
        .fill(if active { ACCENT } else { panel() })
        .stroke(Stroke::new(1.0, if active { ACCENT } else { border() }))
        .corner_radius(14u8)
        .min_size(egui::vec2(0.0, 30.0));
    ui.add(b).clicked() && !active
}

/// Was ein Klick im Schnellzugriff auslöst.
enum QuickAct {
    Remove(String),
    Move(String, bool),
}

impl Gui {
    // ------------------------------------------------------------------ Widget-Leiste

    pub(super) fn widget_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        let open = self.cfg.home.bar_open || self.arrange;
        let mut toggle = false;
        let mut arrange = false;
        if open {
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 40.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if !self.arrange && color_button(ui, Glyph::Up, t("Widgets einklappen"), rgb(0xC98A12), 36.0, true).clicked() {
                    toggle = true;
                }
                if color_button(ui, if self.arrange { Glyph::Check } else { Glyph::Layout }, if self.arrange { t("Fertig") } else { t("Widgets anordnen") }, rgb(0x0F8B8D), 36.0, self.arrange).clicked() {
                    arrange = true;
                }
                if self.arrange {
                    ui.label(egui::RichText::new(t("Pfeile verschieben, Zahlen ändern die Grösse – Lücken füllen sich selbst.")).size(12.5).color(muted()));
                }
            });
            ui.add_space(6.0);
            self.dashboard(ui, ctx, snapshot);
        } else {
            let mut clicked = false;
            egui::Frame::new().fill(panel()).stroke(Stroke::new(1.0, border())).corner_radius(14u8).inner_margin(egui::Margin { left: 4, right: 6, top: 2, bottom: 2 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    clicked = self.bar_chips(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if color_button(ui, Glyph::Down, t("Widgets ausklappen"), rgb(0xC98A12), 36.0, false).clicked() {
                            toggle = true;
                        }
                        if color_button(ui, Glyph::Layout, t("Widgets anordnen"), rgb(0x0F8B8D), 36.0, false).clicked() {
                            arrange = true;
                        }
                    });
                });
            });
            if clicked {
                toggle = true;
            }
        }
        if arrange {
            self.arrange = !self.arrange;
        }
        if toggle {
            self.cfg.home.bar_open = !self.cfg.home.bar_open;
            self.save_cfg();
        }
    }

    /// Die Einträge der eingeklappten Leiste in der Reihenfolge der Widgets; die Schlagzeile
    /// zuletzt, sie füllt den Rest. Liefert true, wenn einer angeklickt wurde.
    fn bar_chips(&mut self, ui: &mut egui::Ui) -> bool {
        let mut kinds: Vec<String> = self.widget_list().into_iter().map(|w| w.kind).filter(|k| k != "ordner" && k != "zuletzt").collect();
        if let Some(i) = kinds.iter().position(|k| k == "nachrichten") {
            let k = kinds.remove(i);
            kinds.push(k);
        }
        let reserve = 96.0;
        let mut clicked = false;
        let n = kinds.len();
        for (i, kind) in kinds.iter().enumerate() {
            let room = ui.available_width() - reserve;
            if room < 60.0 {
                break;
            }
            let color = widget_color(kind);
            let g = widget_glyph(kind);
            let plain = move |p: &egui::Painter, r: Rect| glyph(p, r, g, vivid(color));
            let sep = i + 1 < n;
            let max_w = if kind == "nachrichten" { room } else { room.min(260.0) };
            let hint = widgets::widget_name(kind);
            let hit = match kind.as_str() {
                "uhr" => {
                    let now = chrono::Local::now();
                    use chrono::Datelike;
                    let day: String = weekday(now.weekday().num_days_from_monday() as usize).chars().take(if sprache::is_en() { 3 } else { 2 }).collect();
                    let date = format!("{day} {}", now.format(if sprache::is_en() { "%m/%d" } else { "%d.%m." }));
                    chip_item(ui, &plain, &now.format("%H:%M").to_string(), &date, max_w, sep, hint)
                }
                "wetter" => {
                    let place = self.place();
                    let data = self.sky.lock().ok().and_then(|s| s.data.clone()).filter(|w| w.place == place.name);
                    match data {
                        Some(wx) => {
                            let sky = wx.sky;
                            let draw = move |p: &egui::Painter, r: Rect| weather::draw(p, r.center(), 22.0, sky, ACCENT, panel(), cloud());
                            chip_item(ui, &draw, &weather::deg(wx.temp), &place.name, max_w, sep, hint)
                        }
                        None => chip_item(ui, &plain, "", &place.name, max_w, sep, hint),
                    }
                }
                "mail" => {
                    let v = self.mail.as_ref().map(|m| m.unread.to_string()).unwrap_or_default();
                    chip_item(ui, &plain, &v, "", max_w, sep, hint)
                }
                "termine" => {
                    let next = self.next_event();
                    match next {
                        Some((when, title)) => chip_item(ui, &plain, &when, &title, max_w, sep, hint),
                        None => chip_item(ui, &plain, "", "", max_w, sep, hint),
                    }
                }
                "timer" => {
                    let left = match (self.timer.end, self.timer.left) {
                        (Some(end), _) => Some(end.saturating_duration_since(Instant::now())),
                        (None, Some(l)) => Some(l),
                        _ => None,
                    };
                    let v = if self.timer.rang { t("Zeit um!").to_string() } else { left.map(widgets::fmt_dur).unwrap_or_default() };
                    chip_item(ui, &plain, &v, "", max_w, sep, hint)
                }
                "computer" => {
                    let s = self.sys.lock().map(|s| s.clone()).unwrap_or_default();
                    let v = if s.mem_total == 0 { String::new() } else { format!("{:.0} %", s.cpu) };
                    chip_item(ui, &plain, &v, "", max_w, sep, hint)
                }
                "notizen" => {
                    let v = if self.cfg.notes.is_empty() { String::new() } else { self.cfg.notes.len().to_string() };
                    chip_item(ui, &plain, &v, "", max_w, sep, hint)
                }
                "nachrichten" => {
                    let first = self.news.lock().ok().and_then(|n| n.items.first().map(|i| i.title.clone())).unwrap_or_default();
                    chip_item(ui, &plain, "", &first, max_w, false, hint)
                }
                _ => false,
            };
            clicked |= hit;
        }
        clicked
    }

    /// Nächster Termin (heute ab jetzt, sonst der nächste Tag) als (Zeit oder Tag, Titel).
    fn next_event(&self) -> Option<(String, String)> {
        let has_url = self.cfg.calendar_url.as_deref().is_some_and(|u| !u.trim().is_empty());
        if !has_url {
            return None;
        }
        let c = self.cal.lock().ok()?;
        let now = chrono::Local::now().naive_local();
        let e = c.events.iter().find(|e| if e.all_day { e.start.date() >= now.date() } else { e.start >= now })?;
        let when = if e.start.date() == now.date() {
            if e.all_day {
                t("Heute").to_string()
            } else {
                e.start.format("%H:%M").to_string()
            }
        } else if e.start.date() == now.date() + chrono::Duration::days(1) {
            t("Morgen").to_string()
        } else {
            e.start.format(if sprache::is_en() { "%m/%d" } else { "%d.%m." }).to_string()
        };
        Some((when, e.title.clone()))
    }

    // ------------------------------------------------------------------ Schnellzugriff

    fn quick_list(&self) -> Vec<String> {
        self.cfg.home.quick.clone().unwrap_or_else(|| QUICK_DEFAULT.iter().map(|s| s.to_string()).collect())
    }

    pub(super) fn in_quick(&self, id: &str) -> bool {
        self.quick_list().iter().any(|q| q == id)
    }

    pub(super) fn toggle_quick(&mut self, id: &str) {
        let mut list = self.quick_list();
        if let Some(i) = list.iter().position(|q| q == id) {
            list.remove(i);
        } else {
            list.push(id.to_string());
        }
        self.cfg.home.quick = Some(list);
        self.save_cfg();
    }

    /// Programme, die im Schnellzugriff erscheinen (nur installierte, in eigener Reihenfolge).
    /// Ohne eigene Wahl und ohne Treffer: alle installierten.
    fn quick_apps(&self, mine: &[App]) -> Vec<App> {
        let list = self.quick_list();
        let apps: Vec<App> = list.iter().filter_map(|id| mine.iter().find(|a| &a.id == id).cloned()).collect();
        if apps.is_empty() && self.cfg.home.quick.is_none() {
            return mine.to_vec();
        }
        apps
    }

    pub(super) fn quick_access(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>, mine: &[App]) {
        let updates: usize = mine.iter().filter(|a| snapshot.get(&a.id).is_some_and(Status::update)).count();
        // Kopfzeile: Stern, Linie, Updates, Bearbeiten
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
            glyph(ui.painter(), r, Glyph::Star, vivid(rgb(0xE39B0B)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if color_button(ui, if self.quick_edit { Glyph::Check } else { Glyph::Pencil }, if self.quick_edit { t("Fertig") } else { t("Schnellzugriff bearbeiten") }, rgb(0x2F6FDE), 34.0, self.quick_edit).clicked() {
                    self.quick_edit = !self.quick_edit;
                }
                if updates > 0 {
                    let hint = if updates == 1 { t("1 Update bereit").to_string() } else { tf("{n} Updates bereit", &[("n", &updates)]) };
                    let r = color_button(ui, Glyph::Update, &format!("{hint} – {}", t("Aktualisieren")), rgb(0xE39B0B), 34.0, true);
                    let badge = Pos2::new(r.rect.right() - 2.0, r.rect.top() + 2.0);
                    ui.painter().circle_filled(badge, 8.0, ACCENT);
                    ui.painter().text(badge, egui::Align2::CENTER_CENTER, updates.to_string(), egui::FontId::proportional(10.5), INK);
                    if r.clicked() {
                        self.update_all(ctx, snapshot);
                    }
                }
                let w = ui.available_width() - 8.0;
                let (line, _) = ui.allocate_exact_size(egui::vec2(w.max(0.0), 20.0), Sense::hover());
                ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, border()));
            });
        });
        ui.add_space(8.0);
        let apps = self.quick_apps(mine);
        let (cw, ch, icon) = self.quick_cell();
        let gap = 14.0;
        let per_row = (((ui.available_width() + gap) / (cw + gap)).floor() as usize).max(1);
        let total = apps.len() + 1;
        let mut act: Option<QuickAct> = None;
        let mut toggle_all = false;
        for start in (0..total).step_by(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for i in start..(start + per_row).min(total) {
                    match apps.get(i) {
                        Some(app) => {
                            let st = snapshot.get(&app.id).cloned().unwrap_or_default();
                            if let Some(a) = self.quick_tile(ui, ctx, app, &st, cw, ch, icon) {
                                act = Some(a);
                            }
                        }
                        None => {
                            if self.all_tile(ui, cw, ch, icon).clicked() {
                                toggle_all = true;
                            }
                        }
                    }
                }
            });
            ui.add_space(gap - 6.0);
        }
        if toggle_all {
            self.show_all = !self.show_all;
            self.all_query.clear();
        }
        if let Some(a) = act {
            let mut list: Vec<String> = apps.iter().map(|a| a.id.clone()).collect();
            // Einträge, die gerade nicht installiert sind, bleiben am Ende erhalten.
            for id in self.quick_list() {
                if !list.contains(&id) {
                    list.push(id);
                }
            }
            match a {
                QuickAct::Remove(id) => list.retain(|x| x != &id),
                QuickAct::Move(id, forward) => {
                    if let Some(i) = list.iter().position(|x| x == &id) {
                        if forward && i > 0 {
                            list.swap(i, i - 1);
                        } else if !forward && i + 1 < apps.len() {
                            list.swap(i, i + 1);
                        }
                    }
                }
            }
            self.cfg.home.quick = Some(list);
            self.save_cfg();
        }
    }

    /// Breite und Höhe einer Kachel im Schnellzugriff und Grösse des Symbols, nach den Einstellungen.
    pub(super) fn quick_cell(&self) -> (f32, f32, f32) {
        let icon = [52.0, 68.0, 84.0][self.cfg.home.icon_size.min(2) as usize];
        if self.cfg.home.names {
            ((icon + 24.0_f32).max(104.0), icon + 46.0, icon)
        } else {
            (icon + 24.0, icon + 24.0, icon)
        }
    }

    /// Grosses Programm-Symbol, darunter (nach Wahl) der Name.
    fn quick_tile(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, app: &App, st: &Status, cw: f32, ch: f32, icon: f32) -> Option<QuickAct> {
        let update = st.update();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(cw, ch), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 18.0, hover());
        }
        let ir = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 12.0 + icon / 2.0), Vec2::splat(icon));
        paint_icon(ui, &mut self.textures, app, ir);
        let p = ui.painter();
        if self.cfg.home.names {
            let mut job = egui::text::LayoutJob::simple_singleline(app.name.clone(), egui::FontId::proportional(13.0), soft());
            job.wrap = egui::text::TextWrapping::truncate_at_width(cw - 6.0);
            let galley = p.layout_job(job);
            p.galley(Pos2::new(rect.center().x - galley.size().x / 2.0, ir.bottom() + 7.0), galley, soft());
        }
        if update {
            p.circle_filled(Pos2::new(ir.right() - 1.0, ir.top() + 1.0), 7.0, ACCENT);
            p.circle_stroke(Pos2::new(ir.right() - 1.0, ir.top() + 1.0), 7.0, Stroke::new(2.0, bg()));
        }
        if let Some((_, frac)) = &st.busy {
            let bar = Rect::from_min_size(Pos2::new(ir.left(), ir.bottom() + 3.0), egui::vec2(ir.width(), 4.0));
            p.rect_filled(bar, 2.0, border());
            p.rect_filled(Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac.clamp(0.02, 1.0), 4.0)), 2.0, ACCENT);
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let mut act = None;
        if self.quick_edit {
            let c = Pos2::new(ir.left() + 2.0, ir.top() + 2.0);
            p.circle_filled(c, 10.0, rgb(0xC8102E));
            glyph(p, Rect::from_center_size(c, Vec2::splat(14.0)), Glyph::Close, Color32::WHITE);
        }
        let resp = resp.on_hover_text(hover_hint(app, st, update));
        if resp.clicked() {
            if self.quick_edit {
                act = Some(QuickAct::Remove(app.id.clone()));
            } else if st.busy.is_none() {
                self.launch(ctx, app, st);
            }
        }
        resp.context_menu(|ui| {
            if ((st.native.is_some() && !app.web_only) || st.fremd) && ui.button(t("Starten")).clicked() {
                self.launch(ctx, app, st);
                ui.close_menu();
            }
            if !app.no_web && ui.button(t("Im Browser öffnen")).clicked() {
                self.open_web(ctx, app);
                ui.close_menu();
            }
            if update && ui.button(tf("Aktualisieren auf {v}", &[("v", &st.latest.map(fmt_version).unwrap_or_default())])).clicked() {
                self.install(ctx, app, st.update_kind(), None);
                ui.close_menu();
            }
            ui.separator();
            if ui.button(t("Nach vorn")).clicked() {
                act = Some(QuickAct::Move(app.id.clone(), true));
                ui.close_menu();
            }
            if ui.button(t("Nach hinten")).clicked() {
                act = Some(QuickAct::Move(app.id.clone(), false));
                ui.close_menu();
            }
            if ui.button(t("Aus dem Schnellzugriff")).clicked() {
                act = Some(QuickAct::Remove(app.id.clone()));
                ui.close_menu();
            }
        });
        act
    }

    /// Kachel «Alle Programme»: vier Felder in den Farben der Kategorien.
    fn all_tile(&mut self, ui: &mut egui::Ui, cw: f32, ch: f32, icon: f32) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(cw, ch), Sense::click());
        let r = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 12.0 + icon / 2.0), Vec2::splat(icon + 16.0));
        if self.cfg.home.names {
            let p = ui.painter();
            let galley = p.layout(t("Alle Programme").to_string(), egui::FontId::proportional(13.0), soft(), cw - 6.0);
            p.galley(Pos2::new(rect.center().x - galley.size().x / 2.0, r.bottom() - 1.0), galley, soft());
        }
        let p = ui.painter();
        if self.show_all {
            p.rect(r, 20.0, chip(), Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
        } else {
            if resp.hovered() {
                p.rect_filled(r, 20.0, hover());
            }
            p.rect_stroke(r, 20.0, Stroke::new(1.5, outline()), egui::StrokeKind::Inside);
        }
        let s = (icon * 0.24).round();
        let c = r.center();
        for (i, cat) in CATEGORIES.iter().take(4).enumerate() {
            let dx = if i % 2 == 0 { -s / 2.0 - 3.0 } else { s / 2.0 + 3.0 };
            let dy = if i < 2 { -s / 2.0 - 3.0 } else { s / 2.0 + 3.0 };
            p.rect_filled(Rect::from_center_size(c + egui::vec2(dx, dy), Vec2::splat(s)), 5.0, cat_color(*cat));
        }
        resp.on_hover_text(t("Alle Programme"))
    }

    // ------------------------------------------------------------------ Einstellungen

    /// Einstellungen «Aussehen der Startseite»: Vorschau und Wahlknöpfe. Liefert true bei Änderung.
    pub(super) fn look_settings(&mut self, ui: &mut egui::Ui) -> bool {
        self.look_preview(ui);
        ui.add_space(12.0);
        let mut changed = false;
        let h = &mut self.cfg.home;
        egui::Grid::new("aussehen").num_columns(2).spacing(egui::vec2(18.0, 10.0)).show(ui, |ui| {
            ui.label(t("Programmnamen"));
            ui.horizontal(|ui| {
                for (v, l) in [(true, t("Mit Namen")), (false, t("Nur Symbole"))] {
                    if pill(ui, l, h.names == v) {
                        h.names = v;
                        changed = true;
                    }
                }
            });
            ui.end_row();
            ui.label(t("Symbolgrösse"));
            ui.horizontal(|ui| {
                for (v, l) in [(0u8, t("Klein")), (1, t("Mittel")), (2, t("Gross"))] {
                    if pill(ui, l, h.icon_size == v) {
                        h.icon_size = v;
                        changed = true;
                    }
                }
            });
            ui.end_row();
            ui.label(t("Widget-Leiste"));
            ui.horizontal(|ui| {
                for (v, l) in [(false, t("Eingeklappt")), (true, t("Ausgeklappt"))] {
                    if pill(ui, l, h.bar_open == v) {
                        h.bar_open = v;
                        changed = true;
                    }
                }
            });
            ui.end_row();
            ui.label(t("Alle Programme"));
            ui.horizontal(|ui| {
                for (v, l) in [(true, t("Nach Kategorien")), (false, t("Alle nebeneinander"))] {
                    if pill(ui, l, h.grouped == v) {
                        h.grouped = v;
                        changed = true;
                    }
                }
            });
            ui.end_row();
        });
        changed
    }

    /// Kleine Vorschau der Startseite nach den gewählten Einstellungen.
    fn look_preview(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width().min(640.0);
        let open = self.cfg.home.bar_open;
        let names = self.cfg.home.names;
        let icon = [20.0, 26.0, 32.0][self.cfg.home.icon_size.min(2) as usize];
        let bar_h = if open { 86.0 } else { 18.0 };
        let cell_h = icon + if names { 18.0 } else { 6.0 };
        let height = 26.0 + 10.0 + bar_h + 12.0 + cell_h * 2.0 + 18.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, height), Sense::hover());
        let p = ui.painter().clone();
        p.rect(rect, 12.0, bg(), Stroke::new(1.0, border()), egui::StrokeKind::Inside);
        // Kopfzeile
        let head = Rect::from_min_size(rect.min, egui::vec2(w, 26.0));
        p.rect_filled(head, egui::CornerRadius { nw: 12, ne: 12, sw: 0, se: 0 }, header_bg());
        p.hline(head.x_range(), head.bottom(), Stroke::new(1.0, border()));
        p.rect_filled(Rect::from_center_size(Pos2::new(head.left() + 18.0, head.center().y), Vec2::splat(12.0)), 3.0, ACCENT);
        for (i, c) in [rgb(0x2F6FDE), rgb(0x7B4FD6)].iter().enumerate() {
            p.circle_filled(Pos2::new(head.right() - 44.0 - i as f32 * 18.0, head.center().y), 6.0, mix(vivid(*c), panel(), 0.35));
        }
        p.circle_filled(Pos2::new(head.right() - 20.0, head.center().y), 7.0, ACCENT);
        // Widget-Leiste
        let x0 = rect.left() + 12.0;
        let inner_w = w - 24.0;
        let mut y = head.bottom() + 10.0;
        if open {
            let gap = 6.0;
            let cols = [2.0, 3.0, 1.0, 2.0, 2.0, 2.0];
            let cw = (inner_w - gap * 5.0) / 6.0;
            let mut cx = 0.0;
            let mut row = 0.0;
            for (i, span) in cols.iter().enumerate() {
                if cx + span > 6.0 {
                    cx = 0.0;
                    row += 1.0;
                }
                let r = Rect::from_min_size(
                    Pos2::new(x0 + cx * (cw + gap), y + row * 43.0),
                    egui::vec2(span * cw + (span - 1.0) * gap, 38.0),
                );
                p.rect(r, 6.0, panel(), Stroke::new(1.0, border()), egui::StrokeKind::Inside);
                let kind = crate::core::WIDGET_KINDS.get(i).copied().unwrap_or("uhr");
                glyph(&p, Rect::from_min_size(r.min + egui::vec2(6.0, 6.0), Vec2::splat(10.0)), widget_glyph(kind), vivid(widget_color(kind)));
                p.rect_filled(Rect::from_min_size(r.min + egui::vec2(6.0, 22.0), egui::vec2((r.width() * 0.5).min(60.0), 5.0)), 2.0, chip());
                cx += span;
            }
        } else {
            let r = Rect::from_min_size(Pos2::new(x0, y), egui::vec2(inner_w, bar_h));
            p.rect(r, 6.0, panel(), Stroke::new(1.0, border()), egui::StrokeKind::Inside);
            let mut x = r.left() + 8.0;
            for kind in ["uhr", "wetter", "mail", "termine", "timer", "computer", "nachrichten"] {
                glyph(&p, Rect::from_center_size(Pos2::new(x + 4.0, r.center().y), Vec2::splat(10.0)), widget_glyph(kind), vivid(widget_color(kind)));
                let len = if kind == "nachrichten" { (r.right() - x - 40.0).max(10.0) } else { 22.0 };
                p.rect_filled(Rect::from_min_size(Pos2::new(x + 12.0, r.center().y - 2.0), egui::vec2(len, 4.0)), 2.0, chip());
                x += len + 22.0;
            }
        }
        y += bar_h + 12.0;
        // Schnellzugriff
        let quick: Vec<App> = self.quick_list().iter().filter_map(|id| self.manifest.app.iter().find(|a| &a.id == id).cloned()).collect();
        let cw = if names { (icon + 14.0_f32).max(44.0) } else { icon + 10.0 };
        let per_row = ((inner_w / cw).floor() as usize).max(1);
        let mut items: Vec<Option<&App>> = quick.iter().map(Some).collect();
        items.push(None);
        for (i, it) in items.iter().take(per_row * 2).enumerate() {
            let col = (i % per_row) as f32;
            let row = (i / per_row) as f32;
            let c = Pos2::new(x0 + col * cw + cw / 2.0, y + row * cell_h + icon / 2.0);
            let ir = Rect::from_center_size(c, Vec2::splat(icon));
            match it {
                Some(app) => paint_icon(ui, &mut self.textures, app, ir),
                None => {
                    p.rect_stroke(ir, icon * 0.25, Stroke::new(1.0, outline()), egui::StrokeKind::Inside);
                    let s = icon * 0.22;
                    for (k, cat) in CATEGORIES.iter().take(4).enumerate() {
                        let dx = if k % 2 == 0 { -s / 2.0 - 1.5 } else { s / 2.0 + 1.5 };
                        let dy = if k < 2 { -s / 2.0 - 1.5 } else { s / 2.0 + 1.5 };
                        p.rect_filled(Rect::from_center_size(c + egui::vec2(dx, dy), Vec2::splat(s)), 2.0, cat_color(*cat));
                    }
                }
            }
            if names {
                let name = match it {
                    Some(app) => app.name.clone(),
                    None => t("Alle Programme").to_string(),
                };
                let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(8.5), soft());
                job.wrap = egui::text::TextWrapping::truncate_at_width(cw - 2.0);
                let g = p.layout_job(job);
                p.galley(Pos2::new(c.x - g.size().x / 2.0, ir.bottom() + 3.0), g, soft());
            }
        }
    }

    /// Einstellungen «Widgets»: welche auf der Startseite stehen.
    pub(super) fn widget_settings(&mut self, ui: &mut egui::Ui) {
        let list = self.widget_list();
        let mut toggle: Option<(&'static str, bool)> = None;
        egui::Grid::new("widgets-an").num_columns(2).spacing(egui::vec2(28.0, 8.0)).show(ui, |ui| {
            for (i, kind) in crate::core::WIDGET_KINDS.iter().enumerate() {
                let mut on = list.iter().any(|w| w.kind == *kind);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    glyph(ui.painter(), r, widget_glyph(kind), vivid(widget_color(kind)));
                    if ui.checkbox(&mut on, widgets::widget_name(kind)).changed() {
                        toggle = Some((*kind, on));
                    }
                });
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        if let Some((kind, on)) = toggle {
            let mut list = list;
            if on {
                let w = crate::core::default_widgets().into_iter().find(|w| w.kind == kind).map(|w| w.w).unwrap_or(2);
                list.push(crate::core::WidgetCfg { kind: kind.to_string(), w, h: 2 });
            } else {
                list.retain(|w| w.kind != kind);
            }
            self.set_widgets(list);
        }
    }

    // ------------------------------------------------------------------ Alle Programme

    pub(super) fn all_programs(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>, mine: &[App]) {
        let mut close = false;
        egui::Frame::new().fill(panel()).stroke(Stroke::new(1.0, border())).corner_radius(16u8).inner_margin(14.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                glyph(ui.painter(), r, Glyph::Search, vivid(rgb(0x2F6FDE)));
                let field = ui.add(egui::TextEdit::singleline(&mut self.all_query).desired_width(280.0).font(egui::FontId::proportional(15.0)).margin(egui::vec2(10.0, 7.0)));
                if self.all_focus {
                    field.request_focus();
                    self.all_focus = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if color_button(ui, Glyph::Close, t("Schliessen"), rgb(0x5B616E), 36.0, false).clicked() {
                        close = true;
                    }
                    ui.add_space(6.0);
                    if color_button(ui, Glyph::Apps, t("Alle nebeneinander"), rgb(0x7B4FD6), 36.0, !self.cfg.home.grouped).clicked() {
                        self.cfg.home.grouped = false;
                        self.save_cfg();
                    }
                    if color_button(ui, Glyph::Layers, t("Nach Kategorien"), rgb(0x2F6FDE), 36.0, self.cfg.home.grouped).clicked() {
                        self.cfg.home.grouped = true;
                        self.save_cfg();
                    }
                });
            });
            ui.add_space(10.0);
            let q = self.all_query.trim().to_lowercase();
            let list: Vec<App> = mine
                .iter()
                .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.about_text().to_lowercase().contains(&q) || a.id.contains(&q))
                .cloned()
                .collect();
            if list.is_empty() {
                ui.label(egui::RichText::new(t("Keine Treffer")).color(muted()));
                return;
            }
            if self.cfg.home.grouped && q.is_empty() {
                self.category_tiles(ui, ctx, snapshot, &list);
            } else {
                let cell = if self.cfg.home.names { 96.0 } else { 84.0 };
                let per_row = ((ui.available_width() / cell).floor() as usize).max(1);
                for chunk in list.chunks(per_row) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for app in chunk {
                            let st = snapshot.get(&app.id).cloned().unwrap_or_default();
                            self.icon_button(ui, ctx, app, &st, cell);
                        }
                    });
                }
            }
        });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_all = false;
        }
    }
}
