//! Oberfläche (egui): links die Regeln, rechts die Vorschau Alt -> Neu.
//! Umbenennen läuft zweistufig (erst in temporäre Namen, dann ans Ziel), damit
//! sich Namen auch tauschen lassen; danach ist Rückgängig möglich.

use crate::datefmt::fmt_date;
use crate::rename::{self, Case, Pos, Rules};
use crate::sprache::{t, tf};
use crate::stil::{self, Sym};
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
        stil::init(&cc.egui_ctx, [0x0F, 0x8B, 0x8D]);
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

        // ---------------- Kopfleiste
        stil::kopfleiste(ctx, |ui| {
            stil::titel(ui, "Umbenennwerk");
            stil::trenner(ui);
            if stil::knopf(ui, Sym::Plus, t("Dateien")).on_hover_text(t("Dateien hinzufügen")).clicked() {
                if let Some(p) = rfd::FileDialog::new().set_title(t("Dateien hinzufügen")).pick_files() {
                    self.add_paths(p);
                }
            }
            if stil::knopf(ui, Sym::Ordner, t("Ordner")).on_hover_text(t("Ordner hinzufügen")).clicked() {
                if let Some(d) = rfd::FileDialog::new().set_title(t("Ordner hinzufügen")).pick_folder() {
                    self.add_dir(&d);
                }
            }
            stil::trenner(ui);
            if stil::leise(ui, !self.undo.is_empty(), Sym::Rueckgaengig, t("Rückgängig"), "").clicked() {
                self.do_undo();
            }
            stil::rechts(ui, |ui| {
                if stil::gefahr(ui, !self.rows.is_empty(), Sym::Papierkorb, t("Liste leeren")).clicked() {
                    self.rows.clear();
                    self.undo.clear();
                    self.status = t("Liste geleert.").into();
                }
            });
        });

        // ---------------- Fussleiste
        stil::fussleiste(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        stil::grau(ui, &tf("{n} Dateien", &[("n", &self.rows.len())]));
                        if nconf > 0 {
                            ui.label(egui::RichText::new(tf("{n} Konflikt(e)", &[("n", &nconf)])).size(13.0).color(warn()));
                        }
                    });
                    if self.status_warn {
                        ui.label(egui::RichText::new(self.status.as_str()).size(13.0).color(warn()));
                    } else {
                        stil::grau(ui, self.status.as_str());
                    }
                });
                stil::rechts(ui, |ui| {
                    if stil::haupt(ui, !self.rows.is_empty(), Sym::Umbenennen, t("Umbenennen")).clicked() {
                        self.do_rename();
                    }
                });
            });
        });

        // ---------------- rechts: Regeln
        stil::seitenleiste(ctx, 320.0, |ui| {
            self.rules_ui(ui);
        });

        // Vorschau frisch berechnen (Regeln/Liste können sich oben geändert haben).
        let (new, conf) = self.eval();
        stil::mitte(ctx, |ui| {
            if self.rows.is_empty() {
                stil::leer(ui, Sym::Umbenennen, t("Noch keine Dateien"), t("Noch keine Dateien. Oben «Dateien» oder «Ordner» wählen."));
                return;
            }
            ui.horizontal(|ui| {
                ui.add_sized([34.0, 20.0], egui::Label::new(egui::RichText::new("#").size(12.0).color(thema::muted())));
                let w = ((ui.available_width() - 30.0) / 2.0).max(80.0);
                ui.add_sized([w, 20.0], egui::Label::new(egui::RichText::new(t("Alt")).size(12.0).strong().color(thema::muted())));
                ui.add_sized([20.0, 20.0], egui::Label::new(""));
                ui.add_sized([w, 20.0], egui::Label::new(egui::RichText::new(t("Neu")).size(12.0).strong().color(thema::muted())));
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, r) in self.rows.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 6.0, thema::hover());
                    }
                    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(0.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                    let w = ((rect.width() - 34.0 - 30.0) / 2.0).max(80.0);
                    row.add_sized([34.0, 30.0], egui::Label::new(egui::RichText::new(format!("{}", i + 1)).color(thema::muted())));
                    row.add_sized([w, 30.0], egui::Label::new(r.name.as_str()).truncate());
                    let (ar, _) = row.allocate_exact_size(egui::vec2(20.0, 30.0), egui::Sense::hover());
                    stil::sym(row.painter(), egui::Rect::from_center_size(ar.center(), egui::vec2(14.0, 14.0)), Sym::Umwandeln, thema::muted());
                    let txt = if conf[i] {
                        egui::RichText::new(format!("{}  ⚠", new[i])).color(warn())
                    } else if new[i] == r.name {
                        egui::RichText::new(new[i].as_str()).color(thema::muted())
                    } else {
                        egui::RichText::new(new[i].as_str()).color(accent_text())
                    };
                    row.add_sized([w, 30.0], egui::Label::new(txt).truncate());
                }
            });
        });
    }
}

