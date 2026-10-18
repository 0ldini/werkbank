//! Widgets der Startseite: Raster mit 6 Spalten, Grösse je Widget wählbar, dichtes Packen ohne
//! Lücken (wie das PoolBuddy-Dashboard), Modus «Widgets anordnen».

use super::*;
use crate::core::WidgetCfg;
use crate::feeds;

/// Lage eines Widgets im Raster (Spalte, Zeile, Breite, Höhe in Feldern).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub idx: usize,
    pub col: usize,
    pub row: usize,
    pub w: usize,
    pub h: usize,
}

/// Packt Widgets der Reihe nach an die erste freie Stelle (zeilenweise) und schliesst danach
/// verbliebene Lücken, indem Nachbarn in die Breite wachsen.
pub fn pack(sizes: &[(usize, usize)], cols: usize) -> Vec<Placed> {
    let cols = cols.max(1);
    let mut grid: Vec<Vec<Option<usize>>> = Vec::new();
    let mut placed = Vec::new();
    let free = |grid: &Vec<Vec<Option<usize>>>, r: usize, c: usize| grid.get(r).is_none_or(|row| row[c].is_none());
    for (idx, &(w, h)) in sizes.iter().enumerate() {
        let (w, h) = (w.clamp(1, cols), h.clamp(1, 4));
        let mut row = 0;
        'search: loop {
            for col in 0..=cols - w {
                if (row..row + h).all(|r| (col..col + w).all(|c| free(&grid, r, c))) {
                    while grid.len() < row + h {
                        grid.push(vec![None; cols]);
                    }
                    for r in row..row + h {
                        for c in col..col + w {
                            grid[r][c] = Some(placed.len());
                        }
                    }
                    placed.push(Placed { idx, col, row, w, h });
                    break 'search;
                }
            }
            row += 1;
        }
    }
    // Lücken füllen: nach rechts, dann nach links wachsen, solange alle Zeilen frei sind.
    for i in 0..placed.len() {
        loop {
            let p = placed[i];
            let c = p.col + p.w;
            if c >= cols || !(p.row..p.row + p.h).all(|r| grid[r][c].is_none()) {
                break;
            }
            for r in p.row..p.row + p.h {
                grid[r][c] = Some(i);
            }
            placed[i].w += 1;
        }
        loop {
            let p = placed[i];
            if p.col == 0 || !(p.row..p.row + p.h).all(|r| grid[r][p.col - 1].is_none()) {
                break;
            }
            for r in p.row..p.row + p.h {
                grid[r][p.col - 1] = Some(i);
            }
            placed[i].col -= 1;
            placed[i].w += 1;
        }
    }
    placed
}

pub fn widget_name(kind: &str) -> &'static str {
    t(match kind {
        "uhr" => "Uhr",
        "wetter" => "Wetter",
        "mail" => "Mail",
        "termine" => "Termine",
        "zuletzt" => "Zuletzt geöffnet",
        "notizen" => "Notizen",
        "nachrichten" => "Nachrichten",
        "timer" => "Timer",
        "ordner" => "Ordner",
        "computer" => "Computer",
        _ => "Widget",
    })
}

pub const SRF_NEWS: &str = "https://www.srf.ch/news/bnf/rss/1646";

#[derive(Default)]
pub struct CalState {
    pub events: Vec<feeds::Event>,
    pub error: Option<String>,
    pub busy: bool,
    pub url: String,
}

#[derive(Default)]
pub struct NewsState {
    pub items: Vec<feeds::Item>,
    pub error: Option<String>,
    pub busy: bool,
    pub url: String,
}

#[derive(Default, Clone)]
pub struct SysState {
    pub cpu: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub battery: Option<u8>,
}

#[derive(Default, Clone)]
pub struct MailItem {
    pub from: String,
    pub subject: String,
}

#[derive(Default, Clone)]
pub struct MailStatus {
    pub unread: u64,
    pub items: Vec<MailItem>,
}

#[derive(Default)]
pub struct TimerState {
    /// Ende, solange der Timer läuft.
    pub end: Option<Instant>,
    /// Restzeit, wenn angehalten.
    pub left: Option<Duration>,
    pub minutes: String,
    pub rang: bool,
}

