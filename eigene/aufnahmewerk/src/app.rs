//! Oberfläche (egui), passend zum Mockup: links die Einstellungen, rechts
//! Vorschau, grosser Aufnahmeknopf und letzte Aufnahmen. Der Aufnahme-Motor
//! sitzt in record.rs; hier wird er bedient und der Zustand angezeigt.

use std::time::{Duration, Instant};

use crate::editor::{self, Editor};
use crate::model::{Corner, Mode, Quality, Settings};
use crate::schnapp::{self, Img};
use crate::shot::{self, ShotMode};
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::thema;
use crate::record::{self, Recording};

/// Rot für Aufnahme-Punkt und Auswahlrahmen – in beiden Designs gleich.
fn red() -> egui::Color32 {
    egui::Color32::from_rgb(0xe5, 0x34, 0x2e)
}

enum Phase {
    Idle,
    Countdown { ends: Instant, from: u32 },
    Recording(Recording),
}

/// Oben umschaltbar: Videoaufnahme oder Bildschirmfoto.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Video,
    Foto,
}

/// Bereichswahl auf dem eingefrorenen Bild (randloses Vollbild-Fenster).
struct Overlay {
    img: Img,
    tex: Option<egui::TextureHandle>,
    /// Fensterposition (Punkte) auf dem fotografierten Monitor.
    pos: Option<egui::Pos2>,
    start: Option<egui::Pos2>,
    cur: Option<egui::Pos2>,
    focused: bool,
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
    // Bildschirmfoto
    tab: Tab,
    shot_mode: ShotMode,
    shot_delay: u32,
    shot_jpg: bool,
    shot_job: Option<shot::Slot>,
    shot_hotkey_prev: bool,
    overlay: Option<Overlay>,
    editors: Vec<Editor>,
    next_editor: u64,
    /// Gespeicherte Bildschirmfotos (Name, Pfad, Info).
    shots: Vec<(String, String, String)>,
    thumb: Option<egui::TextureHandle>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0xB2, 0x3A, 0x48]);
        stil::init(&cc.egui_ctx, [0xB2, 0x3A, 0x48]);

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
            tab: Tab::Video,
            shot_mode: ShotMode::Screen,
            shot_delay: 0,
            shot_jpg: false,
            shot_job: None,
            shot_hotkey_prev: false,
            overlay: None,
            editors: Vec::new(),
            next_editor: 1,
            shots: Vec::new(),
            thumb: None,
        }
    }

    /// Läuft gerade ein Bildschirmfoto (Wartezeit oder Bereichswahl)?
    fn shot_busy(&self) -> bool {
        self.shot_job.is_some() || self.overlay.is_some()
    }

    /// Bildschirmfoto auslösen (Knopf oder Tastenkürzel).
    fn start_shot(&mut self, ctx: &egui::Context) {
        if self.shot_busy() {
            return;
        }
        self.last_err = None;
        self.status = if self.shot_delay > 0 {
            tf("Bildschirmfoto in {n} s …", &[("n", &self.shot_delay)])
        } else {
            t("Bildschirmfoto …").to_string()
        };
        self.shot_job = Some(shot::spawn(
            self.shot_mode,
            self.cfg.monitor_index,
            self.shot_delay,
            ctx.clone(),
        ));
    }

    /// Ergebnis des Hintergrund-Threads abholen.
    fn poll_shot(&mut self, ctx: &egui::Context) {
        let res = match &self.shot_job {
            Some(slot) => match slot.lock() {
                Ok(mut g) => g.take(),
                Err(_) => Some(Err(t("Interner Fehler beim Bildschirmfoto.").to_string())),
            },
            None => return,
        };
        let Some(res) = res else {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        self.shot_job = None;
        match res {
            Ok(c) => {
                if c.mode == ShotMode::Region {
                    let ppp = ctx.pixels_per_point().max(0.1);
                    let pos = c.mon_rect.map(|(x, y, w, h)| {
                        egui::pos2(
                            (x as f32 + w as f32 / 2.0 - 120.0) / ppp,
                            (y as f32 + h as f32 / 2.0 - 80.0) / ppp,
                        )
                    });
                    self.status = t("Bereich aufziehen …").to_string();
                    self.overlay = Some(Overlay {
                        img: c.img,
                        tex: None,
                        pos,
                        start: None,
                        cur: None,
                        focused: false,
                    });
                } else {
                    self.open_editor(ctx, c.img);
                }
            }
            Err(e) => {
                self.last_err = Some(tf("Bildschirmfoto fehlgeschlagen: {e}", &[("e", &e)]));
                self.status = t("Bereit").into();
            }
        }
    }

    fn open_editor(&mut self, ctx: &egui::Context, img: Img) {
        self.thumb = Some(ctx.load_texture(
            "letztes-foto",
            editor::color_image(&img),
            egui::TextureOptions::LINEAR,
        ));
        self.status = tf("Bildschirmfoto {w} × {h} im Editor", &[("w", &img.w), ("h", &img.h)]);
        let id = self.next_editor;
        self.next_editor += 1;
        self.editors.push(Editor::new(id, img, self.cfg.out_dir.clone(), self.shot_jpg));
    }

    /// Randloses Vollbild-Fenster mit eingefrorenem Bild; Rahmen aufziehen,
    /// Esc bricht ab.
    fn show_overlay(&mut self, ctx: &egui::Context) {
        let Some(ov) = self.overlay.as_mut() else { return };
        // None = weiter, Some(None) = abgebrochen, Some(Some(bild)) = gewählt
        let mut done: Option<Option<Img>> = None;
        let mut builder = egui::ViewportBuilder::default()
            .with_title(t("Bereich wählen · Aufnahmewerk"))
            .with_decorations(false)
            .with_inner_size([240.0, 160.0])
            .with_fullscreen(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        if let Some(p) = ov.pos {
            builder = builder.with_position(p);
        }
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("aufnahmewerk-bereich"),
            builder,
            |ctx, _class| {
                if !ov.focused {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ov.focused = true;
                }
                if ov.tex.is_none() {
                    let ci = editor::color_image(&ov.img);
                    ov.tex = Some(ctx.load_texture("bereich", ci, egui::TextureOptions::LINEAR));
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()) {
                    done = Some(None);
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(egui::Color32::BLACK))
                    .show(ctx, |ui| {
                        let screen = ctx.screen_rect();
                        let s = schnapp::fit_scale(ov.img.w, ov.img.h, screen.width(), screen.height(), 100.0);
                        let img_rect = egui::Rect::from_center_size(
                            screen.center(),
                            egui::vec2(ov.img.w as f32 * s, ov.img.h as f32 * s),
                        );
                        let resp = ui.allocate_rect(screen, egui::Sense::drag());
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                        let p = ui.painter();
                        if let Some(tex) = &ov.tex {
                            p.image(
                                tex.id(),
                                img_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        }
                        if resp.drag_started() {
                            ov.start = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
                            ov.cur = ov.start;
                        }
                        if resp.dragged() {
                            if let Some(pp) = resp.interact_pointer_pos() {
                                ov.cur = Some(pp);
                            }
                        }
                        // Abdunkeln ausser der Auswahl
                        let veil = egui::Color32::from_black_alpha(120);
                        let sel = match (ov.start, ov.cur) {
                            (Some(a), Some(b)) => Some(egui::Rect::from_two_pos(a, b).intersect(img_rect)),
                            _ => None,
                        };
                        match sel {
                            Some(r) => {
                                let o = screen;
                                p.rect_filled(egui::Rect::from_min_max(o.min, egui::pos2(o.max.x, r.min.y)), 0.0, veil);
                                p.rect_filled(egui::Rect::from_min_max(egui::pos2(o.min.x, r.max.y), o.max), 0.0, veil);
                                p.rect_filled(egui::Rect::from_min_max(egui::pos2(o.min.x, r.min.y), egui::pos2(r.min.x, r.max.y)), 0.0, veil);
                                p.rect_filled(egui::Rect::from_min_max(egui::pos2(r.max.x, r.min.y), egui::pos2(o.max.x, r.max.y)), 0.0, veil);
                                p.rect_stroke(r, 0.0, egui::Stroke::new(2.0, red()));
                                let w = (r.width() / s).round() as u32;
                                let h = (r.height() / s).round() as u32;
                                let label_pos = egui::pos2(r.min.x, (r.min.y - 6.0).max(o.min.y + 18.0));
                                p.text(
                                    label_pos,
                                    egui::Align2::LEFT_BOTTOM,
                                    format!("{w} × {h}"),
                                    egui::FontId::proportional(14.0),
                                    egui::Color32::WHITE,
                                );
                            }
                            None => {
                                p.rect_filled(screen, 0.0, veil);
                            }
                        }
                        // Hinweis oben in der Mitte
                        let hint = t("Bereich mit der Maus aufziehen · Esc bricht ab");
                        let pill = egui::Rect::from_center_size(
                            egui::pos2(screen.center().x, screen.min.y + 34.0),
                            egui::vec2(440.0, 34.0),
                        );
                        p.rect_filled(pill, 17.0, egui::Color32::from_black_alpha(190));
                        p.text(
                            pill.center(),
                            egui::Align2::CENTER_CENTER,
                            hint,
                            egui::FontId::proportional(15.0),
                            egui::Color32::WHITE,
                        );
                        if resp.drag_stopped() {
                            if let (Some(a), Some(b)) = (ov.start, ov.cur) {
                                let ax = (a.x - img_rect.min.x) / s;
                                let ay = (a.y - img_rect.min.y) / s;
                                let bx = (b.x - img_rect.min.x) / s;
                                let by = (b.y - img_rect.min.y) / s;
                                match schnapp::norm_rect(ax, ay, bx, by, ov.img.w, ov.img.h) {
                                    Some(r) if r.2 >= 4 && r.3 >= 4 => {
                                        done = Some(schnapp::crop(&ov.img, r));
                                    }
                                    _ => {
                                        ov.start = None;
                                        ov.cur = None;
                                    }
                                }
                            }
                        }
                    });
            },
        );
        match done {
            Some(Some(img)) => {
                self.overlay = None;
                self.open_editor(ctx, img);
            }
            Some(None) => {
                self.overlay = None;
                self.status = t("Abgebrochen").into();
            }
            None => {}
        }
    }

    /// Alle offenen Editor-Fenster zeigen; geschlossene entfernen.
    fn show_editors(&mut self, ctx: &egui::Context) {
        for ed in self.editors.iter_mut() {
            let builder = egui::ViewportBuilder::default()
                .with_title(ed.title())
                .with_inner_size([1100.0, 760.0])
                .with_min_inner_size([720.0, 480.0]);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("aufnahmewerk-editor", ed.id)),
                builder,
                |ctx, _class| ed.show(ctx),
            );
            for s in ed.saved.drain(..) {
                self.shots.insert(0, s);
            }
        }
        self.shots.truncate(12);
        self.editors.retain(|e| !e.closed);
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
        let hk = hotkey_down(0x52); // 'R'
        if hk && !self.hotkey_prev {
            self.toggle();
        }
        self.hotkey_prev = hk;
        // Bildschirmfoto: Strg+Umschalt+F.
        let hk_shot = hotkey_down(0x46); // 'F'
        if hk_shot && !self.shot_hotkey_prev {
            self.start_shot(ctx);
        }
        self.shot_hotkey_prev = hk_shot;
        self.poll_shot(ctx);

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
        } else {
            // Damit die Tastenkürzel auch im Hintergrund ankommen.
            ctx.request_repaint_after(Duration::from_millis(120));
        }

        self.top_bar(ctx);
        self.status_bar(ctx);
        self.left_panel(ctx);
        self.right_panel(ctx);
        self.show_overlay(ctx);
        self.show_editors(ctx);
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        stil::kopfleiste(ctx, |ui| {
            stil::titel(ui, "Aufnahmewerk");
            stil::grau(ui, t("Bildschirm aufnehmen · Werkbank"));
            stil::trenner(ui);
            ui.allocate_ui(egui::vec2(260.0, 28.0), |ui| {
                stil::wahl(ui, &mut self.tab, &[(Tab::Video, t("Video")), (Tab::Foto, t("Bildschirmfoto"))]);
            });
            stil::rechts(ui, |ui| {
                if stil::leise(ui, true, Sym::Ordner, t("Ordner …"), t("Zielordner wählen")).clicked() {
                    if let Some(d) = rfd::FileDialog::new().set_title(t("Zielordner wählen")).pick_folder() {
                        self.cfg.out_dir = d.display().to_string();
                    }
                }
                stil::grau(ui, &short_dir(&self.cfg.out_dir));
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let recording = self.is_recording();
        let counting = self.is_counting();
        let shot_busy = self.shot_busy();
        let mut do_toggle = false;
        let mut do_shot = false;
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, if recording { stil::rot() } else { stil::gruen() });
                if let Some(e) = &self.last_err {
                    ui.colored_label(stil::rot(), e.as_str());
                } else {
                    stil::grau(ui, self.status.as_str());
                }
                stil::trenner(ui);
                if self.tab == Tab::Foto {
                    let fmt = if self.shot_jpg { "JPG" } else { "PNG" };
                    stil::grau(ui, &tf("Hotkey: Strg+Umschalt+F · {fmt}", &[("fmt", &fmt)]));
                } else {
                    let ton = match (self.cfg.system_audio, self.cfg.microphone) {
                        (true, true) => t("System-Ton + Mikrofon"),
                        (true, false) => t("System-Ton"),
                        (false, true) => t("Mikrofon"),
                        (false, false) => t("ohne Ton"),
                    };
                    stil::grau(ui, &tf("Hotkey: Strg+Umschalt+R · {ton}", &[("ton", &ton)]));
                }
                stil::rechts(ui, |ui| {
                    if self.tab == Tab::Foto {
                        let label = if self.overlay.is_some() {
                            t("Bereich aufziehen …")
                        } else if self.shot_job.is_some() {
                            t("Bildschirmfoto läuft …")
                        } else {
                            t("Bildschirmfoto aufnehmen")
                        };
                        if stil::haupt(ui, !shot_busy, Sym::Kamera, label).on_hover_text(t("oder Strg + Umschalt + F")).clicked() {
                            do_shot = true;
                        }
                    } else {
                        let (sym, label) = if recording {
                            (Sym::Stopp, t("Aufnahme stoppen"))
                        } else if counting {
                            (Sym::Stopp, t("Countdown … (Abbrechen)"))
                        } else {
                            (Sym::Aufnahme, t("Aufnahme starten"))
                        };
                        if stil::haupt(ui, true, sym, label).on_hover_text(t("oder Strg + Umschalt + R")).clicked() {
                            do_toggle = true;
                        }
                    }
                });
            });
        });
        if do_toggle {
            self.toggle();
        }
        if do_shot {
            self.start_shot(ctx);
        }
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("links")
            .resizable(false)
            .exact_width(612.0)
            .frame(egui::Frame::none().fill(thema::bg()).inner_margin(egui::Margin::same(10.0)).stroke(egui::Stroke::new(1.0, thema::border())))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    if self.tab == Tab::Foto {
                        self.card_foto(ui);
                        self.card_foto_format(ui);
                        return;
                    }
                    self.card_modus(ui);
                    self.card_ton(ui);
                    self.card_webcam(ui);
                    self.card_format(ui);
                });
            });
    }

    fn card_modus(&mut self, ui: &mut egui::Ui) {
        let busy = self.is_recording() || self.is_counting();
        stil::karte(ui, "modus", Sym::Aufnahme, t("AUFNAHMEMODUS"), true, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                ui.columns(3, |c| {
                    mode_tile(&mut c[0], &mut self.cfg.mode, Mode::Fullscreen, t("Ganzer Bildschirm"), t("Monitor wählen"));
                    mode_tile(&mut c[1], &mut self.cfg.mode, Mode::Window, t("Einzelnes Fenster"), t("Programm wählen"));
                    mode_tile(&mut c[2], &mut self.cfg.mode, Mode::Region, t("Bereich"), t("Ausschnitt ziehen"));
                });
                ui.add_space(10.0);
                match self.cfg.mode {
                    Mode::Fullscreen => self.monitor_combo(ui, "mon"),
                    Mode::Window => {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("win")
                                .width(ui.available_width() - 140.0)
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
                            if stil::knopf(ui, Sym::Aktualisieren, t("Aktualisieren")).clicked() {
                                self.windows = record::list_windows();
                            }
                        });
                    }
                    Mode::Region => {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(t("Bereich:")).color(thema::muted()));
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

    fn monitor_combo(&mut self, ui: &mut egui::Ui, salt: &str) {
        egui::ComboBox::from_id_salt(salt)
            .width(ui.available_width() - 4.0)
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

    fn card_foto(&mut self, ui: &mut egui::Ui) {
        let busy = self.shot_busy();
        stil::karte(ui, "foto", Sym::Kamera, t("BILDSCHIRMFOTO"), true, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                ui.columns(3, |c| {
                    mode_tile(&mut c[0], &mut self.shot_mode, ShotMode::Screen, t("Ganzer Bildschirm"), t("Monitor wählen"));
                    mode_tile(&mut c[1], &mut self.shot_mode, ShotMode::Region, t("Bereich"), t("Mit der Maus aufziehen"));
                    mode_tile(&mut c[2], &mut self.shot_mode, ShotMode::ActiveWindow, t("Aktives Fenster"), t("Fenster im Vordergrund"));
                });
                ui.add_space(10.0);
                match self.shot_mode {
                    ShotMode::Screen | ShotMode::Region => self.monitor_combo(ui, "foto-mon"),
                    ShotMode::ActiveWindow => {
                        stil::hinweis(ui, t("Fotografiert das Fenster, das vorne ist, sobald Aufnahmewerk ausgeblendet ist. Mit Verzögerung bleibt Zeit, ein anderes Fenster anzuklicken."));
                    }
                }
            });
        });
    }

    fn card_foto_format(&mut self, ui: &mut egui::Ui) {
        stil::karte(ui, "foto-format", Sym::Uhr, t("VOR DEM FOTO · FORMAT"), true, |ui| {
            stil::feld(ui, t("Verzögerung"), |ui| {
                stil::wahl(ui, &mut self.shot_delay, &[(10, "10 s"), (5, "5 s"), (3, "3 s"), (0, t("Aus"))]);
            });
            stil::hinweis(ui, t("Zeit, um Menüs zu öffnen oder Fenster zu wechseln"));
            ui.add_space(8.0);
            stil::feld(ui, t("Format beim Speichern"), |ui| {
                stil::wahl(ui, &mut self.shot_jpg, &[(true, "JPG"), (false, "PNG")]);
            });
            stil::hinweis(ui, t("Strg + S im Editor"));
            ui.add_space(8.0);
            stil::feld(ui, t("Nach dem Foto"), |ui| {
                stil::hinweis(ui, t("Der Editor öffnet sich: Pfeile, Text, Schwärzen, Zuschneiden"));
            });
        });
    }

    fn card_ton(&mut self, ui: &mut egui::Ui) {
        stil::karte(ui, "ton", Sym::Musik, t("TON"), true, |ui| {
            stil::schalter(ui, &mut self.cfg.system_audio, t("System-Ton"));
            stil::hinweis(ui, t("Spiel, Video, Musik vom Rechner"));
            ui.add_space(8.0);
            stil::schalter(ui, &mut self.cfg.microphone, t("Mikrofon"));
            stil::hinweis(ui, t("Deine Stimme für Erklärvideos (Etappe 2)"));
        });
    }

    fn card_webcam(&mut self, ui: &mut egui::Ui) {
        stil::karte(ui, "webcam", Sym::Kamera, t("WEBCAM & ZEIGER"), true, |ui| {
            stil::schalter(ui, &mut self.cfg.webcam, t("Webcam-Overlay"));
            stil::hinweis(ui, t("Kleines Kamerabild in der Ecke"));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Ecke")).size(12.0).color(thema::muted()));
                corner_pick(ui, &mut self.cfg.webcam_corner);
            });
            ui.add_space(8.0);
            stil::schalter(ui, &mut self.cfg.highlight_clicks, t("Mauszeiger & Klicks"));
            stil::hinweis(ui, t("Zeiger hervorheben, Klicks sichtbar"));
        });
    }

    fn card_format(&mut self, ui: &mut egui::Ui) {
        stil::karte(ui, "format", Sym::Zeit, t("VOR DEM START · FORMAT"), true, |ui| {
            stil::feld(ui, t("Countdown"), |ui| {
                stil::wahl(ui, &mut self.cfg.countdown, &[(0, t("Aus")), (3, "3 s"), (5, "5 s")]);
            });
            stil::hinweis(ui, t("3-2-1 vor der Aufnahme"));
            ui.add_space(8.0);
            stil::feld(ui, t("Format · Qualität"), |ui| {
                stil::wahl(ui, &mut self.cfg.quality, &[(Quality::Hoch, t("Hoch")), (Quality::Mittel, t("Mittel")), (Quality::Sparsam, t("Sparsam"))]);
                ui.add_space(4.0);
                stil::wahl(ui, &mut self.cfg.fps, &[(30, "30 fps"), (60, "60 fps")]);
            });
            stil::hinweis(ui, "MP4 · H.264");
        });
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        if self.tab == Tab::Foto {
            self.right_panel_foto(ctx);
            return;
        }
        stil::mitte(ctx, |ui| {
            ui.add_space(4.0);
            // Vorschau-/Zustandsfläche
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 230.0), egui::Sense::hover());
            let p = ui.painter();
            // Bildschirm-Fläche: in beiden Designs dunkel (wie ein Monitor), im dunklen etwas tiefer.
            p.rect_filled(rect, 12.0, thema::pick(egui::Color32::from_rgb(0x1a, 0x1d, 0x22), egui::Color32::from_rgb(0x0c, 0x0d, 0x10)));
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
                    let r = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 14.0), egui::vec2(34.0, 34.0));
                    stil::sym(p, r, Sym::Aufnahme, egui::Color32::from_gray(150));
                    p.text(
                        rect.center() + egui::vec2(0.0, 22.0),
                        egui::Align2::CENTER_CENTER,
                        t("Vorschau"),
                        egui::FontId::proportional(15.0),
                        egui::Color32::from_gray(150),
                    );
                }
            }

            ui.add_space(10.0);
            self.recent_list(ui);
        });
    }

    fn right_panel_foto(&mut self, ctx: &egui::Context) {
        stil::mitte(ctx, |ui| {
            ui.add_space(4.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 230.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 12.0, thema::pick(egui::Color32::from_rgb(0x1a, 0x1d, 0x22), egui::Color32::from_rgb(0x0c, 0x0d, 0x10)));
            if let Some(tex) = &self.thumb {
                let [w, h] = tex.size();
                let inner = rect.shrink(12.0);
                let s = schnapp::fit_scale(w as u32, h as u32, inner.width(), inner.height(), 1.0);
                let r = egui::Rect::from_center_size(inner.center(), egui::vec2(w as f32 * s, h as f32 * s));
                p.image(
                    tex.id(),
                    r,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                let r = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 14.0), egui::vec2(34.0, 34.0));
                stil::sym(p, r, Sym::Kamera, egui::Color32::from_gray(150));
                p.text(
                    rect.center() + egui::vec2(0.0, 22.0),
                    egui::Align2::CENTER_CENTER,
                    t("Letztes Bildschirmfoto"),
                    egui::FontId::proportional(15.0),
                    egui::Color32::from_gray(150),
                );
            }

            ui.add_space(10.0);
            self.shot_list(ui);
        });
    }

    fn shot_list(&mut self, ui: &mut egui::Ui) {
        stil::karte(ui, "fotos", Sym::Bild, t("LETZTE BILDSCHIRMFOTOS"), true, |ui| {
            ui.horizontal(|ui| {
                if stil::knopf(ui, Sym::OrdnerAuf, t("Ordner öffnen")).clicked() {
                    let _ = std::fs::create_dir_all(&self.cfg.out_dir);
                    shot::open_folder(&self.cfg.out_dir);
                }
            });
            ui.add_space(6.0);
            if self.shots.is_empty() {
                stil::grau(ui, t("Noch keine Bildschirmfotos gespeichert."));
                return;
            }
            egui::ScrollArea::vertical().id_salt("fotos").show(ui, |ui| {
                for (name, path, info) in &self.shots {
                    ui.horizontal(|ui| {
                        stil::symbol(ui, Sym::Bild, 18.0, stil::akzent());
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(name.as_str()).strong().size(13.0).color(thema::text()));
                            stil::grau(ui, info.as_str());
                        });
                        stil::rechts(ui, |ui| {
                            if stil::leise(ui, true, Sym::Extern, t("Im Ordner zeigen"), "").clicked() {
                                shot::show_in_folder(path);
                            }
                        });
                    });
                    ui.add_space(4.0);
                }
            });
        });
    }

    fn recent_list(&mut self, ui: &mut egui::Ui) {
        stil::karte(ui, "aufnahmen", Sym::Film, t("LETZTE AUFNAHMEN"), true, |ui| {
            if self.recents.is_empty() {
                ui.add_space(4.0);
                stil::grau(ui, t("Noch keine Aufnahmen in dieser Sitzung."));
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (name, meta) in &self.recents {
                    ui.horizontal(|ui| {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(58.0, 38.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 6.0, egui::Color32::from_rgb(0x2a, 0x30, 0x38));
                        stil::sym(ui.painter(), egui::Rect::from_center_size(r.center(), egui::vec2(18.0, 18.0)), Sym::Play, egui::Color32::from_white_alpha(220));
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(name.as_str()).strong().size(13.0).color(thema::text()));
                            stil::grau(ui, meta.as_str());
                        });
                    });
                    ui.add_space(4.0);
                }
            });
        });
    }
}

