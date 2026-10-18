//! Oberfläche (egui), passend zum Mockup: links die gemischte Dateiliste,
//! rechts die Ausgabe-Einstellungen für die gewählte Datei. Umwandeln startet
//! je Datei einen Hintergrund-Auftrag (convert.rs); Fortschritt wird gepollt.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::convert::{self, Job, JobState};
use crate::model::{
    self, Aspect, AudioFmt, AudioTrack, Fit, ImageFmt, MediaKind, Quality, Target, VideoFmt,
};
use crate::sprache::{t, tf};

fn vio() -> egui::Color32 {
    egui::Color32::from_rgb(0x6d, 0x49, 0xe0)
}
fn vio_soft() -> egui::Color32 {
    egui::Color32::from_rgb(0xef, 0xea, 0xfc)
}
fn kind_color(k: MediaKind) -> egui::Color32 {
    match k {
        MediaKind::Audio => egui::Color32::from_rgb(0x16, 0xa3, 0x4a),
        MediaKind::Image => egui::Color32::from_rgb(0x25, 0x63, 0xeb),
        MediaKind::Video => vio(),
    }
}

struct Entry {
    input: PathBuf,
    name: String,
    kind: MediaKind,
    dims: Option<(u32, u32)>,
    target: Target,
    aspect: Aspect,
    fit: Fit,
    quality: Quality,
    job: Option<Arc<JobState>>,
    /// Tonspuren eines Videos (None, solange ffmpeg noch nachschaut oder fehlt).
    tracks: Arc<Mutex<Option<Vec<AudioTrack>>>>,
    /// Gewählte Tonspur für «Nur Ton»; None = Standardspur der Datei.
    audio_track: Option<u32>,
}

impl Entry {
    /// Tonspur, die beim Umwandeln benutzt wird (nur bei mehreren Spuren nötig).
    fn chosen_track(&self) -> Option<u32> {
        let tracks = self.tracks.lock().ok()?.clone()?;
        if tracks.len() < 2 {
            return None;
        }
        self.audio_track.or_else(|| tracks.iter().find(|t| t.default).map(|t| t.index)).or(Some(0))
    }
}

