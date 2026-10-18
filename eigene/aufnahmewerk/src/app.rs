//! Oberfläche (egui), passend zum Mockup: links die Einstellungen, rechts
//! Vorschau, grosser Aufnahmeknopf und letzte Aufnahmen. Der Aufnahme-Motor
//! sitzt in record.rs; hier wird er bedient und der Zustand angezeigt.

use std::time::Instant;

use crate::model::{Corner, Mode, Quality, Settings};
use crate::sprache::{t, tf};
use crate::record::{self, Recording};

fn red() -> egui::Color32 {
    egui::Color32::from_rgb(0xe5, 0x34, 0x2e)
}
fn red_soft() -> egui::Color32 {
    egui::Color32::from_rgb(0xfd, 0xec, 0xec)
}
fn teal() -> egui::Color32 {
    egui::Color32::from_rgb(0x0f, 0x9f, 0x8a)
}

enum Phase {
    Idle,
    Countdown { ends: Instant, from: u32 },
    Recording(Recording),
}

pub struct App {
    cfg: Settings,
    phase: Phase,
    monitors: Vec<String>,
    windows: Vec<String>,
    recents: Vec<(String, String)>,
    status: String,
    last_err: Option<String>,
    hotkey_prev: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);

        let mut cfg = Settings::default();
        cfg.out_dir = default_out_dir();
        App {
            cfg,
            phase: Phase::Idle,
            monitors: record::list_monitors(),
            windows: Vec::new(),
            recents: Vec::new(),
            status: t("Bereit").into(),
            last_err: None,
            hotkey_prev: false,
        }
    }

    fn is_recording(&self) -> bool {
        matches!(self.phase, Phase::Recording(_))
    }
    fn is_counting(&self) -> bool {
        matches!(self.phase, Phase::Countdown { .. })
    }

    fn toggle(&mut self) {
        match self.phase {
            Phase::Recording(_) => {
                if let Phase::Recording(rec) = &mut self.phase {
                    rec.stop();
                }
                let secs = if let Phase::Recording(rec) = &self.phase {
                    rec.seconds()
                } else {
                    0
                };
                self.finish_to_recent(secs);
                self.phase = Phase::Idle;
                self.status = t("Gespeichert").into();
            }
            Phase::Countdown { .. } => {
                self.phase = Phase::Idle;
                self.status = t("Abgebrochen").into();
            }
            Phase::Idle => {
                if self.cfg.countdown > 0 {
                    self.phase = Phase::Countdown {
                        ends: Instant::now()
                            + std::time::Duration::from_secs(self.cfg.countdown as u64),
                        from: self.cfg.countdown,
                    };
                } else {
                    self.begin();
                }
            }
        }
    }

    fn begin(&mut self) {
        self.last_err = None;
        let dims = match record::source_dims(&self.cfg) {
            Ok(d) => d,
            Err(e) => {
                self.last_err = Some(tf("Quelle nicht verfügbar: {e}", &[("e", &e)]));
                self.phase = Phase::Idle;
                return;
            }
        };
        let bitrate = crate::model::pick_bitrate(dims.0, dims.1, self.cfg.fps, self.cfg.quality);
        let (y, mo, d, h, mi, s) = local_now();
        let name = crate::model::output_filename(y, mo, d, h, mi, s);
        let path = join_path(&self.cfg.out_dir, &name);
        match record::start(&self.cfg, dims, bitrate, path) {
            Ok(rec) => {
                self.phase = Phase::Recording(rec);
                self.status = t("Aufnahme läuft").into();
            }
            Err(e) => {
                self.last_err = Some(tf("Start fehlgeschlagen: {e}", &[("e", &e)]));
                self.phase = Phase::Idle;
            }
        }
    }

    fn finish_to_recent(&mut self, secs: u64) {
        let (y, mo, d, h, mi, s) = local_now();
        let name = crate::model::output_filename(y, mo, d, h, mi, s);
        let meta = format!(
            "{} · {}",
            crate::model::fmt_timecode(secs),
            match self.cfg.mode {
                Mode::Fullscreen => t("Vollbild").to_string(),
                Mode::Window => t("Fenster").to_string(),
                Mode::Region => tf(
                    "Bereich {w}×{h}",
                    &[("w", &self.cfg.region.2), ("h", &self.cfg.region.3)],
                ),
            }
        );
        self.recents.insert(0, (name, meta));
        self.recents.truncate(8);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Global-Hotkey (Strg+Umschalt+R) per Flankenerkennung.
        let hk = hotkey_down();
        if hk && !self.hotkey_prev {
            self.toggle();
        }
        self.hotkey_prev = hk;

        // Countdown abgelaufen -> starten.
        let start_now = match &self.phase {
            Phase::Countdown { ends, .. } => Instant::now() >= *ends,
            _ => false,
        };
        if start_now {
            self.begin();
        }
        // Laufende Aufnahme überwachen (fertig -> in die Liste).
        let finished_secs = match &self.phase {
            Phase::Recording(rec) if rec.is_finished() => Some(rec.seconds()),
            _ => None,
        };
        if let Some(secs) = finished_secs {
            self.finish_to_recent(secs);
            self.phase = Phase::Idle;
            self.status = t("Gespeichert").into();
        }
        if self.is_recording() || self.is_counting() {
            ctx.request_repaint();
        }

        self.top_bar(ctx);
        self.status_bar(ctx);
        self.left_panel(ctx);
        self.right_panel(ctx);
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 7.0, red());
                ui.painter().circle_filled(r.center(), 5.0, egui::Color32::WHITE);
                ui.add_space(4.0);
                ui.heading("Aufnahmewerk");
                ui.add(egui::Label::new(
                    egui::RichText::new(t("Bildschirm aufnehmen · Werkbank")).weak(),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t("Ordner …")).clicked() {
                        if let Some(d) = rfd::FileDialog::new()
                            .set_title(t("Zielordner wählen"))
                            .pick_folder()
                        {
                            self.cfg.out_dir = d.display().to_string();
                        }
                    }
                    ui.add(egui::Label::new(
                        egui::RichText::new(short_dir(&self.cfg.out_dir)).weak(),
                    ));
                });
            });
            ui.add_space(8.0);
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                let col = if self.is_recording() { red() } else { teal() };
                ui.painter().circle_filled(dot.center(), 4.0, col);
                if let Some(e) = &self.last_err {
                    ui.colored_label(red(), e.as_str());
                } else {
                    ui.weak(self.status.as_str());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ton = match (self.cfg.system_audio, self.cfg.microphone) {
                        (true, true) => t("System-Ton + Mikrofon"),
                        (true, false) => t("System-Ton"),
                        (false, true) => t("Mikrofon"),
                        (false, false) => t("ohne Ton"),
                    };
                    ui.weak(tf("Hotkey: Strg+Umschalt+R · {ton}", &[("ton", &ton)]));
                });
            });
            ui.add_space(4.0);
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("links")
            .resizable(false)
            .exact_width(612.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(10.0);
                    self.card_modus(ui);
                    ui.add_space(12.0);
                    self.card_ton(ui);
                    ui.add_space(12.0);
                    self.card_webcam(ui);
                    ui.add_space(12.0);
                    self.card_format(ui);
                    ui.add_space(10.0);
                });
            });
    }

    fn card_modus(&mut self, ui: &mut egui::Ui) {
        card(ui, t("AUFNAHMEMODUS"), |ui| {
            let busy = self.is_recording() || self.is_counting();
            ui.add_enabled_ui(!busy, |ui| {
                ui.columns(3, |c| {
                    mode_tile(&mut c[0], &mut self.cfg.mode, Mode::Fullscreen, t("Ganzer Bildschirm"), t("Monitor wählen"));
                    mode_tile(&mut c[1], &mut self.cfg.mode, Mode::Window, t("Einzelnes Fenster"), t("Programm wählen"));
                    mode_tile(&mut c[2], &mut self.cfg.mode, Mode::Region, t("Bereich"), t("Ausschnitt ziehen"));
                });
                ui.add_space(10.0);
                match self.cfg.mode {
                    Mode::Fullscreen => {
                        egui::ComboBox::from_id_salt("mon")
                            .width(ui.available_width())
                            .selected_text(
                                self.monitors
                                    .get(self.cfg.monitor_index)
                                    .cloned()
                                    .unwrap_or_else(|| t("Monitor 1").into()),
                            )
                            .show_ui(ui, |ui| {
                                for (i, m) in self.monitors.iter().enumerate() {
                                    ui.selectable_value(&mut self.cfg.monitor_index, i, m.as_str());
                                }
                            });
                    }
                    Mode::Window => {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("win")
                                .width(ui.available_width() - 90.0)
                                .selected_text(if self.cfg.window_title.is_empty() {
                                    t("Fenster wählen …").to_string()
                                } else {
                                    self.cfg.window_title.clone()
                                })
                                .show_ui(ui, |ui| {
                                    for w in &self.windows {
                                        ui.selectable_value(
                                            &mut self.cfg.window_title,
                                            w.clone(),
                                            w.as_str(),
                                        );
                                    }
                                });
                            if ui.button(t("Aktualisieren")).clicked() {
                                self.windows = record::list_windows();
                            }
                        });
                    }
                    Mode::Region => {
                        ui.horizontal(|ui| {
                            ui.label(t("Bereich:"));
                            drag_u32(ui, &mut self.cfg.region.2, t("Breite"));
                            ui.label("×");
                            drag_u32(ui, &mut self.cfg.region.3, t("Höhe"));
                            ui.label(t("ab"));
                            drag_i32(ui, &mut self.cfg.region.0, "X");
                            ui.label(",");
                            drag_i32(ui, &mut self.cfg.region.1, "Y");
                        });
                    }
                }
            });
        });
    }

    fn card_ton(&mut self, ui: &mut egui::Ui) {
        card(ui, t("TON"), |ui| {
            row(ui, t("System-Ton"), t("Spiel, Video, Musik vom Rechner"), |ui| {
                toggle(ui, &mut self.cfg.system_audio, teal());
            });
            ui.add_space(8.0);
            row(ui, t("Mikrofon"), t("Deine Stimme für Erklärvideos (Etappe 2)"), |ui| {
                toggle(ui, &mut self.cfg.microphone, teal());
            });
        });
    }

    fn card_webcam(&mut self, ui: &mut egui::Ui) {
        card(ui, t("WEBCAM & ZEIGER"), |ui| {
            row(ui, t("Webcam-Overlay"), t("Kleines Kamerabild in der Ecke"), |ui| {
                corner_pick(ui, &mut self.cfg.webcam_corner);
                ui.add_space(8.0);
                toggle(ui, &mut self.cfg.webcam, red());
            });
            ui.add_space(8.0);
            row(ui, t("Mauszeiger & Klicks"), t("Zeiger hervorheben, Klicks sichtbar"), |ui| {
                toggle(ui, &mut self.cfg.highlight_clicks, red());
            });
        });
    }

    fn card_format(&mut self, ui: &mut egui::Ui) {
        card(ui, t("VOR DEM START · FORMAT"), |ui| {
            row(ui, t("Countdown"), t("3-2-1 vor der Aufnahme"), |ui| {
                chip_u32(ui, &mut self.cfg.countdown, 0, t("Aus"), red());
                chip_u32(ui, &mut self.cfg.countdown, 3, "3 s", red());
                chip_u32(ui, &mut self.cfg.countdown, 5, "5 s", red());
            });
            ui.add_space(8.0);
            row(ui, t("Format · Qualität"), "MP4 · H.264", |ui| {
                chip_q(ui, &mut self.cfg.quality, Quality::Hoch, t("Hoch"));
                chip_q(ui, &mut self.cfg.quality, Quality::Mittel, t("Mittel"));
                chip_q(ui, &mut self.cfg.quality, Quality::Sparsam, t("Sparsam"));
                ui.add_space(8.0);
                chip_u32(ui, &mut self.cfg.fps, 30, "30 fps", teal());
                chip_u32(ui, &mut self.cfg.fps, 60, "60 fps", teal());
            });
        });
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            // Vorschau-/Zustandsfläche
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 230.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 14.0, egui::Color32::from_rgb(0x1a, 0x1d, 0x22));
            match &self.phase {
                Phase::Countdown { ends, from } => {
                    let left = ends.saturating_duration_since(Instant::now()).as_secs() + 1;
                    let n = left.min(*from as u64);
                    p.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{}", n),
                        egui::FontId::proportional(96.0),
                        egui::Color32::WHITE,
                    );
                }
                Phase::Recording(rec) => {
                    let pill = egui::Rect::from_min_size(
                        rect.left_top() + egui::vec2(12.0, 12.0),
                        egui::vec2(150.0, 28.0),
                    );
                    p.rect_filled(pill, 14.0, egui::Color32::from_black_alpha(160));
                    p.circle_filled(pill.left_center() + egui::vec2(16.0, 0.0), 5.0, red());
                    p.text(
                        pill.left_center() + egui::vec2(28.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("REC · {}", crate::model::fmt_timecode(rec.seconds())),
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
                }
                Phase::Idle => {
                    p.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        t("Vorschau"),
                        egui::FontId::proportional(15.0),
                        egui::Color32::from_gray(150),
                    );
                }
            }

            ui.add_space(14.0);
            self.record_button(ui);
            ui.add_space(14.0);
            self.recent_list(ui);
        });
    }

    fn record_button(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(ui.visuals().extreme_bg_color)
            .rounding(14.0)
            .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (c, resp) = ui.allocate_exact_size(
                        egui::vec2(56.0, 56.0),
                        egui::Sense::click(),
                    );
                    let recording = self.is_recording() || self.is_counting();
                    ui.painter().circle_filled(c.center(), 28.0, red());
                    if recording {
                        // Stopp-Quadrat
                        let s = egui::Rect::from_center_size(c.center(), egui::vec2(20.0, 20.0));
                        ui.painter().rect_filled(s, 3.0, egui::Color32::WHITE);
                    } else {
                        ui.painter().circle_filled(c.center(), 11.0, egui::Color32::WHITE);
                    }
                    if resp.clicked() {
                        self.toggle();
                    }
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        let label = if self.is_recording() {
                            t("Aufnahme stoppen")
                        } else if self.is_counting() {
                            t("Countdown … (Abbrechen)")
                        } else {
                            t("Aufnahme starten")
                        };
                        ui.label(egui::RichText::new(label).size(17.0).strong());
                        ui.label(
                            egui::RichText::new(t("oder Strg + Umschalt + R"))
                                .weak()
                                .size(12.0),
                        );
                    });
                });
            });
    }

    fn recent_list(&mut self, ui: &mut egui::Ui) {
        card(ui, t("LETZTE AUFNAHMEN"), |ui| {
            if self.recents.is_empty() {
                ui.add_space(4.0);
                ui.weak(t("Noch keine Aufnahmen in dieser Sitzung."));
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (name, meta) in &self.recents {
                    ui.horizontal(|ui| {
                        let (t, _) =
                            ui.allocate_exact_size(egui::vec2(58.0, 38.0), egui::Sense::hover());
                        ui.painter().rect_filled(t, 6.0, egui::Color32::from_rgb(0x2a, 0x30, 0x38));
                        ui.painter().circle_filled(t.center(), 7.0, egui::Color32::from_white_alpha(220));
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(name.as_str()).strong().size(13.0));
                            ui.weak(meta.as_str());
                        });
                    });
                    ui.add_space(4.0);
                }
            });
        });
    }
}

