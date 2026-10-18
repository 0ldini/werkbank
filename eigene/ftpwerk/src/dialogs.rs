//! Dialoge: Verbindungen verwalten, Vertrauen, Passwort, Ordner/Umbenennen/Löschen, Rechte, Einstellungen.

use std::path::Path;

use egui::RichText;

use crate::app::{Act, App, ACCENT, ERR, WARN};
use crate::model::{mode_from_perms, perms_from_mode, rjoin, Proto, Site};
use crate::sprache::{t, tf};
use crate::store;
use crate::xfer::Cmd;

pub enum Dialog {
    Sites,
    Settings,
    Password { site: Box<Site>, text: String },
    Trust { site: Box<Site>, kind: &'static str, algo: String, fp: String, detail: String, changed: bool },
    NewFolder { remote: bool, name: String },
    Rename { remote: bool, old: String, name: String },
    Delete { remote: bool, items: Vec<(String, bool)> },
    Chmod { items: Vec<(String, bool)>, mode: u32, text: String, recursive: bool },
}

/// Zustand des Verbindungsverwalters.
#[derive(Default)]
pub struct SiteMgr {
    pub sel: Option<usize>,
    pub edit: Site,
}

impl SiteMgr {
    pub fn open(_sites: &[Site], quick: &Site) -> SiteMgr {
        let mut edit = quick.clone();
        edit.name.clear();
        SiteMgr { sel: None, edit }
    }
}

fn valid_name(n: &str) -> bool {
    let n = n.trim();
    !n.is_empty() && n != "." && n != ".." && !n.contains('/') && !n.contains('\\')
}

impl App {
    pub fn show_dialogs(&mut self, ctx: &egui::Context, _acts: &mut Vec<Act>) {
        let Some(mut d) = self.dialog.take() else { return };
        let mut keep = true;
        let mut after: Option<Box<dyn FnOnce(&mut App)>> = None;
        match &mut d {
            Dialog::Sites => keep = self.sites_dialog(ctx),
            Dialog::Settings => keep = self.settings_dialog(ctx),
            Dialog::Password { site, text } => {
                let mut open = true;
                let mut go = false;
                let mut cancel = false;
                egui::Window::new(t("Passwort")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
                    ui.label(tf("Passwort für {u}@{h}:", &[("u", &site.user), ("h", &site.host)]));
                    let r = ui.add(egui::TextEdit::singleline(text).password(true).desired_width(260.0));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.button(t("Verbinden")).clicked() {
                            go = true;
                        }
                        if ui.button(t("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    });
                });
                if go {
                    let mut s = (**site).clone();
                    s.password = std::mem::take(text);
                    s.user = s.user.clone();
                    // Eingegebenes Passwort in die Schnellzeile übernehmen (nur im Speicher).
                    self.quick.password = s.password.clone();
                    after = Some(Box::new(move |a: &mut App| a.connect_with_password(s)));
                    keep = false;
                } else if cancel || !open {
                    keep = false;
                }
            }
            Dialog::Trust { site, kind, algo, fp, detail, changed } => {
                let mut accept = false;
                let mut cancel = false;
                egui::Window::new(if *changed { t("⚠ Server hat sich geändert") } else { t("Dem Server vertrauen?") })
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.set_max_width(480.0);
                        if *changed {
                            ui.label(RichText::new(t("Der Fingerabdruck dieses Servers ist nicht mehr derselbe wie beim letzten Mal. Das kann ein Server-Umbau sein – oder jemand fängt die Verbindung ab.")).color(ERR));
                        } else {
                            ui.label(tf("{h} ist noch nicht bekannt. Prüfe den Fingerabdruck, bevor du ihm vertraust.", &[("h", &site.host)]));
                        }
                        ui.add_space(6.0);
                        if *kind == "ssh" {
                            ui.label(format!("{}: {}", t("Schlüsselart"), algo));
                        }
                        if !detail.is_empty() {
                            ui.label(RichText::new(detail.as_str()).weak());
                        }
                        ui.label(t("Fingerabdruck (SHA-256):"));
                        ui.add(egui::TextEdit::multiline(&mut fp.as_str()).font(egui::TextStyle::Monospace).desired_rows(2).desired_width(f32::INFINITY));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let label = if *changed { t("Trotzdem vertrauen und verbinden") } else { t("Vertrauen und verbinden") };
                            if ui.button(RichText::new(label).color(if *changed { WARN } else { ACCENT })).clicked() {
                                accept = true;
                            }
                            if ui.button(t("Abbrechen")).clicked() {
                                cancel = true;
                            }
                        });
                    });
                if accept {
                    if let Ok(mut tr) = self.trust.lock() {
                        tr.trust(kind, &site.hostport(), fp);
                        tr.save();
                    }
                    let s = (**site).clone();
                    after = Some(Box::new(move |a: &mut App| a.connect(s)));
                    keep = false;
                } else if cancel {
                    self.say(t("Verbindung nicht aufgebaut."));
                    keep = false;
                }
            }
            Dialog::NewFolder { remote, name } => {
                let mut ok = false;
                let mut cancel = false;
                egui::Window::new(t("Neuer Ordner")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(260.0));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        ok = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(valid_name(name), egui::Button::new(t("Anlegen"))).clicked() {
                            ok = true;
                        }
                        if ui.button(t("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    });
                });
                if ok && valid_name(name) {
                    let n = name.trim().to_string();
                    let remote = *remote;
                    after = Some(Box::new(move |a: &mut App| a.make_folder(remote, n)));
                    keep = false;
                } else if cancel {
                    keep = false;
                }
            }
            Dialog::Rename { remote, old, name } => {
                let mut ok = false;
                let mut cancel = false;
                egui::Window::new(t("Umbenennen")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(old.as_str());
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(280.0));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        ok = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(valid_name(name) && name.trim() != old.as_str(), egui::Button::new(t("Umbenennen"))).clicked() {
                            ok = true;
                        }
                        if ui.button(t("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    });
                });
                if ok && valid_name(name) && name.trim() != old.as_str() {
                    let (remote, o, n) = (*remote, old.clone(), name.trim().to_string());
                    after = Some(Box::new(move |a: &mut App| a.rename_item(remote, o, n)));
                    keep = false;
                } else if cancel {
                    keep = false;
                }
            }
            Dialog::Delete { remote, items } => {
                let mut ok = false;
                let mut cancel = false;
                egui::Window::new(t("Löschen")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    if items.len() == 1 {
                        ui.label(tf("«{n}» wirklich löschen?", &[("n", &items[0].0)]));
                    } else {
                        ui.label(tf("{n} Einträge wirklich löschen?", &[("n", &items.len())]));
                    }
                    if items.iter().any(|i| i.1) {
                        ui.label(RichText::new(t("Ordner werden samt Inhalt gelöscht.")).color(WARN));
                    }
                    ui.label(RichText::new(if *remote { t("Auf dem Server gibt es keinen Papierkorb.") } else { t("Lokal: wird endgültig gelöscht, nicht in den Papierkorb verschoben.") }).weak());
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new(t("Löschen")).color(ERR)).clicked() {
                            ok = true;
                        }
                        if ui.button(t("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    });
                });
                if ok {
                    let (remote, items) = (*remote, items.clone());
                    after = Some(Box::new(move |a: &mut App| a.delete_items(remote, items)));
                    keep = false;
                } else if cancel {
                    keep = false;
                }
            }
            Dialog::Chmod { items, mode, text, recursive } => {
                let mut ok = false;
                let mut cancel = false;
                egui::Window::new(t("Rechte ändern")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    if items.len() == 1 {
                        ui.label(items[0].0.as_str());
                    } else {
                        ui.label(tf("{n} Einträge", &[("n", &items.len())]));
                    }
                    let names = [t("Besitzer"), t("Gruppe"), t("Andere")];
                    let bits = [t("lesen"), t("schreiben"), t("ausführen")];
                    egui::Grid::new("perm").num_columns(4).show(ui, |ui| {
                        ui.label("");
                        for b in bits {
                            ui.label(b);
                        }
                        ui.end_row();
                        for (row, n) in names.iter().enumerate() {
                            ui.label(*n);
                            for col in 0..3 {
                                let bit = 1u32 << (8 - (row * 3 + col));
                                let mut on = *mode & bit != 0;
                                if ui.checkbox(&mut on, "").changed() {
                                    if on {
                                        *mode |= bit;
                                    } else {
                                        *mode &= !bit;
                                    }
                                    *text = format!("{:o}", *mode);
                                }
                            }
                            ui.end_row();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(t("Oktal:"));
                        if ui.add(egui::TextEdit::singleline(text).desired_width(50.0)).changed() {
                            if let Ok(m) = u32::from_str_radix(text.trim(), 8) {
                                *mode = m & 0o777;
                            }
                        }
                        ui.label(RichText::new(perms_from_mode(*mode)).monospace().weak());
                    });
                    if items.iter().any(|i| i.1) {
                        ui.checkbox(recursive, t("Auch für alles darin"));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(t("Übernehmen")).clicked() {
                            ok = true;
                        }
                        if ui.button(t("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    });
                });
                if ok {
                    let _ = mode_from_perms;
                    let (items, mode, recursive) = (items.clone(), *mode & 0o777, *recursive);
                    after = Some(Box::new(move |a: &mut App| a.chmod_items(items, mode, recursive)));
                    keep = false;
                } else if cancel {
                    keep = false;
                }
            }
        }
        if keep {
            self.dialog = Some(d);
        }
        if let Some(f) = after {
            f(self);
        }
    }

    fn connect_with_password(&mut self, site: Site) {
        // Passwort ist jetzt gesetzt; leeres Passwort wäre erneut eine Rückfrage → trotzdem verbinden.
        let mut s = site;
        if s.password.is_empty() {
            s.password = String::new();
        }
        self.connect_forced(s);
    }

    // ------------------------------------------------ Aktionen

    fn make_folder(&mut self, remote: bool, name: String) {
        if remote {
            let p = rjoin(&self.remote.cur, &name);
            if let Some(s) = &self.session {
                s.send(Cmd::Mkdir(p));
            }
        } else {
            let p = Path::new(&self.local.cur).join(&name);
            match std::fs::create_dir(&p) {
                Ok(()) => self.refresh_local(),
                Err(e) => self.fail(tf("Ordner anlegen: {e}", &[("e", &e)])),
            }
        }
    }

    fn rename_item(&mut self, remote: bool, old: String, new: String) {
        if remote {
            let (from, to) = (rjoin(&self.remote.cur, &old), rjoin(&self.remote.cur, &new));
            if let Some(s) = &self.session {
                s.send(Cmd::Rename { from, to });
            }
        } else {
            let base = Path::new(&self.local.cur).to_path_buf();
            match std::fs::rename(base.join(&old), base.join(&new)) {
                Ok(()) => self.refresh_local(),
                Err(e) => self.fail(tf("Umbenennen: {e}", &[("e", &e)])),
            }
        }
    }

    fn delete_items(&mut self, remote: bool, items: Vec<(String, bool)>) {
        if remote {
            let list: Vec<(String, bool)> = items.into_iter().map(|(n, d)| (rjoin(&self.remote.cur, &n), d)).collect();
            if let Some(s) = &self.session {
                s.send(Cmd::Delete(list));
            }
        } else {
            let base = Path::new(&self.local.cur).to_path_buf();
            let mut err: Option<String> = None;
            for (n, is_dir) in items {
                let p = base.join(&n);
                let r = if is_dir { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
                if let Err(e) = r {
                    err = Some(format!("{n}: {e}"));
                    break;
                }
            }
            if let Some(e) = err {
                self.fail(tf("Löschen: {e}", &[("e", &e)]));
            }
            self.refresh_local();
        }
    }

    fn chmod_items(&mut self, items: Vec<(String, bool)>, mode: u32, recursive: bool) {
        let list: Vec<(String, bool)> = items.into_iter().map(|(n, d)| (rjoin(&self.remote.cur, &n), d)).collect();
        if let Some(s) = &self.session {
            s.send(Cmd::Chmod { items: list, mode, recursive });
        }
    }

    // ------------------------------------------------ Verbindungsverwalter

    /// Gibt `false` zurück, wenn der Dialog geschlossen werden soll.
    fn sites_dialog(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut connect: Option<Site> = None;
        egui::Window::new(t("Verbindungen")).collapsible(false).default_size([640.0, 420.0]).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(190.0);
                    if ui.button(t("➕ Neue Verbindung")).clicked() {
                        self.sm.sel = None;
                        self.sm.edit = Site::default();
                    }
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("sitelist").max_height(300.0).show(ui, |ui| {
                        let mut pick: Option<usize> = None;
                        for (i, s) in self.sites.iter().enumerate() {
                            if ui.selectable_label(self.sm.sel == Some(i), s.title()).clicked() {
                                pick = Some(i);
                            }
                        }
                        if let Some(i) = pick {
                            self.sm.sel = Some(i);
                            self.sm.edit = self.sites[i].clone();
                        }
                        if self.sites.is_empty() {
                            ui.label(RichText::new(t("Noch keine gespeichert.")).weak());
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    let e = &mut self.sm.edit;
                    egui::Grid::new("siteedit").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label(t("Name"));
                        ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(260.0));
                        ui.end_row();
                        ui.label(t("Protokoll"));
                        let before = e.proto;
                        egui::ComboBox::from_id_salt("sproto").width(260.0).selected_text(t(e.proto.label())).show_ui(ui, |ui| {
                            for p in Proto::ALL {
                                ui.selectable_value(&mut e.proto, p, t(p.label()));
                            }
                        });
                        if before != e.proto {
                            e.port = e.proto.default_port();
                        }
                        ui.end_row();
                        ui.label(t("Server"));
                        ui.add(egui::TextEdit::singleline(&mut e.host).desired_width(260.0));
                        ui.end_row();
                        ui.label(t("Port"));
                        ui.add(egui::DragValue::new(&mut e.port).range(1..=65535));
                        ui.end_row();
                        ui.label(t("Benutzer"));
                        ui.add(egui::TextEdit::singleline(&mut e.user).desired_width(260.0));
                        ui.end_row();
                        ui.label(t("Passwort"));
                        ui.add(egui::TextEdit::singleline(&mut e.password).password(true).desired_width(260.0));
                        ui.end_row();
                        if e.proto == Proto::Sftp {
                            ui.label(t("Schlüsseldatei"));
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut e.key_file).desired_width(220.0));
                                if ui.button("…").clicked() {
                                    if let Some(p) = rfd::FileDialog::new().set_directory(store::home_dir().join(".ssh")).pick_file() {
                                        e.key_file = p.display().to_string();
                                    }
                                }
                            });
                            ui.end_row();
                        }
                        ui.label(t("Serverordner"));
                        ui.add(egui::TextEdit::singleline(&mut e.remote_dir).desired_width(260.0).hint_text("/"));
                        ui.end_row();
                        ui.label(t("Lokaler Ordner"));
                        ui.add(egui::TextEdit::singleline(&mut e.local_dir).desired_width(260.0));
                        ui.end_row();
                    });
                    let can = store::can_save_password();
                    ui.add_enabled(can, egui::Checkbox::new(&mut e.save_password, t("Passwort speichern (verschlüsselt, nur unter Windows)")));
                    if !can {
                        e.save_password = false;
                    }
                    if e.proto == Proto::Ftp {
                        ui.label(RichText::new(t("⚠ Einfaches FTP sendet Passwort und Dateien unverschlüsselt.")).color(WARN));
                    }
                    ui.add_space(6.0);
                    let ok = !e.host.trim().is_empty();
                    ui.horizontal(|ui| {
                        if ui.add_enabled(ok, egui::Button::new(RichText::new(t("Verbinden")).strong())).clicked() {
                            connect = Some(self.sm.edit.clone());
                        }
                        if ui.add_enabled(ok, egui::Button::new(t("💾 Speichern"))).clicked() {
                            let mut s = self.sm.edit.clone();
                            if s.name.trim().is_empty() {
                                s.name = s.title();
                            }
                            if !s.save_password {
                                // Nicht gespeicherte Passwörter nie in der Liste behalten.
                                s.password.clear();
                            }
                            match self.sm.sel {
                                Some(i) if i < self.sites.len() => self.sites[i] = s,
                                _ => {
                                    self.sites.push(s);
                                    self.sm.sel = Some(self.sites.len() - 1);
                                }
                            }
                            if let Err(err) = store::save_sites(&self.sites) {
                                self.status = tf("Speichern fehlgeschlagen: {e}", &[("e", &err)]);
                                self.status_err = true;
                            }
                        }
                        if ui.add_enabled(self.sm.sel.is_some(), egui::Button::new(RichText::new(t("🗑 Löschen")).color(ERR))).clicked() {
                            if let Some(i) = self.sm.sel.take() {
                                if i < self.sites.len() {
                                    self.sites.remove(i);
                                    let _ = store::save_sites(&self.sites);
                                }
                            }
                        }
                    });
                });
            });
        });
        if let Some(s) = connect {
            self.quick = s.clone();
            self.connect(s);
            return false;
        }
        open
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut changed = false;
        let mut forget: Option<(String, String)> = None;
        egui::Window::new(t("Einstellungen")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(t("Gleichzeitige Übertragungen:"));
                changed |= ui.add(egui::DragValue::new(&mut self.settings.parallel).range(1..=8)).changed();
            });
            ui.label(RichText::new(t("Gilt ab der nächsten Verbindung.")).weak());
            if ui.checkbox(&mut self.settings.show_hidden, t("Versteckte Dateien anzeigen")).changed() {
                changed = true;
                let h = self.settings.show_hidden;
                self.local.rebuild(h);
                self.remote.rebuild(h);
                self.refresh_local();
            }
            ui.separator();
            ui.strong(t("Vertraute Server"));
            let all: Vec<(String, String, String)> = self.trust.lock().map(|t| t.all().to_vec()).unwrap_or_default();
            if all.is_empty() {
                ui.label(RichText::new(t("Keine.")).weak());
            }
            for (k, h, _) in all {
                ui.horizontal(|ui| {
                    ui.label(format!("{} {}", if k == "ssh" { "SSH" } else { "TLS" }, h));
                    if ui.small_button(t("Vergessen")).clicked() {
                        forget = Some((k.clone(), h.clone()));
                    }
                });
            }
        });
        if let Some((k, h)) = forget {
            if let Ok(mut tr) = self.trust.lock() {
                tr.forget(&k, &h);
                tr.save();
            }
        }
        if changed {
            self.settings.save();
        }
        open
    }
}
