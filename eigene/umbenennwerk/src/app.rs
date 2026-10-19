//! Oberfläche (egui): links die Regeln, rechts die Vorschau Alt -> Neu.
//! Umbenennen läuft zweistufig (erst in temporäre Namen, dann ans Ziel), damit
//! sich Namen auch tauschen lassen; danach ist Rückgängig möglich.

use crate::datefmt::fmt_date;
use crate::rename::{self, Case, Pos, Rules};
use crate::sprache::{t, tf};
use crate::thema;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

struct Row {
    path: PathBuf,
    name: String,
    mtime: i64,
}

pub struct App {
    rows: Vec<Row>,
    rules: Rules,
    date_fmt: String,
    undo: Vec<(PathBuf, PathBuf)>, // (neuer Pfad, alter Pfad)
    status: String,
    status_warn: bool,
}

fn mtime_secs(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Füllung der Umbenennen-Schaltfläche (weisse Schrift darauf) – in beiden Designs gleich.
fn accent() -> egui::Color32 {
    egui::Color32::from_rgb(0x1f, 0x6f, 0xeb)
}
/// Akzent als Schriftfarbe (neue Namen), lesbar im hellen und dunklen Design.
fn accent_text() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0x1d, 0x4e, 0xd8), egui::Color32::from_rgb(0x60, 0xa5, 0xfa))
}
/// Warn-/Konfliktfarbe als Schrift.
fn warn() -> egui::Color32 {
    thema::pick(egui::Color32::from_rgb(0xb9, 0x1c, 0x1c), egui::Color32::from_rgb(0xf8, 0x71, 0x71))
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        for (_, f) in style.text_styles.iter_mut() {
            f.size *= 1.03;
        }
        cc.egui_ctx.set_style(style);
        thema::apply(&cc.egui_ctx);
        thema::titelleiste(cc, [0x0F, 0x8B, 0x8D]);
        App {
            rows: Vec::new(),
            rules: Rules::default(),
            date_fmt: "JJJJ-MM-TT".into(),
            undo: Vec::new(),
            status: t("Dateien oder einen Ordner hinzufügen.").into(),
            status_warn: false,
        }
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        let have: HashSet<PathBuf> = self.rows.iter().map(|r| r.path.clone()).collect();
        for p in paths {
            if p.is_file() && !have.contains(&p) {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
                if let Some(name) = name {
                    let mtime = mtime_secs(&p);
                    self.rows.push(Row { path: p, name, mtime });
                }
            }
        }
        self.status = tf("{n} Dateien", &[("n", &self.rows.len())]);
        self.status_warn = false;
    }

    fn add_dir(&mut self, dir: &Path) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut paths = Vec::new();
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() {
                    paths.push(p);
                }
            }
            paths.sort();
            self.add_paths(paths);
        }
    }

    fn date_for(&self, r: &Row) -> Option<String> {
        if self.rules.add_date {
            Some(fmt_date(r.mtime, &self.date_fmt))
        } else {
            None
        }
    }

    /// Liefert die neuen Namen und je Zeile ein Konflikt-Flag.
    fn eval(&self) -> (Vec<String>, Vec<bool>) {
        let new: Vec<String> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| rename::build(&r.name, i, self.date_for(r).as_deref(), &self.rules))
            .collect();
        let mut conf = rename::conflicts(&new);
        let ours: HashSet<PathBuf> = self.rows.iter().map(|r| r.path.clone()).collect();
        for (i, r) in self.rows.iter().enumerate() {
            if conf[i] || new[i] == r.name {
                continue;
            }
            if let Some(parent) = r.path.parent() {
                let target = parent.join(&new[i]);
                if target.exists() && !ours.contains(&target) {
                    conf[i] = true;
                }
            }
        }
        (new, conf)
    }

    fn do_rename(&mut self) {
        let (new, conf) = self.eval();
        let mut jobs: Vec<(usize, PathBuf, PathBuf)> = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            if conf[i] || new[i] == r.name {
                continue;
            }
            if let Some(parent) = r.path.parent() {
                jobs.push((i, r.path.clone(), parent.join(&new[i])));
            }
        }
        if jobs.is_empty() {
            self.status = t("Nichts umzubenennen.").into();
            self.status_warn = false;
            return;
        }
        // Phase 1: alt -> temporär
        let mut temps: Vec<(usize, PathBuf, PathBuf)> = Vec::new();
        let mut err = 0usize;
        for (k, (i, old, fin)) in jobs.iter().enumerate() {
            let tmp = old.with_file_name(format!(".uwtmp_{}_{}", std::process::id(), k));
            match std::fs::rename(old, &tmp) {
                Ok(_) => temps.push((*i, tmp, fin.clone())),
                Err(_) => err += 1,
            }
        }
        // Phase 2: temporär -> Ziel
        let mut undo: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut done = 0usize;
        for (i, tmp, fin) in temps.into_iter() {
            let old = self.rows[i].path.clone();
            match std::fs::rename(&tmp, &fin) {
                Ok(_) => {
                    if let Some(name) = fin.file_name().map(|n| n.to_string_lossy().into_owned()) {
                        self.rows[i].name = name;
                    }
                    self.rows[i].path = fin.clone();
                    undo.push((fin, old));
                    done += 1;
                }
                Err(_) => {
                    let _ = std::fs::rename(&tmp, &old);
                    err += 1;
                }
            }
        }
        self.undo = undo;
        self.status = if err > 0 {
            tf("{n} umbenannt, {f} fehlgeschlagen", &[("n", &done), ("f", &err)])
        } else {
            tf("{n} umbenannt", &[("n", &done)])
        };
        self.status_warn = err > 0;
    }

    fn do_undo(&mut self) {
        let mut ok = 0usize;
        for (newp, oldp) in self.undo.clone().into_iter().rev() {
            if std::fs::rename(&newp, &oldp).is_ok() {
                if let Some(r) = self.rows.iter_mut().find(|r| r.path == newp) {
                    if let Some(name) = oldp.file_name().map(|n| n.to_string_lossy().into_owned()) {
                        r.name = name;
                    }
                    r.path = oldp.clone();
                }
                ok += 1;
            }
        }
        self.undo.clear();
        self.status = tf("{n} rückgängig gemacht", &[("n", &ok)]);
        self.status_warn = false;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let nconf = self.eval().1.iter().filter(|&&c| c).count();

        egui::TopBottomPanel::top("tbar").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(t("  ＋ Dateien  ")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_title(t("Dateien hinzufügen")).pick_files() {
                        self.add_paths(p);
                    }
                }
                if ui.button(t("  📁 Ordner  ")).clicked() {
                    if let Some(d) = rfd::FileDialog::new().set_title(t("Ordner hinzufügen")).pick_folder() {
                        self.add_dir(&d);
                    }
                }
                ui.add_enabled_ui(!self.undo.is_empty(), |ui| {
                    if ui.button(t("  ↩ Rückgängig  ")).clicked() {
                        self.do_undo();
                    }
                });
                if ui.button(t("Liste leeren")).clicked() {
                    self.rows.clear();
                    self.undo.clear();
                    self.status = t("Liste geleert.").into();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let btn = egui::Button::new(
                        egui::RichText::new(t("  ✓  Umbenennen  ")).color(egui::Color32::WHITE),
                    )
                    .fill(accent());
                    if ui.add_enabled(!self.rows.is_empty(), btn).clicked() {
                        self.do_rename();
                    }
                    ui.add_space(10.0);
                    if nconf > 0 {
                        ui.colored_label(
                            warn(),
                            tf("{n} Konflikt(e)", &[("n", &nconf)]),
                        );
                        ui.label("·");
                    }
                    ui.weak(tf("{n} Dateien", &[("n", &self.rows.len())]));
                });
            });
            ui.add_space(6.0);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if self.status_warn {
                    ui.colored_label(warn(), self.status.as_str());
                } else {
                    ui.weak(self.status.as_str());
                }
            });
            ui.add_space(4.0);
        });

        egui::SidePanel::left("rules")
            .resizable(false)
            .exact_width(320.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(8.0);
                    self.rules_ui(ui);
                    ui.add_space(10.0);
                });
            });

        // Vorschau frisch berechnen (Regeln/Liste können sich oben geändert haben).
        let (new, conf) = self.eval();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            egui::Grid::new("head")
                .num_columns(4)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("#");
                    ui.strong(t("Alt"));
                    ui.label("");
                    ui.strong(t("Neu"));
                    ui.end_row();
                });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("rows")
                    .num_columns(4)
                    .spacing([8.0, 4.0])
                    .striped(true)
                    .min_col_width(20.0)
                    .show(ui, |ui| {
                        for (i, r) in self.rows.iter().enumerate() {
                            ui.weak(format!("{}", i + 1));
                            ui.label(r.name.as_str());
                            ui.weak("→");
                            if conf[i] {
                                ui.colored_label(
                                    warn(),
                                    format!("{}  ⚠", new[i]),
                                );
                            } else if new[i] == r.name {
                                ui.weak(new[i].as_str());
                            } else {
                                ui.colored_label(accent_text(), new[i].as_str());
                            }
                            ui.end_row();
                        }
                    });
                if self.rows.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.weak(t("Noch keine Dateien. Oben «Dateien» oder «Ordner» wählen."));
                    });
                }
            });
        });
    }
}