pub struct App {
    entries: Vec<Entry>,
    sel: usize,
    out_dir: String,
    ffmpeg: Option<String>,
    ctx: egui::Context,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        App {
            entries: Vec::new(),
            sel: 0,
            out_dir: default_out_dir(),
            ffmpeg: convert::find_ffmpeg(),
            ctx: cc.egui_ctx.clone(),
        }
    }

    fn add_file(&mut self, p: PathBuf) {
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kind = match model::detect_kind(&ext) {
            Some(k) => k,
            None => return, // nicht unterstützt
        };
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dims = if kind == MediaKind::Image {
            image::image_dimensions(&p).ok()
        } else {
            None
        };
        let tracks: Arc<Mutex<Option<Vec<AudioTrack>>>> = Arc::default();
        if kind == MediaKind::Video {
            if let Some(ff) = self.ffmpeg.clone() {
                let (t, path, ctx) = (tracks.clone(), p.clone(), self.ctx.clone());
                std::thread::spawn(move || {
                    let found = convert::probe_audio(&ff, &path);
                    if let Ok(mut g) = t.lock() {
                        *g = found;
                    }
                    ctx.request_repaint();
                });
            }
        }
        self.entries.push(Entry {
            input: p,
            name,
            kind,
            dims,
            target: model::default_target(kind),
            aspect: Aspect::Original,
            fit: Fit::Crop,
            quality: Quality::Hoch,
            job: None,
            tracks,
            audio_track: None,
        });
    }

    fn apply_to_all(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let s = &self.entries[self.sel];
        let (aspect, fit, quality) = (s.aspect, s.fit, s.quality);
        // Zielformat nur auf gleichartige Dateien übertragen.
        let tgt = s.target;
        for e in self.entries.iter_mut() {
            e.aspect = aspect;
            e.fit = fit;
            e.quality = quality;
            // «Nur Ton» gilt auch für Videos in der Liste.
            let fits = e.kind == tgt.kind() || (tgt.kind() == MediaKind::Audio && e.kind == MediaKind::Video);
            if fits {
                e.target = tgt;
            }
        }
    }

    fn start_all(&mut self) {
        let out_dir = PathBuf::from(&self.out_dir);
        let ffmpeg = self.ffmpeg.clone();
        for e in self.entries.iter_mut() {
            let running = e
                .job
                .as_ref()
                .map(|j| !j.done.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false);
            let done_ok = e
                .job
                .as_ref()
                .map(|j| j.ok.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false);
            if running || done_ok {
                continue;
            }
            let job = Job {
                input: e.input.clone(),
                target: e.target,
                aspect: e.aspect,
                fit: e.fit,
                quality: e.quality,
                out_dir: out_dir.clone(),
                audio_track: if e.target.kind() == MediaKind::Audio { e.chosen_track() } else { None },
            };
            e.job = Some(convert::start_job(job, ffmpeg.clone()));
        }
    }

    fn any_running(&self) -> bool {
        self.entries.iter().any(|e| {
            e.job
                .as_ref()
                .map(|j| j.running.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false)
        })
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Hereingezogene Dateien aufnehmen.
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for p in dropped {
            self.add_file(p);
        }
        if self.any_running() {
            ctx.request_repaint();
        }

        self.top_bar(ctx);
        self.status_bar(ctx);
        self.right_panel(ctx);
        self.left_panel(ctx);
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 7.0, vio());
                paint_convert_logo(ui.painter(), r.center());
                ui.add_space(4.0);
                ui.heading("Wandelwerk");
                ui.add(egui::Label::new(
                    egui::RichText::new(t("Dateien umwandeln · Werkbank")).weak(),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t("Zielordner …")).clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.out_dir = d.display().to_string();
                        }
                    }
                    ui.add(egui::Label::new(
                        egui::RichText::new(short_dir(&self.out_dir)).weak(),
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
                let (col, txt) = if self.ffmpeg.is_some() {
                    let txt = if convert::HAS_EMBEDDED_FFMPEG { t("ffmpeg eingebaut") } else { t("ffmpeg gefunden") };
                    (egui::Color32::from_rgb(0x16, 0xa3, 0x4a), txt)
                } else {
                    (egui::Color32::from_rgb(0xe5, 0x34, 0x2e), t("ffmpeg fehlt (nur Bilder möglich)"))
                };
                ui.painter().circle_filled(dot.center(), 4.0, col);
                ui.weak(txt);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(t("Bilder in Rust · Ton & Video über ffmpeg"));
                });
            });
            ui.add_space(4.0);
        });
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("rechts")
            .resizable(false)
            .exact_width(404.0)
            .show(ctx, |ui| {
                ui.add_space(10.0);
                if self.entries.is_empty() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.weak(t("Noch keine Dateien."));
                        ui.add_space(6.0);
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new(format!("   {}   ", t("Dateien hinzufügen"))).color(egui::Color32::WHITE),
                            ).fill(vio()))
                            .clicked()
                        {
                            self.pick_files();
                        }
                    });
                    return;
                }
                if self.sel >= self.entries.len() {
                    self.sel = 0;
                }
                self.output_card(ui);
                ui.add_space(12.0);
                self.action_card(ui);
            });
    }

    fn output_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            let kind = self.entries[self.sel].kind;
            let name = self.entries[self.sel].name.clone();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("AUSGABE")).size(11.0).strong().color(egui::Color32::from_gray(150)));
                ui.add(egui::Label::new(egui::RichText::new(format!("· {name}")).weak()).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Label::new(egui::RichText::new(t("Für alle übernehmen")).color(vio()).size(12.0)).sense(egui::Sense::click())).clicked() {
                        self.apply_to_all();
                    }
                });
            });

            // Zielformat je nach Typ
            ui.add_space(10.0);
            sub(ui, match kind { MediaKind::Audio=>t("Zielformat (Ton)"), MediaKind::Image=>t("Zielformat (Bild)"), MediaKind::Video=>t("Zielformat (Video)") });
            let e = &mut self.entries[self.sel];
            ui.horizontal_wrapped(|ui| match kind {
                MediaKind::Audio => {
                    for f in AudioFmt::all() {
                        let sel = e.target == Target::Audio(f);
                        if chip(ui, sel, f.label()).clicked() { e.target = Target::Audio(f); }
                    }
                }
                MediaKind::Image => {
                    for f in ImageFmt::all() {
                        let sel = e.target == Target::Image(f);
                        if chip(ui, sel, f.label()).clicked() { e.target = Target::Image(f); }
                    }
                }
                MediaKind::Video => {
                    for f in VideoFmt::all() {
                        let sel = e.target == Target::Video(f);
                        if chip(ui, sel, f.label()).clicked() { e.target = Target::Video(f); }
                    }
                }
            });

            // Video: nur die Tonspur speichern (z. B. MP3 aus einem MKV-Film).
            if kind == MediaKind::Video {
                ui.add_space(12.0);
                sub(ui, t("Nur Ton speichern"));
                let e = &mut self.entries[self.sel];
                ui.horizontal_wrapped(|ui| {
                    for f in AudioFmt::all() {
                        let sel = e.target == Target::Audio(f);
                        if chip(ui, sel, f.label()).clicked() { e.target = Target::Audio(f); }
                    }
                });
                if e.target.kind() == MediaKind::Audio {
                    let tracks = e.tracks.lock().ok().and_then(|g| g.clone());
                    match tracks {
                        Some(tl) if tl.is_empty() => {
                            ui.add_space(6.0);
                            ui.colored_label(egui::Color32::from_rgb(0xe5, 0x34, 0x2e), t("Dieses Video hat keine Tonspur."));
                        }
                        Some(tl) if tl.len() > 1 => {
                            ui.add_space(10.0);
                            sub(ui, t("Tonspur"));
                            let chosen = e.chosen_track();
                            for tr in &tl {
                                if chip(ui, chosen == Some(tr.index), &track_label(tr)).clicked() {
                                    e.audio_track = Some(tr.index);
                                }
                            }
                        }
                        Some(tl) => {
                            ui.add_space(4.0);
                            ui.weak(tf("Tonspur: {spur}", &[("spur", &track_label(&tl[0]))]));
                        }
                        None => {}
                    }
                }
            }

            // Seitenverhältnis + Anpassung nur für Bild/Video
            let kind = self.entries[self.sel].target.kind();
            if kind != MediaKind::Audio {
                ui.add_space(12.0);
                sub(ui, t("Seitenverhältnis"));
                let e = &mut self.entries[self.sel];
                ui.horizontal_wrapped(|ui| {
                    for a in Aspect::all() {
                        if chip(ui, e.aspect == a, a.label()).clicked() { e.aspect = a; }
                    }
                });

                // Vorschau
                let (asp, fit, dims) = {
                    let e = &self.entries[self.sel];
                    (e.aspect, e.fit, e.dims)
                };
                ui.add_space(8.0);
                let (prect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 118.0), egui::Sense::hover());
                draw_preview(ui, prect, dims, asp, fit);

                ui.add_space(10.0);
                sub(ui, t("Anpassung"));
                let e = &mut self.entries[self.sel];
                ui.horizontal_wrapped(|ui| {
                    for f in [Fit::Crop, Fit::Pad, Fit::Stretch] {
                        if chip(ui, e.fit == f, t(f.label())).clicked() { e.fit = f; }
                    }
                });
            }

            // Qualität (nicht bei verlustfreien Formaten wie WAV/FLAC/PNG/BMP/TIFF – trotzdem anzeigen, schadet nicht)
            ui.add_space(12.0);
            sub(ui, t("Qualität"));
            let e = &mut self.entries[self.sel];
            ui.horizontal_wrapped(|ui| {
                for q in [Quality::Hoch, Quality::Mittel, Quality::Klein] {
                    if chip(ui, e.quality == q, t(q.label())).clicked() { e.quality = q; }
                }
            });
        });
    }

    fn action_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            sub(ui, t("Zielordner"));
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(short_dir(&self.out_dir))).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t("Wählen …")).clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.out_dir = d.display().to_string();
                        }
                    }
                });
            });
            ui.add_space(12.0);
            let n = self.entries.len();
            let btn = egui::Button::new(
                egui::RichText::new(tf("{n} Dateien umwandeln", &[("n", &n)])).color(egui::Color32::WHITE).size(15.0),
            )
            .fill(vio())
            .min_size(egui::vec2(ui.available_width(), 46.0));
            if ui.add(btn).clicked() {
                self.start_all();
            }
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t("DATEIEN")).size(11.0).strong().color(egui::Color32::from_gray(150)));
                    ui.weak(format!("{}", self.entries.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t("Leeren")).clicked() {
                            self.entries.clear();
                        }
                        if ui.add(egui::Button::new(egui::RichText::new(t("+ Hinzufügen")).color(egui::Color32::WHITE)).fill(vio())).clicked() {
                            self.pick_files();
                        }
                    });
                });
                ui.add_space(10.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut remove: Option<usize> = None;
                    for i in 0..self.entries.len() {
                        self.file_row(ui, i, &mut remove);
                        ui.add_space(8.0);
                    }
                    if let Some(i) = remove {
                        self.entries.remove(i);
                    }
                    ui.add_space(4.0);
                    ui.weak(t("Dateien hierher ziehen – Ton, Bild oder Video gemischt."));
                });
            });
        });
    }

    fn file_row(&mut self, ui: &mut egui::Ui, i: usize, remove: &mut Option<usize>) {
        let sel = i == self.sel;
        let fill = if sel { vio_soft() } else { egui::Color32::WHITE };
        let stroke = if sel { vio() } else { egui::Color32::from_gray(228) };
        let resp = egui::Frame::none()
            .fill(fill)
            .rounding(11.0)
            .stroke(egui::Stroke::new(1.0, stroke))
            .inner_margin(egui::Margin::symmetric(13.0, 11.0))
            .show(ui, |ui| {
                let e = &self.entries[i];
                ui.horizontal(|ui| {
                    // Typ-Icon
                    let (ic, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
                    ui.painter().rect_filled(ic, 9.0, kind_color(e.kind));
                    ui.painter().text(ic.center(), egui::Align2::CENTER_CENTER,
                        match e.kind { MediaKind::Audio=>t("T"), MediaKind::Image=>t("B"), MediaKind::Video=>t("V") },
                        egui::FontId::proportional(15.0), egui::Color32::WHITE);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(e.name.as_str()).strong().size(13.5)).truncate());
                        ui.weak(source_line(e));
                        if let Some(j) = &e.job {
                            if j.running.load(std::sync::atomic::Ordering::SeqCst) {
                                ui.add(egui::ProgressBar::new(j.percent()).fill(vio()));
                            }
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Label::new(egui::RichText::new("×").size(16.0).weak()).sense(egui::Sense::click())).clicked() {
                            *remove = Some(i);
                        }
                        ui.add_space(6.0);
                        status_badge(ui, e);
                    });
                });
            })
            .response
            .interact(egui::Sense::click());
        if resp.clicked() {
            self.sel = i;
        }
    }

    fn pick_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter(
                t("Medien"),
                &[
                    "mp3", "wav", "flac", "ogg", "m4a", "aac", "opus", "wma", "jpg", "jpeg",
                    "png", "bmp", "gif", "tif", "tiff", "webp", "mp4", "mov", "mkv", "webm",
                    "avi", "wmv", "m4v", "flv", "mpg", "mpeg",
                ],
            )
            .pick_files()
        {
            for f in files {
                self.add_file(f);
            }
        }
    }
}

