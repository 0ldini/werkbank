//! Oberfläche (egui): oben Ordnerwahl, links der Grössenbaum, rechts die
//! grössten Dateien und Dateitypen. Der Scan läuft in einem Hintergrund-Thread,
//! damit das Fenster flüssig bleibt; Fortschritt über Atomics.

use crate::scan::{self, fmt_size, group_thousands, Node};
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
use crate::thema;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Instant;

/// Türkis für Schrift, Balken und Linien – je Design lesbar auf dem Hintergrund.
fn teal() -> egui::Color32 {
    stil::akzent()
}

struct Prog {
    files: Arc<AtomicU64>,
    dirs: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}

pub struct App {
    root: Option<PathBuf>,
    tree: Option<Node>,
    largest: Vec<(String, u64)>,
    exts: Vec<(String, u64, u64)>,
    expanded: HashSet<String>,
    scanning: bool,
    prog: Option<Prog>,
    rx: Option<Receiver<Option<Node>>>,
    started: Option<Instant>,
    elapsed: f32,
    status: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.03;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0x6B, 0x4F, 0xA0]);
        stil::init(&cc.egui_ctx, [0x6B, 0x4F, 0xA0]);
        App {
            root: None,
            tree: None,
            largest: Vec::new(),
            exts: Vec::new(),
            expanded: HashSet::new(),
            scanning: false,
            prog: None,
            rx: None,
            started: None,
            elapsed: 0.0,
            status: t("Ordner wählen, um den Speicherplatz zu analysieren.").into(),
        }
    }

    fn start_scan(&mut self, path: PathBuf) {
        let files = Arc::new(AtomicU64::new(0));
        let dirs = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (fa, da, ba, ca) = (files.clone(), dirs.clone(), bytes.clone(), cancel.clone());
        let p = path.clone();
        std::thread::spawn(move || {
            let n = scan::scan(&p, &fa, &da, &ba, &ca);
            let _ = tx.send(n);
        });
        self.prog = Some(Prog { files, dirs, bytes, cancel });
        self.rx = Some(rx);
        self.scanning = true;
        self.tree = None;
        self.largest.clear();
        self.exts.clear();
        self.expanded.clear();
        self.root = Some(path);
        self.started = Some(Instant::now());
        self.status = t("Wird gescannt …").into();
    }

    fn poll(&mut self) {
        let done = if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(res) => Some(res),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(None),
            }
        } else {
            None
        };
        if let Some(res) = done {
            self.scanning = false;
            self.rx = None;
            self.elapsed = self.started.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
            match res {
                Some(node) => {
                    self.largest = scan::largest_files(&node, 30);
                    self.exts = scan::ext_sizes(&node);
                    self.expanded.insert(node.name.clone()); // Wurzel aufgeklappt
                    let secs = format!("{:.1}", self.elapsed);
                    self.status = tf("Fertig in {secs} s", &[("secs", &secs)]);
                    self.tree = Some(node);
                }
                None => {
                    self.status = t("Abgebrochen.").into();
                    self.prog = None;
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.scanning {
            ctx.request_repaint();
        }

        let total = self
            .tree
            .as_ref()
            .map(|t| t.size)
            .or_else(|| self.prog.as_ref().map(|p| p.bytes.load(Ordering::Relaxed)))
            .unwrap_or(0);

        // ---------------- Kopfleiste
        stil::kopfleiste(ctx, |ui| {
            stil::titel(ui, "Speicherwerk");
            stil::trenner(ui);
            if stil::knopf_an(ui, !self.scanning, Sym::OrdnerAuf, t("Ordner wählen")).clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.start_scan(d);
                }
            }
            if let Some(r) = &self.root {
                ui.add(egui::Label::new(egui::RichText::new(r.display().to_string()).color(thema::muted())).truncate());
            }
            stil::rechts(ui, |ui| {
                if stil::leise(ui, self.tree.is_some(), Sym::Download, t("CSV speichern …"), "").clicked() {
                    self.export_csv();
                }
            });
        });

        // ---------------- Fussleiste
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.scanning {
                    if let Some(p) = &self.prog {
                        ui.spinner();
                        stil::grau(
                            ui,
                            &tf(
                                "Scanne … {files} Dateien · {dirs} Ordner",
                                &[("files", &group_thousands(p.files.load(Ordering::Relaxed))), ("dirs", &group_thousands(p.dirs.load(Ordering::Relaxed)))],
                            ),
                        );
                    }
                } else if let Some(p) = &self.prog {
                    stil::grau(
                        ui,
                        &tf(
                            "{files} Dateien · {dirs} Ordner · {status}",
                            &[
                                ("files", &group_thousands(p.files.load(Ordering::Relaxed))),
                                ("dirs", &group_thousands(p.dirs.load(Ordering::Relaxed))),
                                ("status", &self.status),
                            ],
                        ),
                    );
                } else {
                    stil::grau(ui, self.status.as_str());
                }
                stil::rechts(ui, |ui| {
                    if self.scanning {
                        if stil::knopf(ui, Sym::Stopp, t("Abbrechen")).clicked() {
                            if let Some(p) = &self.prog {
                                p.cancel.store(true, Ordering::Relaxed);
                            }
                        }
                    } else if let Some(r) = self.root.clone() {
                        if stil::haupt(ui, true, Sym::Aktualisieren, t("Neu scannen")).clicked() {
                            self.start_scan(r);
                        }
                    } else if stil::haupt(ui, true, Sym::OrdnerAuf, t("Ordner wählen")).clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.start_scan(d);
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(tf("Gesamt: {size}", &[("size", &fmt_size(total))])).strong());
                });
            });
        });

        if self.tree.is_some() {
            stil::seitenleiste(ctx, 330.0, |ui| {
                stil::karte(ui, "groesste", Sym::Datei, t("Grösste Dateien"), true, |ui| {
                    let maxf = self.largest.first().map(|x| x.1).unwrap_or(1).max(1);
                    for (name, sz) in self.largest.iter().take(15) {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(short(name)).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.monospace(fmt_size(*sz));
                                bar(ui, *sz as f32 / maxf as f32, teal(), 80.0);
                            });
                        });
                    }
                });
                stil::karte(ui, "typen", Sym::Ebenen, t("Dateitypen"), true, |ui| {
                    let maxe = self.exts.first().map(|x| x.1).unwrap_or(1).max(1);
                    for (ext, sz, _c) in self.exts.iter().take(15) {
                        ui.horizontal(|ui| {
                            let shown = if ext == "(ohne)" { format!(".{}", t("(ohne)")) } else { format!(".{}", ext) };
                            ui.add(egui::Label::new(egui::RichText::new(shown).color(teal()).strong()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.monospace(fmt_size(*sz));
                                bar(ui, *sz as f32 / maxe as f32, teal(), 80.0);
                            });
                        });
                    }
                });
            });
        }

        stil::mitte(ctx, |ui| {
            if let Some(tree) = &self.tree {
                let total = tree.size.max(1);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    node_ui(ui, tree, &tree.name, 0, total, &mut self.expanded);
                });
            } else if self.scanning {
                stil::leer(ui, Sym::Suche, t("Wird gescannt …"), "");
            } else {
                stil::leer(ui, Sym::OrdnerAuf, t("Noch kein Ordner"), t("Oben einen Ordner wählen."));
            }
        });
    }
}