// ---------- kleine UI-Bausteine ----------

fn card<R>(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui) -> R) {
    egui::Frame::none()
        .fill(egui::Color32::WHITE)
        .rounding(14.0)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(228)))
        .inner_margin(egui::Margin::symmetric(16.0, 15.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(11.0)
                    .strong()
                    .color(egui::Color32::from_gray(150)),
            );
            ui.add_space(10.0);
            body(ui);
        });
}

fn row<R>(ui: &mut egui::Ui, name: &str, desc: &str, right: impl FnOnce(&mut egui::Ui) -> R) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(name).size(14.0).strong());
            ui.label(egui::RichText::new(desc).weak().size(11.5));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

fn mode_tile(ui: &mut egui::Ui, cur: &mut Mode, val: Mode, title: &str, sub: &str) {
    let sel = *cur == val;
    let fill = if sel { red_soft() } else { egui::Color32::WHITE };
    let stroke = if sel { red() } else { egui::Color32::from_gray(222) };
    let resp = egui::Frame::none()
        .fill(fill)
        .rounding(11.0)
        .stroke(egui::Stroke::new(1.5, stroke))
        .inner_margin(egui::Margin::symmetric(8.0, 12.0))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .size(13.5)
                        .strong()
                        .color(if sel { red() } else { egui::Color32::from_gray(40) }),
                );
                ui.label(egui::RichText::new(sub).weak().size(11.0));
            });
        })
        .response
        .interact(egui::Sense::click());
    if resp.clicked() {
        *cur = val;
    }
}