// ---------- Bausteine ----------

fn card<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) {
    egui::Frame::none()
        .fill(egui::Color32::WHITE)
        .rounding(14.0)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(228)))
        .inner_margin(egui::Margin::symmetric(16.0, 15.0))
        .show(ui, |ui| {
            body(ui);
        });
}

/// Weisses Doppelpfeil-Symbol für das Logo (gezeichnet, ohne Schrift-Glyph).
fn paint_convert_logo(p: &egui::Painter, c: egui::Pos2) {
    let st = egui::Stroke::new(2.0, egui::Color32::WHITE);
    // oberer Pfeil nach rechts
    p.line_segment([egui::pos2(c.x - 6.0, c.y - 3.5), egui::pos2(c.x + 6.0, c.y - 3.5)], st);
    p.line_segment([egui::pos2(c.x + 2.5, c.y - 7.0), egui::pos2(c.x + 6.0, c.y - 3.5)], st);
    p.line_segment([egui::pos2(c.x + 2.5, c.y), egui::pos2(c.x + 6.0, c.y - 3.5)], st);
    // unterer Pfeil nach links
    p.line_segment([egui::pos2(c.x - 6.0, c.y + 3.5), egui::pos2(c.x + 6.0, c.y + 3.5)], st);
    p.line_segment([egui::pos2(c.x - 6.0, c.y + 3.5), egui::pos2(c.x - 2.5, c.y)], st);
    p.line_segment([egui::pos2(c.x - 6.0, c.y + 3.5), egui::pos2(c.x - 2.5, c.y + 7.0)], st);
}