impl App {
    fn export_csv(&self) {
        if let Some(tree) = &self.tree {
            let name = format!("{}.csv", tree.name);
            if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                let _ = std::fs::write(path, scan::to_csv(tree));
            }
        }
    }
}

/// Letzter Pfadteil + Elternordner (kurz), damit die Herkunft sichtbar bleibt.
fn short(path: &str) -> String {
    let parts: Vec<&str> = path.rsplitn(3, '/').collect();
    match parts.as_slice() {
        [file, parent, _] => format!("{}/{}", parent, file),
        [file, parent] => format!("{}/{}", parent, file),
        [file] => file.to_string(),
        _ => path.to_string(),
    }
}

/// Kleines Aufklapp-Dreieck, selbst gezeichnet (unabhängig von Schrift-Glyphen).
fn tri_toggle(ui: &mut egui::Ui, open: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 16.0), egui::Sense::click());
    let c = rect.center();
    let col = if resp.hovered() { teal() } else { thema::muted() };
    let pts = if open {
        vec![
            egui::pos2(c.x - 4.0, c.y - 2.5),
            egui::pos2(c.x + 4.0, c.y - 2.5),
            egui::pos2(c.x, c.y + 3.5),
        ]
    } else {
        vec![
            egui::pos2(c.x - 2.5, c.y - 4.0),
            egui::pos2(c.x - 2.5, c.y + 4.0),
            egui::pos2(c.x + 3.5, c.y),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
    resp.clicked()
}

fn bar(ui: &mut egui::Ui, frac: f32, color: egui::Color32, w: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 8.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, thema::pick(egui::Color32::from_gray(235), egui::Color32::from_rgb(0x32, 0x35, 0x3d)));
    let mut fr = rect;
    fr.set_width(rect.width() * frac.clamp(0.0, 1.0));
    p.rect_filled(fr, 4.0, color);
}

fn node_ui(
    ui: &mut egui::Ui,
    node: &Node,
    key: &str,
    depth: usize,
    total: u64,
    expanded: &mut HashSet<String>,
) {
    let is_open = node.is_dir && expanded.contains(key);
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 16.0);
        if node.is_dir {
            if tri_toggle(ui, is_open) {
                if is_open {
                    expanded.remove(key);
                } else {
                    expanded.insert(key.to_string());
                }
            }
        } else {
            ui.add_space(14.0);
        }
        let label = if node.is_dir {
            egui::RichText::new(node.name.as_str()).strong()
        } else {
            egui::RichText::new(node.name.as_str()).color(thema::muted())
        };
        ui.add(egui::Label::new(label).truncate());
        let frac = node.size as f32 / total as f32;
        let col = if node.is_dir { teal() } else { thema::pick(egui::Color32::from_gray(150), egui::Color32::from_gray(110)) };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(fmt_size(node.size));
            ui.weak(format!("{:>3.0}%", frac * 100.0));
            bar(ui, frac, col, 220.0);
        });
    });
    if is_open {
        for ch in &node.children {
            let ckey = format!("{}/{}", key, ch.name);
            node_ui(ui, ch, &ckey, depth + 1, total, expanded);
        }
    }
}
