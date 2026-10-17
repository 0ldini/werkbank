//! Oberfläche (egui): links die Liste aller Bilder und Videos, rechts die
//! Einstellungen, unten Ausgabedatei, Start und Fortschritt. Das Rendern läuft
//! in einem Hintergrundthread (render.rs); die Oberfläche fragt den Stand ab.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::model::{self, Item, Kind, OutFmt, Quality, Settings, Transition};
use crate::render::{self, Control};
use crate::tool;

fn vio() -> egui::Color32 {
    egui::Color32::from_rgb(0x6d, 0x49, 0xe0)
}
fn blue() -> egui::Color32 {
    egui::Color32::from_rgb(0x25, 0x63, 0xeb)
}
fn orange() -> egui::Color32 {
    egui::Color32::from_rgb(0xd9, 0x77, 0x06)
}
fn red() -> egui::Color32 {
    egui::Color32::from_rgb(0xdc, 0x26, 0x26)
}

const RES: [(&str, u32, u32); 4] = [
    ("Full HD 1920×1080", 1920, 1080),
    ("4K 3840×2160", 3840, 2160),
    ("HD 1280×720", 1280, 720),
    ("Vorschau 640×360", 640, 360),
];

/// Ergebnis des Ausmessens einer Datei, vom Hilfsthread gemeldet.
enum ProbeMsg {
    One(usize, Result<Item, String>),
    Done,
}

struct Probing {
    rx: Receiver<ProbeMsg>,
    pending: Vec<Option<Result<Item, String>>>,
    got: usize,
}