fn battery() -> Option<u8> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        // SAFETY: GetSystemPowerStatus füllt nur die übergebene Struktur.
        let mut st: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        if unsafe { GetSystemPowerStatus(&mut st) } != 0 && st.BatteryFlag != 128 && st.BatteryLifePercent <= 100 {
            return Some(st.BatteryLifePercent);
        }
        None
    }
    #[cfg(not(windows))]
    {
        for e in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
            if e.file_name().to_string_lossy().starts_with("BAT") {
                if let Some(v) = std::fs::read_to_string(e.path().join("capacity")).ok().and_then(|s| s.trim().parse().ok()) {
                    return Some(v);
                }
            }
        }
        None
    }
}

fn mail_status() -> Option<MailStatus> {
    let path = sprache::werkbank_file()?.with_file_name("mailwerk-status.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let items = v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| MailItem {
                    from: x["from"].as_str().unwrap_or("").to_string(),
                    subject: x["subject"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(MailStatus { unread: v["unread"].as_u64().unwrap_or(0), items })
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

fn gb(bytes: u64) -> String {
    let v = bytes as f64 / 1_073_741_824.0;
    if sprache::is_en() {
        format!("{v:.1} GB")
    } else {
        format!("{v:.1} GB").replace('.', ",")
    }
}

/// Kleiner Fortschrittsbalken mit Beschriftung.
fn meter(ui: &mut egui::Ui, label: &str, value: &str, frac: f32, color: Color32) {
    if ui.available_width() < 240.0 {
        ui.label(egui::RichText::new(label).size(13.0));
        ui.label(egui::RichText::new(value).size(12.0).color(muted()));
    } else {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).size(13.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(value).size(13.0).color(muted()));
            });
        });
    }
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 7.0), Sense::hover());
    ui.painter().rect_filled(r, 4.0, chip());
    ui.painter().rect_filled(Rect::from_min_size(r.min, egui::vec2(r.width() * frac.clamp(0.0, 1.0), r.height())), 4.0, color);
}

/// Titelzeile eines Widgets mit optionalem Link rechts; liefert true, wenn der Link geklickt wurde.
fn title_row(ui: &mut egui::Ui, title: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(14.0).color(muted()));
        if let Some(a) = action {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked = link(ui, a).clicked();
            });
        }
    });
    clicked
}

impl Gui {
    pub(super) fn widget_list(&self) -> Vec<WidgetCfg> {
        if self.cfg.home.widgets.is_empty() {
            crate::core::default_widgets()
        } else {
            self.cfg.home.widgets.clone()
        }
    }

    fn set_widgets(&mut self, list: Vec<WidgetCfg>) {
        self.cfg.home.widgets = list;
        self.save_cfg();
    }