fn toggle(ui: &mut egui::Ui, on: &mut bool, col: egui::Color32) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 23.0), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let bg = if *on { col } else { egui::Color32::from_gray(210) };
    ui.painter().rect_filled(rect, 11.5, bg);
    let cx = if *on { rect.right() - 11.5 } else { rect.left() + 11.5 };
    ui.painter()
        .circle_filled(egui::pos2(cx, rect.center().y), 9.5, egui::Color32::WHITE);
}

fn corner_pick(ui: &mut egui::Ui, cur: &mut Corner) {
    let order = [
        (Corner::ObenLinks, Corner::ObenRechts),
        (Corner::UntenLinks, Corner::UntenRechts),
    ];
    ui.vertical(|ui| {
        for (a, b) in order {
            ui.horizontal(|ui| {
                corner_cell(ui, cur, a);
                corner_cell(ui, cur, b);
            });
        }
    });
}

fn corner_cell(ui: &mut egui::Ui, cur: &mut Corner, val: Corner) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    if resp.clicked() {
        *cur = val;
    }
    let sel = *cur == val;
    let stroke = if sel { red() } else { egui::Color32::from_gray(205) };
    ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.5, stroke));
    if sel {
        ui.painter().rect_filled(rect.shrink(4.0), 2.0, red_soft());
    }
}