impl App {
    fn rules_ui(&mut self, ui: &mut egui::Ui) {
        let full = f32::INFINITY;
        stil::karte(ui, "basis", Sym::Umbenennen, t("Basisname"), true, |ui| {
            stil::schalter(ui, &mut self.rules.replace_name, t("Namen komplett ersetzen"));
            if self.rules.replace_name {
                stil::feld(ui, t("Neuer Name:"), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.rules.new_name).desired_width(full));
                });
            } else {
                stil::feld(ui, t("Suchen:"), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.rules.find).desired_width(full));
                });
                stil::feld(ui, t("Ersetzen:"), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.rules.repl).desired_width(full));
                });
            }
        });
        stil::karte(ui, "nummer", Sym::Sortieren, t("Nummerierung"), true, |ui| {
            stil::schalter(ui, &mut self.rules.numbering, t("Fortlaufend nummerieren"));
            if self.rules.numbering {
                stil::feld(ui, t("Start / Schritt:"), |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.rules.num_start).range(0..=1_000_000));
                        ui.add(egui::DragValue::new(&mut self.rules.num_step).range(1..=10_000));
                    });
                });
                ui.columns(2, |c| {
                    stil::feld(&mut c[0], t("Stellen:"), |ui| {
                        ui.add(egui::DragValue::new(&mut self.rules.num_pad).range(1..=10));
                    });
                    stil::feld(&mut c[1], t("Trennzeichen:"), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.rules.num_sep).desired_width(60.0));
                    });
                });
                stil::feld(ui, t("Position:"), |ui| {
                    seg_pos(ui, &mut self.rules.num_pos);
                });
            }
        });
        stil::karte(ui, "praefix", Sym::Text, t("Präfix / Suffix"), false, |ui| {
            stil::feld(ui, t("Präfix:"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.rules.prefix).desired_width(full));
            });
            stil::feld(ui, t("Suffix:"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.rules.suffix).desired_width(full));
            });
        });
        stil::karte(ui, "gross", Sym::Text, t("Gross-/Kleinschreibung"), false, |ui| {
            stil::wahl(ui, &mut self.rules.case, &[(Case::Original, t("Original")), (Case::Upper, t("GROSS")), (Case::Lower, t("klein")), (Case::Title, t("Titel"))]);
        });
        stil::karte(ui, "datum", Sym::Kalender, t("Datum einfügen"), false, |ui| {
            stil::schalter(ui, &mut self.rules.add_date, t("Änderungsdatum der Datei"));
            if self.rules.add_date {
                stil::feld(ui, t("Format:"), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.date_fmt).desired_width(full));
                });
                stil::feld(ui, t("Position:"), |ui| {
                    seg_pos(ui, &mut self.rules.date_pos);
                });
                stil::hinweis(ui, t("Tokens: JJJJ MM TT hh mm ss (UTC)"));
            }
        });
        stil::karte(ui, "bereinigen", Sym::Haken, t("Bereinigen"), false, |ui| {
            stil::schalter(ui, &mut self.rules.spaces_us, t("Leerzeichen → Unterstrich"));
            stil::schalter(ui, &mut self.rules.strip_special, t("Sonderzeichen entfernen"));
            stil::schalter(ui, &mut self.rules.ext_lower, t("Erweiterung klein schreiben"));
        });
    }
}

fn seg_pos(ui: &mut egui::Ui, pos: &mut Pos) {
    stil::wahl(ui, pos, &[(Pos::Front, t("Vorn")), (Pos::Back, t("Hinten"))]);
}