    /// Daten der Widgets im Hintergrund holen (Kalender, Nachrichten, Computer, Mail).
    pub(super) fn tick_widgets(&mut self, ctx: &egui::Context) {
        let kinds: Vec<String> = self.widget_list().into_iter().map(|w| w.kind).collect();
        let has = |k: &str| kinds.iter().any(|x| x == k);
        // Termine alle 15 Minuten oder wenn die Adresse geändert wurde.
        let cal_url = self.cfg.calendar_url.clone().unwrap_or_default().trim().replace("webcal://", "https://");
        let cal_changed = self.cal.lock().map(|c| c.url != cal_url).unwrap_or(false);
        if has("termine") && !cal_url.is_empty() && (cal_changed || self.cal_at.is_none_or(|t| t.elapsed() > Duration::from_secs(900))) {
            self.cal_at = Some(Instant::now());
            let (cal, ctx) = (self.cal.clone(), ctx.clone());
            if let Ok(mut c) = cal.lock() {
                if c.busy && !cal_changed {
                    return;
                }
                c.busy = true;
                c.url = cal_url.clone();
            }
            std::thread::spawn(move || {
                let r = core::http_text(&cal_url, 8 << 20);
                if let Ok(mut c) = cal.lock() {
                    c.busy = false;
                    match r {
                        Ok(text) => {
                            c.events = feeds::parse_ics(&text, chrono::Local::now().naive_local().date().and_time(chrono::NaiveTime::MIN), 14);
                            c.error = None;
                        }
                        Err(e) => c.error = Some(format!("{e:#}")),
                    }
                }
                ctx.request_repaint();
            });
        }
        let news_url = self.cfg.news_url.clone().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| SRF_NEWS.to_string());
        let news_changed = self.news.lock().map(|n| n.url != news_url).unwrap_or(false);
        if has("nachrichten") && (news_changed || self.news_at.is_none_or(|t| t.elapsed() > Duration::from_secs(1200))) {
            self.news_at = Some(Instant::now());
            let (news, ctx) = (self.news.clone(), ctx.clone());
            if let Ok(mut n) = news.lock() {
                n.busy = true;
                n.url = news_url.clone();
            }
            std::thread::spawn(move || {
                let r = core::http_text(&news_url, 4 << 20);
                if let Ok(mut n) = news.lock() {
                    n.busy = false;
                    match r {
                        Ok(text) => {
                            n.items = feeds::parse_rss(&text);
                            n.error = if n.items.is_empty() { Some(t("Keine Einträge gefunden").to_string()) } else { None };
                        }
                        Err(e) => n.error = Some(format!("{e:#}")),
                    }
                }
                ctx.request_repaint();
            });
        }
        if has("computer") && !self.sys_started {
            self.sys_started = true;
            let (sys, ctx) = (self.sys.clone(), ctx.clone());
            std::thread::spawn(move || {
                let mut s = sysinfo::System::new();
                loop {
                    s.refresh_cpu_usage();
                    s.refresh_memory();
                    if let Ok(mut st) = sys.lock() {
                        st.cpu = s.global_cpu_usage();
                        st.mem_used = s.used_memory();
                        st.mem_total = s.total_memory();
                        st.battery = battery();
                    }
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_secs(2));
                }
            });
        }
        if has("mail") && self.mail_at.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
            self.mail_at = Some(Instant::now());
            self.mail = mail_status();
        }
        if has("timer") {
            if let Some(end) = self.timer.end {
                if Instant::now() >= end {
                    self.timer.end = None;
                    self.timer.left = None;
                    self.timer.rang = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                } else {
                    ctx.request_repaint_after(Duration::from_millis(250));
                }
            }
        }
    }

    /// Das Widget-Raster; im Modus «anordnen» mit Werkzeugleiste je Widget.
    pub(super) fn dashboard(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        let list = self.widget_list();
        let width = ui.available_width();
        let cols = if width >= 900.0 {
            6
        } else if width >= 560.0 {
            4
        } else {
            2
        };
        let scale = |w: u8| -> usize { ((w as usize * cols + 5) / 6).clamp(1, cols) };
        let sizes: Vec<(usize, usize)> = list.iter().map(|w| (scale(w.w), w.h as usize)).collect();
        let placed = pack(&sizes, cols);
        let (gap, row_h) = (14.0, 92.0);
        let cell_w = (width - gap * (cols as f32 - 1.0)) / cols as f32;
        let rows = placed.iter().map(|p| p.row + p.h).max().unwrap_or(0);
        let height = if rows == 0 { 0.0 } else { rows as f32 * row_h + (rows as f32 - 1.0) * gap };
        let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        let mut action: Option<(usize, &'static str, u8)> = None;
        for p in &placed {
            let r = Rect::from_min_size(
                Pos2::new(area.left() + p.col as f32 * (cell_w + gap), area.top() + p.row as f32 * (row_h + gap)),
                egui::vec2(p.w as f32 * cell_w + (p.w as f32 - 1.0) * gap, p.h as f32 * row_h + (p.h as f32 - 1.0) * gap),
            );
            let edit = self.arrange;
            let stroke = if edit { Stroke::new(1.5, outline()) } else { Stroke::new(1.0, border()) };
            ui.painter().rect(r, 16.0, panel(), stroke, egui::StrokeKind::Inside);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(egui::vec2(18.0, 14.0))).layout(egui::Layout::top_down(egui::Align::Min)));
            child.set_clip_rect(r.shrink(2.0));
            let cfg = &list[p.idx];
            if edit {
                child.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(egui::RichText::new(widget_name(&cfg.kind)).strong());
                    if ui.small_button("⬅").on_hover_text(t("Nach vorn")).clicked() {
                        action = Some((p.idx, "vor", 0));
                    }
                    if ui.small_button("➡").on_hover_text(t("Nach hinten")).clicked() {
                        action = Some((p.idx, "zurueck", 0));
                    }
                    ui.label(egui::RichText::new(t("Breite")).size(12.0).color(muted()));
                    for w in [1u8, 2, 3, 4, 6] {
                        if ui.selectable_label(cfg.w == w, w.to_string()).clicked() {
                            action = Some((p.idx, "breite", w));
                        }
                    }
                    ui.label(egui::RichText::new(t("Höhe")).size(12.0).color(muted()));
                    for h in [1u8, 2, 3] {
                        if ui.selectable_label(cfg.h == h, h.to_string()).clicked() {
                            action = Some((p.idx, "hoehe", h));
                        }
                    }
                    if ui.small_button("×").on_hover_text(t("Widget entfernen")).clicked() {
                        action = Some((p.idx, "weg", 0));
                    }
                });
                child.separator();
            }
            let compact = p.h == 1;
            match cfg.kind.as_str() {
                "uhr" => self.w_clock(&mut child, compact),
                "wetter" => self.w_weather(&mut child, ctx, p.w, compact),
                "mail" => self.w_mail(&mut child, ctx, snapshot),
                "termine" => self.w_calendar(&mut child),
                "zuletzt" => self.w_recent(&mut child, snapshot),
                "notizen" => self.w_notes(&mut child),
                "nachrichten" => self.w_news(&mut child, p.w),
                "timer" => self.w_timer(&mut child, ctx, compact),
                "ordner" => self.w_folders(&mut child),
                "computer" => self.w_computer(&mut child),
                _ => {}
            }
        }
        if let Some((idx, what, v)) = action {
            let mut list = list;
            match what {
                "vor" if idx > 0 => list.swap(idx, idx - 1),
                "zurueck" if idx + 1 < list.len() => list.swap(idx, idx + 1),
                "breite" => list[idx].w = v,
                "hoehe" => list[idx].h = v,
                "weg" => {
                    list.remove(idx);
                }
                _ => {}
            }
            self.set_widgets(list);
        }
        if self.arrange {
            ui.add_space(10.0);
            let missing: Vec<&str> = crate::core::WIDGET_KINDS.iter().copied().filter(|k| !self.widget_list().iter().any(|w| w.kind == *k)).collect();
            ui.horizontal_wrapped(|ui| {
                if missing.is_empty() {
                    ui.label(egui::RichText::new(t("Alle Widgets sind auf der Startseite.")).color(muted()));
                }
                for k in missing {
                    if button(ui, &format!("+ {}", widget_name(k)), Look::Outline).clicked() {
                        let mut list = self.widget_list();
                        list.push(WidgetCfg { kind: k.to_string(), w: 2, h: 2 });
                        self.set_widgets(list);
                    }
                }
                if link(ui, t("Vorgabe wiederherstellen")).clicked() {
                    self.set_widgets(Vec::new());
                }
            });
        }
    }

    fn w_clock(&mut self, ui: &mut egui::Ui, compact: bool) {
        use chrono::Timelike;
        let now = chrono::Local::now();
        if compact {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(now.format("%H:%M").to_string()).size(34.0).strong());
                ui.label(egui::RichText::new(long_date(now.date_naive())).size(14.0).color(soft()));
            });
            return;
        }
        ui.label(egui::RichText::new(greeting(now.hour())).size(14.0).color(muted()));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(now.format("%H:%M").to_string()).size(58.0).strong());
        ui.label(egui::RichText::new(long_date(now.date_naive())).size(15.0).color(soft()));
    }

    fn w_weather(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, w: usize, compact: bool) {
        let place = self.place();
        let (data, error, busy) = match self.sky.lock() {
            Ok(s) => (s.data.clone().filter(|w| w.place == place.name), s.error.clone(), s.busy),
            Err(_) => (None, None, false),
        };
        if title_row(ui, &place.name, Some(t("Ort ändern"))) {
            self.go(View::Prefs);
        }
        let Some(wx) = data else {
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new(t("Wetter wird geladen …")).color(muted()));
                });
            } else {
                ui.label(egui::RichText::new(t("Kein Wetter verfügbar")).size(15.0));
                if let Some(e) = &error {
                    ui.label(egui::RichText::new(t("Keine Verbindung zum Wetterdienst")).size(12.5).color(muted())).on_hover_text(e);
                }
                if link(ui, t("Erneut versuchen")).clicked() {
                    self.fetch_weather(ctx);
                }
            }
            return;
        };
        let desc = wx.code.map(|c| t(weather::describe(c, wx.sky != weather::Sky::Moon).0).to_string()).unwrap_or_else(|| wx.text.clone());
        let mut info = format!("H {} · T {}", weather::deg(wx.today_max), weather::deg(wx.today_min));
        if wx.rain_prob > 0.0 {
            info += &format!(" · {}", tf("Regen {p} %", &[("p", &format!("{:.0}", wx.rain_prob))]));
        }
        let side = w >= 3 && !compact;
        let top = ui.cursor().min;
        let width = ui.available_width();
        let left_w = if side { (width * 0.42).min(220.0) } else { width };
        ui.allocate_ui(egui::vec2(left_w, 0.0), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(weather::deg(wx.temp)).size(if compact { 30.0 } else if side { 44.0 } else { 34.0 }).strong());
                if compact {
                    ui.label(egui::RichText::new(&desc).size(14.0));
                }
            });
            if side {
                ui.label(egui::RichText::new(&desc).size(15.0));
                ui.label(egui::RichText::new(&info).size(12.5).color(muted()));
            } else if !compact {
                ui.label(egui::RichText::new(format!("{desc} · {info}")).size(12.5).color(soft()));
            }
        });
        let icon_s = if compact || !side { 34.0 } else { 56.0 };
        let icon_c = if side { Pos2::new(top.x + left_w - icon_s * 0.5, top.y + icon_s * 0.5) } else { Pos2::new(top.x + width - icon_s * 0.5, top.y + icon_s * 0.5) };
        weather::draw(ui.painter(), icon_c, icon_s, wx.sky, ACCENT, panel(), cloud());
        if compact {
            return;
        }
        let days: Vec<&weather::Day> = wx.days.iter().take(5).collect();
        if days.is_empty() {
            return;
        }
        let rect = if side {
            Rect::from_min_max(Pos2::new(top.x + left_w + 16.0, top.y), Pos2::new(top.x + width, ui.max_rect().bottom()))
        } else {
            let bottom = ui.max_rect().bottom();
            Rect::from_min_max(Pos2::new(top.x, (bottom - 52.0).max(ui.cursor().min.y)), Pos2::new(top.x + width, bottom))
        };
        if side {
            ui.painter().line_segment([Pos2::new(rect.left() - 8.0, rect.top() + 4.0), Pos2::new(rect.left() - 8.0, rect.bottom() - 4.0)], Stroke::new(1.0, border()));
        }
        let cw = rect.width() / days.len() as f32;
        let cy = rect.center().y;
        let p = ui.painter();
        for (i, d) in days.iter().enumerate() {
            let x = rect.left() + cw * (i as f32 + 0.5);
            p.text(Pos2::new(x, cy - 20.0), egui::Align2::CENTER_CENTER, weekday_short(&d.weekday), egui::FontId::proportional(12.0), muted());
            weather::draw(p, Pos2::new(x, cy), 22.0, d.sky, ACCENT, panel(), cloud());
            p.text(Pos2::new(x, cy + 20.0), egui::Align2::CENTER_CENTER, weather::deg(d.max), egui::FontId::proportional(13.0), text());
        }
    }

    fn w_mail(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &HashMap<String, Status>) {
        let app = self.manifest.app.iter().find(|a| a.id == "mailwerk").cloned();
        let open = title_row(ui, t("Mail"), app.as_ref().map(|_| t("Öffnen")));
        if let (true, Some(app)) = (open, &app) {
            let st = snapshot.get(&app.id).cloned().unwrap_or_default();
            if st.installed() {
                self.launch(ctx, app, &st);
            } else {
                self.open_catalog(Chip::Office);
            }
        }
        match &self.mail {
            Some(m) => {
                ui.horizontal(|ui| {
                    if let Some(app) = &app {
                        icon(ui, &mut self.textures, app, 34.0);
                    }
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(m.unread.to_string()).size(28.0).strong());
                        ui.label(egui::RichText::new(t("ungelesen")).size(12.5).color(muted()));
                    });
                });
                for it in m.items.iter().take(3) {
                    let mut job = egui::text::LayoutJob::simple_singleline(format!("{} · {}", it.from, it.subject), egui::FontId::proportional(13.0), text());
                    job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                    ui.label(job);
                }
            }
            None => {
                ui.label(egui::RichText::new(t("Mailwerk einmal öffnen, dann erscheinen hier die ungelesenen Mails.")).size(13.0).color(muted()));
            }
        }
    }

    fn w_calendar(&mut self, ui: &mut egui::Ui) {
        let has_url = self.cfg.calendar_url.as_deref().is_some_and(|u| !u.trim().is_empty());
        if title_row(ui, t("Termine"), Some(if has_url { t("Kalender") } else { t("Verbinden") })) {
            self.go(View::Prefs);
        }
        if !has_url {
            ui.label(egui::RichText::new(t("Kalender-Adresse (ICS) in den Einstellungen eintragen, z. B. die «Privatadresse im iCal-Format» von Google Kalender.")).size(13.0).color(muted()));
            return;
        }
        let (events, error, busy) = match self.cal.lock() {
            Ok(c) => (c.events.clone(), c.error.clone(), c.busy),
            Err(_) => (Vec::new(), None, false),
        };
        if events.is_empty() {
            if busy {
                ui.spinner();
            } else if let Some(e) = error {
                ui.label(egui::RichText::new(t("Kalender nicht erreichbar")).color(warn())).on_hover_text(e);
            } else {
                ui.label(egui::RichText::new(t("Keine Termine in den nächsten zwei Wochen.")).color(muted()));
            }
            return;
        }
        let today = chrono::Local::now().date_naive();
        let now = chrono::Local::now().naive_local();
        for e in events.iter().filter(|e| e.all_day || e.start >= now - chrono::Duration::hours(1)) {
            if ui.available_height() < 18.0 {
                break;
            }
            let day = e.start.date();
            let d = (day - today).num_days();
            let label = match d {
                0 => t("Heute").to_string(),
                1 => t("Morgen").to_string(),
                2..=6 => weekday_short_full(day),
                _ => day.format(if sprache::is_en() { "%m/%d" } else { "%d.%m." }).to_string(),
            };
            let time = if e.all_day { String::new() } else { e.start.format("%H:%M ").to_string() };
            ui.horizontal(|ui| {
                ui.add_sized([56.0, 18.0], egui::Label::new(egui::RichText::new(label).strong().size(13.5)));
                let mut job = egui::text::LayoutJob::simple_singleline(format!("{time}{}", e.title), egui::FontId::proportional(13.5), text());
                job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                ui.label(job);
            });
        }
    }

    fn w_recent(&mut self, ui: &mut egui::Ui, snapshot: &HashMap<String, Status>) {
        let now = chrono::Local::now();
        let recent: Vec<Recent> = self.cfg.recent.iter().filter(|r| r.path.exists()).take(6).cloned().collect();
        let pick_file = title_row(ui, t("Zuletzt geöffnet"), Some(t("Datei öffnen …")));
        let mut open: Option<Recent> = None;
        if recent.is_empty() {
            ui.label(egui::RichText::new(t("Noch keine Dateien")).size(15.0));
            ui.label(egui::RichText::new(t("Dateien, die du über die Werkbank öffnest, erscheinen hier.")).size(12.5).color(muted()));
        }
        for r in &recent {
            if ui.available_height() < 40.0 {
                break;
            }
            let Some(app) = self.manifest.app.iter().find(|a| a.id == r.app) else { continue };
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect.expand2(egui::vec2(6.0, 0.0)), 8.0, hover());
            }
            paint_icon(ui, &mut self.textures, app, Rect::from_min_size(Pos2::new(rect.left(), rect.top() + 5.0), Vec2::splat(30.0)));
            let name = r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let p = ui.painter();
            let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(14.0), text());
            job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 42.0);
            p.galley(Pos2::new(rect.left() + 42.0, rect.top() + 3.0), p.layout_job(job), text());
            p.text(Pos2::new(rect.left() + 42.0, rect.top() + 22.0), egui::Align2::LEFT_TOP, format!("{} · {}", app.name, when(r.when, now)), egui::FontId::proportional(12.0), muted());
            let installed = snapshot.get(&app.id).is_some_and(|s| s.native.is_some());
            if resp.on_hover_text(r.path.display().to_string()).clicked() {
                if installed {
                    open = Some(r.clone());
                } else {
                    self.notice = Some(tf("{name} ist nicht mehr installiert.", &[("name", &app.name)]));
                }
            }
        }
        if let Some(r) = open {
            self.open_recent(&r);
        }
        if pick_file {
            self.open_file(snapshot);
        }
    }

    fn w_notes(&mut self, ui: &mut egui::Ui) {
        title_row(ui, t("Notizen"), None);
        let mut remove = None;
        let rows = ((ui.available_height() - 34.0) / 22.0).floor().max(0.0) as usize;
        for (i, n) in self.cfg.notes.iter().enumerate().take(rows) {
            ui.horizontal(|ui| {
                let mut job = egui::text::LayoutJob::simple_singleline(n.clone(), egui::FontId::proportional(14.0), text());
                job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width() - 24.0);
                ui.label(job).on_hover_text(n);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("×").size(11.0).color(muted())).frame(false)).on_hover_text(t("Erledigt")).clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if self.cfg.notes.len() > rows {
            ui.label(egui::RichText::new(tf("+ {n} weitere", &[("n", &(self.cfg.notes.len() - rows))])).size(12.0).color(muted()));
        }
        let r = ui.add(egui::TextEdit::singleline(&mut self.note_input).hint_text(t("+ Notiz")).desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.note_input.trim().is_empty() {
            self.cfg.notes.push(self.note_input.trim().to_string());
            self.note_input.clear();
            self.save_notes();
            r.request_focus();
        }
        if let Some(i) = remove {
            self.cfg.notes.remove(i);
            self.save_notes();
        }
    }

    fn save_notes(&mut self) {
        let mut cfg = core::load_config();
        cfg.notes = self.cfg.notes.clone();
        let _ = core::save_config(&cfg);
    }

    fn w_news(&mut self, ui: &mut egui::Ui, w: usize) {
        let (items, error, busy) = match self.news.lock() {
            Ok(n) => (n.items.clone(), n.error.clone(), n.busy),
            Err(_) => (Vec::new(), None, false),
        };
        if title_row(ui, t("Nachrichten"), Some(t("Quelle wählen"))) {
            self.go(View::Prefs);
        }
        if items.is_empty() {
            if busy {
                ui.spinner();
            } else if let Some(e) = error {
                ui.label(egui::RichText::new(t("Nachrichten nicht erreichbar")).color(warn())).on_hover_text(e);
            }
            return;
        }
        let now = chrono::Local::now();
        let cols = if w >= 4 { 2 } else { 1 };
        let col_w = (ui.available_width() - 20.0 * (cols as f32 - 1.0)) / cols as f32;
        let fit = ((ui.available_height() + 4.0) / 44.0).floor().max(1.0) as usize * cols;
        let mut open = None;
        egui::Grid::new(ui.id().with("news")).num_columns(cols).spacing(egui::vec2(20.0, 4.0)).show(ui, |ui| {
            for (i, it) in items.iter().take(fit).enumerate() {
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    let mut job = egui::text::LayoutJob::simple_singleline(it.title.clone(), egui::FontId::proportional(14.0), text());
                    job.wrap = egui::text::TextWrapping::truncate_at_width(col_w);
                    if ui.add(egui::Label::new(job).sense(Sense::click())).on_hover_text(&it.title).clicked() && !it.link.is_empty() {
                        open = Some(it.link.clone());
                    }
                    let age = it.date.map(|d| {
                        let m = (now - d).num_minutes().max(0);
                        if m < 60 {
                            tf("vor {n} Min.", &[("n", &m)])
                        } else if m < 60 * 24 {
                            tf("vor {n} Std.", &[("n", &(m / 60))])
                        } else {
                            d.format("%d.%m.").to_string()
                        }
                    });
                    ui.label(egui::RichText::new(age.unwrap_or_default()).size(12.0).color(muted()));
                });
                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
        if let Some(l) = open {
            let _ = open::that(l);
        }
    }

    fn w_timer(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context, compact: bool) {
        title_row(ui, t("Timer"), None);
        let left = match (self.timer.end, self.timer.left) {
            (Some(end), _) => end.saturating_duration_since(Instant::now()),
            (None, Some(l)) => l,
            _ => Duration::ZERO,
        };
        let running = self.timer.end.is_some();
        let shown = if self.timer.rang { t("Zeit um!").to_string() } else { fmt_dur(left) };
        let color = if self.timer.rang { accent_text() } else { text() };
        ui.label(egui::RichText::new(shown).size(if compact { 26.0 } else { 40.0 }).strong().color(color));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for m in [5u64, 25, 60] {
                if ui.small_button(format!("{m}")).on_hover_text(tf("{n} Minuten", &[("n", &m)])).clicked() {
                    self.timer = TimerState { end: Some(Instant::now() + Duration::from_secs(m * 60)), ..Default::default() };
                }
            }
            let r = ui.add(egui::TextEdit::singleline(&mut self.timer.minutes).hint_text(t("Min.")).desired_width(38.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(m) = self.timer.minutes.trim().replace(',', ".").parse::<f64>() {
                    if m > 0.0 && m < 24.0 * 60.0 {
                        self.timer = TimerState { end: Some(Instant::now() + Duration::from_secs_f64(m * 60.0)), ..Default::default() };
                    }
                }
            }
            if running {
                if ui.small_button(t("Pause")).clicked() {
                    self.timer.left = Some(left);
                    self.timer.end = None;
                }
            } else if self.timer.left.is_some() && ui.small_button(t("Weiter")).clicked() {
                self.timer.end = Some(Instant::now() + left);
                self.timer.left = None;
            }
            if (running || self.timer.left.is_some() || self.timer.rang) && ui.small_button(t("Stopp")).clicked() {
                self.timer = TimerState::default();
            }
        });
    }

    fn folder_list(&self) -> Vec<std::path::PathBuf> {
        if let Some(f) = &self.cfg.folders {
            return f.clone();
        }
        directories::UserDirs::new()
            .map(|u| [u.download_dir(), u.document_dir(), u.picture_dir()].into_iter().flatten().map(|p| p.to_path_buf()).collect())
            .unwrap_or_default()
    }

    fn w_folders(&mut self, ui: &mut egui::Ui) {
        let edit_label = if self.folders_edit { t("Fertig") } else { t("Bearbeiten") };
        if title_row(ui, t("Ordner"), Some(edit_label)) {
            self.folders_edit = !self.folders_edit;
        }
        let folders = self.folder_list();
        let mut remove = None;
        for (i, f) in folders.iter().enumerate() {
            if ui.available_height() < 22.0 {
                break;
            }
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.display().to_string());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                glyph(ui.painter(), r, Glyph::Folder, muted());
                if ui.add(egui::Label::new(egui::RichText::new(name).size(14.0)).sense(Sense::click())).on_hover_text(f.display().to_string()).clicked() {
                    let _ = open::that(f);
                }
                if self.folders_edit && ui.add(egui::Button::new(egui::RichText::new("×").size(11.0).color(muted())).frame(false)).clicked() {
                    remove = Some(i);
                }
            });
        }
        if self.folders_edit && link(ui, t("+ Ordner")).clicked() {
            if let Some(dir) = rfd::FileDialog::new().set_title(t("Ordner wählen")).pick_folder() {
                let mut list = folders.clone();
                list.push(dir);
                self.save_folders(list);
            }
        }
        if let Some(i) = remove {
            let mut list = folders;
            list.remove(i);
            self.save_folders(list);
        }
    }

    fn save_folders(&mut self, list: Vec<std::path::PathBuf>) {
        self.cfg.folders = Some(list.clone());
        let mut cfg = core::load_config();
        cfg.folders = Some(list);
        let _ = core::save_config(&cfg);
    }

    fn w_computer(&mut self, ui: &mut egui::Ui) {
        title_row(ui, t("Computer"), None);
        let s = self.sys.lock().map(|s| s.clone()).unwrap_or_default();
        if s.mem_total == 0 {
            ui.spinner();
            return;
        }
        meter(ui, t("Prozessor"), &format!("{:.0} %", s.cpu), s.cpu / 100.0, rgb(0x0F8B8D));
        let frac = s.mem_used as f32 / s.mem_total as f32;
        meter(ui, t("Arbeitsspeicher"), &format!("{} / {}", gb(s.mem_used), gb(s.mem_total)), frac, rgb(0x7B4FD6));
        if let Some(b) = s.battery {
            ui.label(egui::RichText::new(tf("Akku {p} %", &[("p", &b)])).size(12.5).color(muted()));
        }
    }
}