fn chip_u32(ui: &mut egui::Ui, cur: &mut u32, val: u32, label: &str, col: egui::Color32) {
    let sel = *cur == val;
    let resp = ui.add(egui::Button::new(
        egui::RichText::new(label)
            .size(12.5)
            .color(if sel { col } else { egui::Color32::from_gray(80) }),
    ).fill(if sel { tint(col) } else { egui::Color32::WHITE })
     .stroke(egui::Stroke::new(1.0, if sel { col } else { egui::Color32::from_gray(222) })));
    if resp.clicked() {
        *cur = val;
    }
}

fn chip_q(ui: &mut egui::Ui, cur: &mut Quality, val: Quality, label: &str) {
    let sel = *cur == val;
    let resp = ui.add(egui::Button::new(
        egui::RichText::new(label)
            .size(12.5)
            .color(if sel { red() } else { egui::Color32::from_gray(80) }),
    ).fill(if sel { red_soft() } else { egui::Color32::WHITE })
     .stroke(egui::Stroke::new(1.0, if sel { red() } else { egui::Color32::from_gray(222) })));
    if resp.clicked() {
        *cur = val;
    }
}

fn drag_u32(ui: &mut egui::Ui, v: &mut u32, tip: &str) {
    ui.add(egui::DragValue::new(v).range(2..=16384).speed(2.0))
        .on_hover_text(tip);
}
fn drag_i32(ui: &mut egui::Ui, v: &mut i32, tip: &str) {
    ui.add(egui::DragValue::new(v).range(0..=16384).speed(2.0))
        .on_hover_text(tip);
}