impl App {
    fn rules_ui(&mut self, ui: &mut egui::Ui) {
        section(ui, t("Basisname"));
        ui.checkbox(&mut self.rules.replace_name, t("Namen komplett ersetzen"));
        if self.rules.replace_name {
            ui.horizontal(|ui| {
                ui.label(t("Neuer Name:"));
                ui.text_edit_singleline(&mut self.rules.new_name);
            });
        } else {
            egui::Grid::new("fr").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(t("Suchen:"));
                ui.text_edit_singleline(&mut self.rules.find);
                ui.end_row();
                ui.label(t("Ersetzen:"));
                ui.text_edit_singleline(&mut self.rules.repl);
                ui.end_row();
            });
        }

        ui.add_space(10.0);
        section(ui, t("Nummerierung"));
        ui.checkbox(&mut self.rules.numbering, t("Fortlaufend nummerieren"));
        if self.rules.numbering {
            egui::Grid::new("num").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(t("Start / Schritt:"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.rules.num_start).range(0..=1_000_000));
                    ui.add(egui::DragValue::new(&mut self.rules.num_step).range(1..=10_000));
                });
                ui.end_row();
                ui.label(t("Stellen:"));
                ui.add(egui::DragValue::new(&mut self.rules.num_pad).range(1..=10));
                ui.end_row();
                ui.label(t("Trennzeichen:"));
                ui.add(egui::TextEdit::singleline(&mut self.rules.num_sep).desired_width(60.0));
                ui.end_row();
                ui.label(t("Position:"));
                seg_pos(ui, &mut self.rules.num_pos);
                ui.end_row();
            });
        }

        ui.add_space(10.0);
        section(ui, t("Präfix / Suffix"));
        egui::Grid::new("ps").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label(t("Präfix:"));
            ui.text_edit_singleline(&mut self.rules.prefix);
            ui.end_row();
            ui.label(t("Suffix:"));
            ui.text_edit_singleline(&mut self.rules.suffix);
            ui.end_row();
        });

        ui.add_space(10.0);
        section(ui, t("Gross-/Kleinschreibung"));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.rules.case, Case::Original, t("Original"));
            ui.selectable_value(&mut self.rules.case, Case::Upper, t("GROSS"));
            ui.selectable_value(&mut self.rules.case, Case::Lower, t("klein"));
            ui.selectable_value(&mut self.rules.case, Case::Title, t("Titel"));
        });

        ui.add_space(10.0);
        section(ui, t("Datum einfügen"));
        ui.checkbox(&mut self.rules.add_date, t("Änderungsdatum der Datei"));
        if self.rules.add_date {
            egui::Grid::new("dt").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(t("Format:"));
                ui.text_edit_singleline(&mut self.date_fmt);
                ui.end_row();
                ui.label(t("Position:"));
                seg_pos(ui, &mut self.rules.date_pos);
                ui.end_row();
            });
            ui.weak(t("Tokens: JJJJ MM TT hh mm ss (UTC)"));
        }

        ui.add_space(10.0);
        section(ui, t("Bereinigen"));
        ui.checkbox(&mut self.rules.spaces_us, t("Leerzeichen → Unterstrich"));
        ui.checkbox(&mut self.rules.strip_special, t("Sonderzeichen entfernen"));
        ui.checkbox(&mut self.rules.ext_lower, t("Erweiterung klein schreiben"));
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).strong().size(12.0).color(thema::muted()));
}

fn seg_pos(ui: &mut egui::Ui, pos: &mut Pos) {
    ui.horizontal(|ui| {
        ui.selectable_value(pos, Pos::Front, t("Vorn"));
        ui.selectable_value(pos, Pos::Back, t("Hinten"));
    });
}