/// «Mi» → «Mittwoch» bzw. «Wednesday».
fn weekday_short_full(d: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    weekday(d.weekday().num_days_from_monday() as usize).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(p: &[Placed], cols: usize) -> Vec<Vec<bool>> {
        let rows = p.iter().map(|x| x.row + x.h).max().unwrap_or(0);
        let mut g = vec![vec![false; cols]; rows];
        for x in p {
            for r in x.row..x.row + x.h {
                for c in x.col..x.col + x.w {
                    assert!(!g[r][c], "überlappt");
                    g[r][c] = true;
                }
            }
        }
        g
    }

    #[test]
    fn ohne_luecken() {
        // Vorgabe: 3 volle Reihen.
        let s: Vec<(usize, usize)> = crate::core::default_widgets().iter().map(|w| (w.w as usize, w.h as usize)).collect();
        let p = pack(&s, 6);
        assert!(covered(&p, 6).iter().all(|r| r.iter().all(|c| *c)));
        // Ungünstige Breiten: Lücken werden aufgefüllt.
        let p = pack(&[(4, 2), (4, 2), (1, 2)], 6);
        let g = covered(&p, 6);
        assert!(g.iter().all(|r| r.iter().all(|c| *c)), "{p:?}");
        // Breiter als das Raster wird gekappt.
        assert_eq!(pack(&[(6, 1)], 2)[0].w, 2);
    }
}