fn tint(c: egui::Color32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 28)
}

// ---------- Plattform-Helfer ----------

/// Strg+Umschalt+R aktuell gedrückt?
fn hotkey_down() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let down = |vk: i32| (GetAsyncKeyState(vk) as u16 & 0x8000) != 0;
        down(0x11) && down(0x10) && down(0x52) // VK_CONTROL, VK_SHIFT, 'R'
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Lokale Zeit (Jahr, Monat, Tag, Stunde, Minute, Sekunde).
fn local_now() -> (i32, u32, u32, u32, u32, u32) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let s = GetLocalTime();
        (
            s.wYear as i32,
            s.wMonth as u32,
            s.wDay as u32,
            s.wHour as u32,
            s.wMinute as u32,
            s.wSecond as u32,
        )
    }
    #[cfg(not(windows))]
    {
        (2026, 1, 1, 0, 0, 0)
    }
}

fn default_out_dir() -> String {
    if let Ok(up) = std::env::var("USERPROFILE") {
        format!("{}\\Videos\\Aufnahmen", up)
    } else {
        "Aufnahmen".into()
    }
}

fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('\\') || dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}\\{}", dir, name)
    }
}

fn short_dir(dir: &str) -> String {
    let parts: Vec<&str> = dir.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => "—".into(),
        1 => parts[0].to_string(),
        n => format!("{}\\{}", parts[n - 2], parts[n - 1]),
    }
}