fn sub(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).color(egui::Color32::from_gray(110)));
    ui.add_space(6.0);
}

fn chip(ui: &mut egui::Ui, sel: bool, label: &str) -> egui::Response {
    let col = vio();
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .size(12.5)
                .color(if sel { col } else { egui::Color32::from_gray(80) }),
        )
        .fill(if sel { vio_soft() } else { egui::Color32::WHITE })
        .stroke(egui::Stroke::new(1.0, if sel { col } else { egui::Color32::from_gray(222) })),
    )
}

fn status_badge(ui: &mut egui::Ui, e: &Entry) {
    let badge = format!(
        "{}{}",
        if e.kind == MediaKind::Video && e.target.kind() == MediaKind::Audio {
            tf("{format} (Ton)", &[("format", &e.target.label())])
        } else {
            e.target.label().to_string()
        },
        if e.target.kind() != MediaKind::Audio && e.aspect != Aspect::Original {
            format!(" · {}", e.aspect.label())
        } else {
            String::new()
        }
    );
    match &e.job {
        Some(j) if j.done.load(std::sync::atomic::Ordering::SeqCst) => {
            if j.ok.load(std::sync::atomic::Ordering::SeqCst) {
                ui.colored_label(egui::Color32::from_rgb(0x16, 0xa3, 0x4a), t("fertig"));
            } else {
                let m = j.msg.lock().unwrap().clone();
                ui.colored_label(egui::Color32::from_rgb(0xe5, 0x34, 0x2e), t("Fehler")).on_hover_text(m);
            }
        }
        Some(j) if j.running.load(std::sync::atomic::Ordering::SeqCst) => {
            ui.weak(format!("{} %", (j.percent() * 100.0) as u32));
        }
        _ => {
            ui.label(egui::RichText::new(badge).color(vio()).strong().size(12.0));
        }
    }
}