// ---------- kleine UI-Bausteine ----------

fn mode_tile<T: PartialEq + Copy>(ui: &mut egui::Ui, cur: &mut T, val: T, title: &str, sub: &str) {
    let sel = *cur == val;
    let fill = if sel { stil::toenung() } else { thema::panel() };
    let stroke = if sel { stil::farbe() } else { thema::border() };
    let resp = egui::Frame::none()
        .fill(fill)
        .rounding(10.0)
        .stroke(egui::Stroke::new(if sel { 1.5 } else { 1.0 }, stroke))
        .inner_margin(egui::Margin::symmetric(8.0, 12.0))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .size(13.5)
                        .strong()
                        .color(if sel { stil::akzent() } else { thema::text() }),
                );
                ui.label(egui::RichText::new(sub).color(thema::muted()).size(11.0));
            });
        })
        .response
        .interact(egui::Sense::click());
    if resp.clicked() {
        *cur = val;
    }
}

fn corner_pick(ui: &mut egui::Ui, cur: &mut Corner) {
    let order = [
        (Corner::ObenLinks, Corner::ObenRechts),
        (Corner::UntenLinks, Corner::UntenRechts),
    ];
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
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
    let stroke = if sel { stil::farbe() } else { thema::border() };
    ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.5, stroke));
    if sel {
        ui.painter().rect_filled(rect.shrink(4.0), 2.0, stil::toenung());
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

// ---------- Plattform-Helfer ----------

/// Strg+Umschalt+<Taste> aktuell gedrückt? (`key` = virtueller Tastencode,
/// z. B. 0x52 für «R», 0x46 für «F».)
fn hotkey_down(key: i32) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let down = |vk: i32| (GetAsyncKeyState(vk) as u16 & 0x8000) != 0;
        down(0x11) && down(0x10) && down(key) // VK_CONTROL, VK_SHIFT, Taste
    }
    #[cfg(not(windows))]
    {
        let _ = key;
        false
    }
}

/// Lokale Zeit (Jahr, Monat, Tag, Stunde, Minute, Sekunde).
pub(crate) fn local_now() -> (i32, u32, u32, u32, u32, u32) {
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

pub(crate) fn default_out_dir() -> String {
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