pub struct App {
    items: Vec<Item>,
    sel: Option<usize>,
    s: Settings,
    res: usize,
    ffmpeg: Option<String>,
    out_path: String,
    recursive: bool,
    probing: Option<Probing>,
    job: Option<Arc<Control>>,
    started: Instant,
    status: String,
    status_ok: bool,
    notes: Vec<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.04;
        }
        cc.egui_ctx.set_style(style);
        let ffmpeg = tool::find_ffmpeg();
        let status = if ffmpeg.is_some() {
            "Bilder und Videos hineinziehen oder oben hinzufügen.".to_string()
        } else {
            "ffmpeg wurde nicht gefunden. ffmpeg.exe neben das Programm legen oder installieren.".to_string()
        };
        App {
            items: Vec::new(),
            sel: None,
            s: Settings::default(),
            res: 0,
            ffmpeg,
            out_path: tool::default_output().display().to_string(),
            recursive: true,
            probing: None,
            job: None,
            started: Instant::now(),
            status,
            status_ok: true,
            notes: Vec::new(),
        }
    }

    // ---------------------------------------------------------- Dateien hinzufügen

    /// Nimmt Dateien und Ordner entgegen; gemessen wird im Hintergrund.
    fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let Some(ff) = self.ffmpeg.clone() else {
            self.status = "ffmpeg fehlt – ohne ffmpeg kann Diawerk nichts einlesen.".into();
            self.status_ok = false;
            return;
        };
        if self.probing.is_some() {
            self.status = "Es wird noch eingelesen – bitte einen Moment warten.".into();
            return;
        }
        let mut files: Vec<PathBuf> = Vec::new();
        for p in paths {
            if p.is_dir() {
                files.extend(model::collect_files(&p, self.recursive));
            } else if model::detect_kind(&p).is_some() {
                files.push(p);
            }
        }
        let known: HashSet<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        files.retain(|p| !known.contains(p) && seen.insert(p.clone()));
        if files.is_empty() {
            self.status = "Keine neuen Bilder oder Videos gefunden.".into();
            return;
        }
        let n = files.len();
        let (tx, rx) = channel::<ProbeMsg>();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let next = AtomicUsize::new(0);
            let workers = 4.min(n).max(1);
            std::thread::scope(|sc| {
                for _ in 0..workers {
                    let (tx, files, ff, next, ctx2) = (tx.clone(), &files, &ff, &next, &ctx2);
                    sc.spawn(move || loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        if i >= files.len() {
                            break;
                        }
                        let r = model::probe(ff, &files[i]).map_err(|e| {
                            format!(
                                "{}: {e}",
                                files[i].file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                            )
                        });
                        let _ = tx.send(ProbeMsg::One(i, r));
                        ctx2.request_repaint();
                    });
                }
            });
            let _ = tx.send(ProbeMsg::Done);
            ctx2.request_repaint();
        });
        self.status = format!("Lese {n} Dateien ein …");
        self.status_ok = true;
        self.probing = Some(Probing { rx, pending: (0..n).map(|_| None).collect(), got: 0 });
    }

    fn poll_probe(&mut self) {
        let mut finished = false;
        if let Some(p) = self.probing.as_mut() {
            while let Ok(msg) = p.rx.try_recv() {
                match msg {
                    ProbeMsg::One(i, r) => {
                        p.pending[i] = Some(r);
                        p.got += 1;
                    }
                    ProbeMsg::Done => finished = true,
                }
            }
            if !finished {
                self.status = format!("Lese ein … {} / {}", p.got, p.pending.len());
            }
        }
        if finished {
            if let Some(p) = self.probing.take() {
                let mut ok = 0;
                let mut bad = Vec::new();
                for r in p.pending.into_iter().flatten() {
                    match r {
                        Ok(it) => {
                            self.items.push(it);
                            ok += 1;
                        }
                        Err(e) => bad.push(e),
                    }
                }
                self.status = format!("{ok} Dateien hinzugefügt.");
                self.status_ok = true;
                if !bad.is_empty() {
                    self.status.push_str(&format!(" {} nicht lesbar (siehe Hinweise).", bad.len()));
                    self.notes.extend(bad.into_iter().map(|b| format!("Nicht lesbar: {b}")));
                }
            }
        }
    }

    // ---------------------------------------------------------- Rendern

    fn start_render(&mut self) {
        let Some(ff) = self.ffmpeg.clone() else { return };
        let mut out = PathBuf::from(self.out_path.trim());
        if out.as_os_str().is_empty() {
            self.status = "Bitte eine Ausgabedatei wählen.".into();
            self.status_ok = false;
            return;
        }
        let ext = self.s.format.ext();
        if out.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some(ext.to_string()) {
            out.set_extension(ext);
            self.out_path = out.display().to_string();
        }
        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() && !parent.is_dir() {
                self.status = "Der Ausgabeordner existiert nicht.".into();
                self.status_ok = false;
                return;
            }
        }
        self.notes.clear();
        self.started = Instant::now();
        self.status = "Diashow wird gerechnet …".into();
        self.status_ok = true;
        self.job = Some(render::start(ff, model::with_credits(&self.items, &self.s), self.s.clone(), out));
    }

    fn poll_job(&mut self) {
        let Some(job) = self.job.clone() else { return };
        if !job.finished.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(mut l) = job.log.lock() {
            self.notes.append(&mut l);
        }
        let res = job.result.lock().ok().and_then(|mut r| r.take());
        let took = self.started.elapsed().as_secs_f32();
        match res {
            Some(Ok(())) => {
                self.status = format!("Fertig nach {} – {}", model::fmt_hms(took), self.out_path);
                self.status_ok = true;
            }
            Some(Err(e)) => {
                self.status = e;
                self.status_ok = false;
            }
            None => {}
        }
        self.job = None;
    }

    // ---------------------------------------------------------- Liste bearbeiten

    fn move_sel(&mut self, up: bool) {
        if let Some(i) = self.sel {
            let j = if up { i.checked_sub(1) } else { Some(i + 1) };
            if let Some(j) = j {
                if j < self.items.len() {
                    self.items.swap(i, j);
                    self.sel = Some(j);
                }
            }
        }
    }

    fn remove_sel(&mut self) {
        if let Some(i) = self.sel {
            if i < self.items.len() {
                self.items.remove(i);
            }
            self.sel = if self.items.is_empty() { None } else { Some(i.min(self.items.len() - 1)) };
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_probe();
        self.poll_job();
        let busy = self.job.is_some() || self.probing.is_some();
        if busy {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        // Dateien/Ordner aufs Fenster ziehen
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() && self.job.is_none() {
            self.add_paths(ctx, dropped);
        }

        let running = self.job.is_some();
        let tl = model::build_timeline(&self.items, &self.s);
        let total_len = model::build_timeline(&model::with_credits(&self.items, &self.s), &self.s).total_secs(self.s.fps);
        let n_vid = self.items.iter().filter(|i| i.kind == Kind::Video).count();
        let n_pano = tl.slots.iter().filter(|s| s.pano).count();
        let n_img = self.items.len() - n_vid - n_pano;

        // ---------------- oben: Liste verwalten
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.heading("Diawerk");
                ui.separator();
                let can_edit = !running && self.probing.is_none();
                if ui.add_enabled(can_edit, egui::Button::new("+ Bilder / Videos …")).clicked() {
                    let mut exts: Vec<&str> = model::IMAGE_EXT.to_vec();
                    exts.extend_from_slice(model::VIDEO_EXT);
                    if let Some(files) = rfd::FileDialog::new()
                        .set_title("Bilder und Videos wählen")
                        .add_filter("Bilder und Videos", &exts)
                        .pick_files()
                    {
                        self.add_paths(ctx, files);
                    }
                }
                if ui.add_enabled(can_edit, egui::Button::new("+ Ordner …")).clicked() {
                    if let Some(dir) = rfd::FileDialog::new().set_title("Ordner wählen").pick_folder() {
                        self.add_paths(ctx, vec![dir]);
                    }
                }
                ui.checkbox(&mut self.recursive, "inkl. Unterordner");
                ui.separator();
                if ui.add_enabled(can_edit && self.items.len() > 1, egui::Button::new("Nach Namen")).clicked() {
                    self.items.sort_by(|a, b| model::natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy()));
                    self.sel = None;
                }
                if ui.add_enabled(can_edit && self.items.len() > 1, egui::Button::new("Nach Datum")).clicked() {
                    self.items.sort_by_key(|i| i.modified);
                    self.sel = None;
                }
                if ui
                    .add_enabled(can_edit && n_vid > 0 && self.items.len() > n_vid, egui::Button::new("Videos einstreuen"))
                    .on_hover_text("Verteilt die Videos gleichmässig zwischen die Bilder")
                    .clicked()
                {
                    let items = std::mem::take(&mut self.items);
                    self.items = model::spread_videos(items);
                    self.sel = None;
                }
                ui.separator();
                let has_sel = can_edit && self.sel.is_some();
                if ui.add_enabled(has_sel, egui::Button::new("↑")).clicked() {
                    self.move_sel(true);
                }
                if ui.add_enabled(has_sel, egui::Button::new("↓")).clicked() {
                    self.move_sel(false);
                }
                if ui.add_enabled(has_sel, egui::Button::new("Entfernen")).clicked() {
                    self.remove_sel();
                }
                if ui.add_enabled(can_edit && !self.items.is_empty(), egui::Button::new("Liste leeren")).clicked() {
                    self.items.clear();
                    self.sel = None;
                }
            });
            ui.add_space(4.0);
        });

        // ---------------- unten: Ausgabe, Start, Fortschritt
        egui::TopBottomPanel::bottom("bottom").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Ausgabe:");
                ui.add_enabled(
                    !running,
                    egui::TextEdit::singleline(&mut self.out_path).desired_width((ui.available_width() - 110.0).max(120.0)),
                );
                if ui.add_enabled(!running, egui::Button::new("Wählen …")).clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .set_title("Diashow speichern unter")
                        .set_file_name(format!("Diashow.{}", self.s.format.ext()))
                        .add_filter("Video", &[self.s.format.ext()])
                        .save_file()
                    {
                        self.out_path = p.display().to_string();
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if let Some(job) = self.job.clone() {
                    if ui.button("Abbrechen").clicked() {
                        job.cancel.store(true, Ordering::SeqCst);
                    }
                    let done = job.done.load(Ordering::Relaxed) as f32;
                    let total = job.total.load(Ordering::Relaxed).max(1) as f32;
                    let frac = (done / total).clamp(0.0, 1.0);
                    let el = self.started.elapsed().as_secs_f32();
                    let eta = if frac > 0.02 { el / frac - el } else { 0.0 };
                    ui.add(egui::ProgressBar::new(frac).show_percentage().desired_width((ui.available_width() - 190.0).max(60.0)));
                    if frac > 0.02 {
                        ui.label(format!("noch ca. {}", model::fmt_hms(eta)));
                    }
                } else {
                    let can_start = self.ffmpeg.is_some() && !self.items.is_empty() && self.probing.is_none();
                    let btn = egui::Button::new(egui::RichText::new("Diashow erstellen").strong().color(egui::Color32::WHITE))
                        .fill(vio());
                    if ui.add_enabled(can_start, btn).clicked() {
                        self.start_render();
                    }
                    if !self.items.is_empty() {
                        ui.label(format!(
                            "{} Bilder · {} Panoramen · {} Videos · Länge ca. {}",
                            n_img,
                            n_pano,
                            n_vid,
                            model::fmt_hms(total_len)
                        ));
                    }
                }
            });
            ui.add_space(2.0);
            let col = if self.status_ok { ui.visuals().text_color() } else { red() };
            ui.label(egui::RichText::new(&self.status).color(col));
            if !self.notes.is_empty() {
                ui.collapsing(format!("Hinweise ({})", self.notes.len()), |ui| {
                    egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                        for n in &self.notes {
                            ui.label(n);
                        }
                    });
                });
            }
            ui.add_space(4.0);
        });

        // ---------------- rechts: Einstellungen
        egui::SidePanel::right("settings").min_width(310.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    ui.add_space(4.0);
                    ui.heading("Film");
                    egui::ComboBox::from_label("Auflösung")
                        .selected_text(RES[self.res].0)
                        .show_ui(ui, |ui| {
                            for (i, r) in RES.iter().enumerate() {
                                ui.selectable_value(&mut self.res, i, r.0);
                            }
                        });
                    self.s.width = RES[self.res].1;
                    self.s.height = RES[self.res].2;
                    egui::ComboBox::from_label("Bilder pro Sekunde")
                        .selected_text(format!("{}", self.s.fps))
                        .show_ui(ui, |ui| {
                            for f in [24u32, 25, 30, 50, 60] {
                                ui.selectable_value(&mut self.s.fps, f, format!("{f}"));
                            }
                        });
                    egui::ComboBox::from_label("Qualität")
                        .selected_text(self.s.quality.label())
                        .show_ui(ui, |ui| {
                            for q in Quality::ALL {
                                ui.selectable_value(&mut self.s.quality, q, q.label());
                            }
                        });
                    {
                        let before = self.s.format;
                        egui::ComboBox::from_label("Format")
                            .selected_text(self.s.format.label())
                            .show_ui(ui, |ui| {
                                for f in OutFmt::ALL {
                                    ui.selectable_value(&mut self.s.format, f, f.label());
                                }
                            });
                        if self.s.format != before {
                            let mut p = PathBuf::from(self.out_path.trim());
                            if !p.as_os_str().is_empty() {
                                p.set_extension(self.s.format.ext());
                                self.out_path = p.display().to_string();
                            }
                        }
                    }
                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Bilder");
                    ui.add(egui::Slider::new(&mut self.s.still_secs, 2.0..=15.0).text("Sekunden pro Bild").suffix(" s"));
                    ui.add(egui::Slider::new(&mut self.s.zoom_pct, 0.0..=10.0).text("sanfter Zoom").suffix(" %"));
                    ui.checkbox(&mut self.s.crop_to_fill, "Bilder nahe 16:9 füllen den Schirm");
                    ui.small("Aus: nichts wird abgeschnitten. Hoch- und 4:3-Bilder stehen immer ganz im Bild, vor unscharfem Hintergrund.");

                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Panoramen");
                    ui.add(egui::Slider::new(&mut self.s.pano_ratio, 1.8..=4.0).text("ab Seitenverhältnis"));
                    ui.label(
                        egui::RichText::new(format!("{n_pano} Panoramen erkannt"))
                            .color(if n_pano > 0 { orange() } else { ui.visuals().weak_text_color() }),
                    );
                    ui.add(egui::Slider::new(&mut self.s.pano_speed, 60.0..=500.0).text("Tempo").suffix(" px/s"));
                    ui.add(egui::Slider::new(&mut self.s.pano_min_secs, 3.0..=20.0).text("mindestens").suffix(" s"));
                    ui.add(egui::Slider::new(&mut self.s.pano_max_secs, 8.0..=60.0).text("höchstens").suffix(" s"));
                    ui.add(egui::Slider::new(&mut self.s.pano_pause, 0.0..=2.0).text("Ruhe am Rand").suffix(" s"));
                    ui.small("Panoramen fahren von links nach rechts und bleiben länger stehen. Tempo gilt bei 1080 Zeilen Höhe.");

                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Übergänge");
                    egui::ComboBox::from_label("Art")
                        .selected_text(self.s.transition.label())
                        .show_ui(ui, |ui| {
                            for t in Transition::ALL {
                                ui.selectable_value(&mut self.s.transition, t, t.label());
                            }
                        });
                    ui.add(egui::Slider::new(&mut self.s.trans_secs, 0.0..=3.0).text("Dauer").suffix(" s"));
                    ui.small("Vor und nach Panoramen wird immer weich überblendet. Am Anfang und Ende blendet der Film aus Schwarz ein und aus.");

                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Videos");
                    ui.add(egui::Slider::new(&mut self.s.video_max_secs, 0.0..=300.0).text("längstens").suffix(" s"));
                    ui.small("0 = ganze Länge. Der Ton der Videos wird weggelassen.");

                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Musik");
                    if self.s.music.is_empty() {
                        ui.label("keine");
                    }
                    let mut remove: Option<usize> = None;
                    let mut up: Option<usize> = None;
                    for (i, m) in self.s.music.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{}. {}",
                                i + 1,
                                m.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                            ));
                            if i > 0 && ui.small_button("↑").clicked() {
                                up = Some(i);
                            }
                            if ui.small_button("✕").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = up {
                        self.s.music.swap(i, i - 1);
                    }
                    if let Some(i) = remove {
                        self.s.music.remove(i);
                    }
                    if ui.button("MP3 hinzufügen …").clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .set_title("Musik wählen (mehrere möglich)")
                            .add_filter("Musik", &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus"])
                            .pick_files()
                        {
                            self.s.music.extend(files);
                        }
                    }
                    ui.small("Die Titel laufen der Reihe nach, werden bei Bedarf wiederholt, blenden ein und am Ende aus.");

                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Vor- und Abspann");
                    ui.checkbox(&mut self.s.credits.intro, "Vorspann mit dem Fotografen");
                    ui.checkbox(&mut self.s.credits.outro, "Abspann mit Dank");
                    ui.add(egui::Slider::new(&mut self.s.credits.card_secs, 3.0..=20.0).text("Dauer").suffix(" s"));
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.s.credits.name);
                    ui.label("Titelzeile (Vorspann)");
                    ui.text_edit_singleline(&mut self.s.credits.intro_kicker);
                    ui.label("Text (Vorspann)");
                    ui.text_edit_multiline(&mut self.s.credits.intro_text);
                    ui.label("Titelzeile (Abspann)");
                    ui.text_edit_singleline(&mut self.s.credits.outro_kicker);
                    ui.label("Text (Abspann)");
                    ui.text_edit_multiline(&mut self.s.credits.outro_text);
                    match self.s.credits.portrait.clone() {
                        Some(p) => {
                            ui.label(format!(
                                "Porträt: {}",
                                p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                            ));
                            if ui.button("Porträt entfernen").clicked() {
                                self.s.credits.portrait = None;
                            }
                        }
                        None => {
                            ui.label("Porträt: keines");
                        }
                    }
                    if ui.button("Porträt wählen …").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Porträt des Fotografen")
                            .add_filter("Bilder", &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff"])
                            .pick_file()
                        {
                            self.s.credits.portrait = Some(p);
                        }
                    }
                    if model::find_font(self.s.credits.font.as_deref()).is_none() {
                        ui.colored_label(red(), "Keine Systemschrift gefunden – bitte eine Schrift wählen.");
                    }
                    if ui.button("Schrift wählen … (optional)").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Schriftdatei")
                            .add_filter("Schrift", &["ttf", "otf"])
                            .pick_file()
                        {
                            self.s.credits.font = Some(p);
                        }
                    }
                    ui.small("Layout wie die Webseite des Fotografen: links Titel, Name und Text, rechts das Porträt.");
                });
            });
        });

        // ---------------- Mitte: die Liste
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.items.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.heading("Noch keine Bilder");
                    ui.label("Ordner oder Dateien hierher ziehen – oder oben hinzufügen.");
                    ui.label("Videos (MP4, MOV …) kommen in die Liste wie Bilder.");
                });
                return;
            }
            let row_h = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y + 4.0;
            let mut new_sel = self.sel;
            let items = &self.items;
            let slots = &tl.slots;
            let fps = self.s.fps as f32;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, row_h, items.len(), |ui, range| {
                    for i in range {
                        let it = &items[i];
                        let sl = &slots[i];
                        let (tag, col) = match (it.kind, sl.pano) {
                            (Kind::Video, _) => ("VIDEO   ", vio()),
                            (Kind::Intro | Kind::Outro, _) => ("KARTE   ", vio()),
                            (_, true) => ("PANORAMA", orange()),
                            _ => ("BILD    ", blue()),
                        };
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{:>4}", i + 1)).monospace().weak());
                            ui.label(egui::RichText::new(tag).monospace().strong().color(col));
                            let text = format!(
                                "{}    {}×{}    {:.1} s",
                                it.name,
                                it.w,
                                it.h,
                                sl.frames as f32 / fps
                            );
                            if ui.selectable_label(new_sel == Some(i), text).clicked() {
                                new_sel = Some(i);
                            }
                        });
                    }
                });
            self.sel = new_sel;
        });
    }
}