fn source_line(e: &Entry) -> String {
    let from = e
        .input
        .extension()
        .map(|x| x.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    match e.dims {
        Some((w, h)) => tf("von {from} · {w}×{h}", &[("from", &from), ("w", &w), ("h", &h)]),
        None => tf("von {from}", &[("from", &from)]),
    }
}

/// Deutsche Sprachnamen aus `model::language_name` (zum Übersetzen mit `t`).
const LANG_NAMES: &[&str] = &[
    "Deutsch", "Schweizerdeutsch", "Englisch", "Französisch", "Italienisch", "Spanisch",
    "Portugiesisch", "Niederländisch", "Russisch", "Japanisch", "Koreanisch", "Chinesisch",
    "Türkisch", "Polnisch",
];

/// Wie `AudioTrack::label`, aber in der gewählten Sprache
/// (z. B. «Deutsch · AC3 · 5.1 · Kommentar» bzw. «German · AC3 · 5.1 · Kommentar»).
fn track_label(tr: &AudioTrack) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(match tr.lang.as_deref() {
        Some(code) => {
            let de = model::language_name(code);
            match LANG_NAMES.iter().copied().find(|n| *n == de) {
                Some(n) => t(n).to_string(),
                None => de,
            }
        }
        None => tf("Spur {n}", &[("n", &(tr.index + 1))]),
    });
    if !tr.codec.is_empty() {
        parts.push(tr.codec.clone());
    }
    if !tr.channels.is_empty() {
        parts.push(tr.channels.clone());
    }
    if let Some(title) = &tr.title {
        parts.push(title.clone());
    }
    parts.join(" · ")
}

/// Zeichnet eine kleine Seitenverhältnis-Vorschau (Crop/Pad/Stretch).
fn draw_preview(ui: &mut egui::Ui, area: egui::Rect, dims: Option<(u32, u32)>, aspect: Aspect, fit: Fit) {
    let p = ui.painter();
    p.rect_filled(area, 10.0, egui::Color32::from_gray(247));
    let sr = match dims {
        Some((w, h)) if h > 0 => w as f32 / h as f32,
        _ => 16.0 / 9.0,
    };
    let inner = area.shrink(16.0);
    let src = fit_rect(inner, sr);

    match aspect.ratio() {
        None => {
            p.rect_filled(src, 4.0, vio());
        }
        Some(tr) => {
            let tr = tr as f32;
            match fit {
                Fit::Crop => {
                    p.rect_filled(src, 4.0, vio());
                    let crop = fit_rect_inside(src, tr);
                    // Ränder abdunkeln
                    let dim = egui::Color32::from_black_alpha(110);
                    for r in around(src, crop) {
                        p.rect_filled(r, 0.0, dim);
                    }
                    p.rect_stroke(crop, 2.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                }
                Fit::Pad => {
                    let canvas = fit_rect(inner, tr);
                    p.rect_filled(canvas, 4.0, egui::Color32::from_gray(40));
                    let inside = fit_rect_inside(canvas, sr);
                    p.rect_filled(inside, 2.0, vio());
                }
                Fit::Stretch => {
                    let stretched = fit_rect(inner, tr);
                    p.rect_filled(stretched, 4.0, vio());
                }
            }
        }
    }
}

fn fit_rect(area: egui::Rect, ratio: f32) -> egui::Rect {
    let aw = area.width();
    let ah = area.height();
    let (w, h) = if aw / ah > ratio {
        (ah * ratio, ah)
    } else {
        (aw, aw / ratio)
    };
    egui::Rect::from_center_size(area.center(), egui::vec2(w, h))
}

fn fit_rect_inside(outer: egui::Rect, ratio: f32) -> egui::Rect {
    let ow = outer.width();
    let oh = outer.height();
    let (w, h) = if ow / oh > ratio {
        (oh * ratio, oh)
    } else {
        (ow, ow / ratio)
    };
    egui::Rect::from_center_size(outer.center(), egui::vec2(w, h))
}

/// Die (bis zu vier) Randflächen zwischen `outer` und dem zentrierten `inner`.
fn around(outer: egui::Rect, inner: egui::Rect) -> Vec<egui::Rect> {
    let mut v = Vec::new();
    if inner.top() > outer.top() {
        v.push(egui::Rect::from_min_max(outer.left_top(), egui::pos2(outer.right(), inner.top())));
    }
    if inner.bottom() < outer.bottom() {
        v.push(egui::Rect::from_min_max(egui::pos2(outer.left(), inner.bottom()), outer.right_bottom()));
    }
    if inner.left() > outer.left() {
        v.push(egui::Rect::from_min_max(egui::pos2(outer.left(), inner.top()), egui::pos2(inner.left(), inner.bottom())));
    }
    if inner.right() < outer.right() {
        v.push(egui::Rect::from_min_max(egui::pos2(inner.right(), inner.top()), egui::pos2(outer.right(), inner.bottom())));
    }
    v
}

fn default_out_dir() -> String {
    if let Ok(up) = std::env::var("USERPROFILE") {
        format!("{}\\Downloads\\Umgewandelt", up)
    } else if let Ok(home) = std::env::var("HOME") {
        format!("{}/Umgewandelt", home)
    } else {
        "Umgewandelt".into()
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
