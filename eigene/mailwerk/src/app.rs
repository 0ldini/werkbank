//! Oberfläche (egui): Seitenleiste mit Konten und Ordnern, Nachrichtenliste, Lesebereich,
//! Schreiben und Konto-Einrichtung. Die Oberfläche zeichnet nur; alles Netzwerk läuft im `Backend`.

use std::time::Duration;

use egui::{pos2, vec2, Align2, Color32, FontId, RichText, Sense};

use crate::backend::Backend;
use crate::mime::{self, Address};
use crate::presets;
use crate::role::Role;
use crate::save;
use crate::sprache::{is_en, t, tf};
use crate::store::Saved;
use crate::textutil;
use crate::timeutil;
use crate::types::{Account, Cmd, Ev, HeaderRow, Kind, MailBody, Security};

const ACCENT: Color32 = Color32::from_rgb(0x0B, 0x7A, 0x70);
const ERR: Color32 = Color32::from_rgb(0xC0, 0x39, 0x2B);
const STORAGE_KEY: &str = "mailwerk_v1";
const HEADERS_PER_LOAD: u32 = 100;
const REFRESH_SECS: f64 = 300.0;

// ================================================================ Zustand

struct Setup {
    acc: Account,
    is_new: bool,
    hint: String,
    error: String,
}

struct Compose {
    acc_id: u64,
    to: String,
    cc: String,
    bcc: String,
    show_cc: bool,
    subject: String,
    body: String,
    atts: Vec<(String, Vec<u8>)>,
    in_reply_to: String,
    references: String,
    sending: bool,
    error: String,
}

impl Compose {
    fn new(acc_id: u64) -> Compose {
        Compose {
            acc_id,
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            show_cc: false,
            subject: String::new(),
            body: String::new(),
            atts: Vec::new(),
            in_reply_to: String::new(),
            references: String::new(),
            sending: false,
            error: String::new(),
        }
    }
}

enum Mode {
    New,
    Reply(bool),
    Forward,
}

/// Was die Oberfläche in diesem Bild ausgelöst hat. Wird nach dem Zeichnen ausgeführt.
enum Act {
    SelectAccount(u64),
    SelectFolder(String),
    SelectMail(String),
    Back,
    Refresh,
    Compose(Mode),
    ToggleFlag,
    MarkUnread,
    Archive,
    Delete,
    SaveAtt(usize),
    AddAccount,
    AddDemo,
    EditAccount(u64),
    AskDeleteAccount(u64),
    DeleteAccount(u64),
    CancelDelete,
    SuggestServers,
    SaveSetup,
    CancelSetup,
    SendCompose,
    CloseCompose,
    PickAttachment,
    RemoveAttachment(usize),
    ClearStatus,
    ToggleNav,
}

pub struct App {
    saved: Saved,
    backend: Backend,
    folder_id: String,
    sel_mail: Option<String>,
    open: Option<MailBody>,
    pending: usize,
    status: Option<(String, bool)>,
    search: String,
    only_unread: bool,
    setup: Option<Setup>,
    compose: Option<Compose>,
    confirm_delete: Option<u64>,
    show_nav: bool,
    last_refresh: f64,
    styled_dark: Option<bool>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let saved = cc
            .storage
            .and_then(|s| s.get_string(STORAGE_KEY))
            .map(|t| Saved::from_json_str(&t))
            .unwrap_or_default();
        let mut app = App {
            saved,
            backend: Backend::new(cc.egui_ctx.clone()),
            folder_id: String::new(),
            sel_mail: None,
            open: None,
            pending: 0,
            status: None,
            search: String::new(),
            only_unread: false,
            setup: None,
            compose: None,
            confirm_delete: None,
            show_nav: false,
            last_refresh: 0.0,
            styled_dark: None,
        };
        let first = app.saved.accounts.iter().find(|a| a.id == app.saved.selected).or(app.saved.accounts.first()).map(|a| a.id);
        if let Some(id) = first {
            app.select_account(id);
        }
        app
    }

    // ------------------------------------------------------------ Hilfen

    fn cur_acc(&self) -> Option<&Account> {
        self.saved.accounts.iter().find(|a| a.id == self.saved.selected)
    }

    fn usable(acc: &Account) -> bool {
        !(cfg!(target_arch = "wasm32") && !acc.works_in_browser())
    }

    fn cmd(&mut self, cmd: Cmd) {
        if let Some(acc) = self.cur_acc().cloned() {
            if Self::usable(&acc) {
                self.pending += 1;
                self.backend.send(&acc, cmd);
            }
        }
    }

    fn folder_by_role(&self, role: Role) -> Option<String> {
        let acc = self.cur_acc()?;
        self.saved.cache(acc.id)?.folders.iter().find(|f| f.role == role).map(|f| f.id.clone())
    }

    fn cur_role(&self) -> Role {
        self.cur_acc()
            .and_then(|a| self.saved.cache(a.id))
            .and_then(|c| c.folders.iter().find(|f| f.id == self.folder_id))
            .map(|f| f.role)
            .unwrap_or(Role::Other)
    }

    fn select_account(&mut self, id: u64) {
        self.saved.selected = id;
        self.sel_mail = None;
        self.open = None;
        self.show_nav = false;
        self.folder_id = self.saved.cache(id).map(|c| c.folder_id.clone()).unwrap_or_default();
        self.refresh();
    }

    fn refresh(&mut self) {
        self.cmd(Cmd::Folders);
        if !self.folder_id.is_empty() {
            let folder = self.folder_id.clone();
            self.cmd(Cmd::Headers { folder, count: HEADERS_PER_LOAD });
        }
    }

    fn load_headers(&mut self) {
        let folder = self.folder_id.clone();
        self.cmd(Cmd::Headers { folder, count: HEADERS_PER_LOAD });
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some((text.into(), error));
    }

    fn row_mut(&mut self, id: &str) -> Option<&mut HeaderRow> {
        let acc = self.saved.selected;
        self.saved.cache_mut(acc).headers.iter_mut().find(|h| h.id == id)
    }

    /// Ungelesen-Zähler des aktuellen Ordners von Hand nachführen, bis der Server neue Zahlen liefert.
    fn bump_unread(&mut self, delta: i32) {
        let acc = self.saved.selected;
        let folder = self.folder_id.clone();
        if let Some(f) = self.saved.cache_mut(acc).folders.iter_mut().find(|f| f.id == folder) {
            f.unread = (f.unread as i32 + delta).max(0) as u32;
        }
    }

    fn remove_row(&mut self, id: &str) -> Option<HeaderRow> {
        let acc = self.saved.selected;
        let c = self.saved.cache_mut(acc);
        let i = c.headers.iter().position(|h| h.id == id)?;
        let row = c.headers.remove(i);
        if let Some(f) = c.folders.iter_mut().find(|f| f.id == c.folder_id) {
            f.total = f.total.saturating_sub(1);
            if row.unread {
                f.unread = f.unread.saturating_sub(1);
            }
        }
        Some(row)
    }

    // ------------------------------------------------------------ Ereignisse

    fn on_event(&mut self, ev: Ev) {
        match ev {
            Ev::Folders { acc, folders } => {
                self.pending = self.pending.saturating_sub(1);
                self.saved.cache_mut(acc).folders = folders;
                if acc == self.saved.selected {
                    let known = self.saved.cache(acc).map_or(false, |c| c.folders.iter().any(|f| f.id == self.folder_id));
                    if !known {
                        let pick = self
                            .saved
                            .cache(acc)
                            .and_then(|c| c.folders.iter().find(|f| f.role == Role::Inbox).or(c.folders.first()))
                            .map(|f| f.id.clone());
                        if let Some(id) = pick {
                            self.folder_id = id;
                            self.load_headers();
                        }
                    }
                }
            }
            Ev::Headers { acc, folder, rows, total } => {
                self.pending = self.pending.saturating_sub(1);
                if acc == self.saved.selected && !self.folder_id.is_empty() && folder != self.folder_id {
                    return; // veraltete Antwort für einen Ordner, den man schon verlassen hat
                }
                let c = self.saved.cache_mut(acc);
                if let Some(f) = c.folders.iter_mut().find(|f| f.id == folder) {
                    f.total = total;
                }
                c.folder_id = folder;
                c.headers = rows;
            }
            Ev::Body { acc, folder, body } => {
                self.pending = self.pending.saturating_sub(1);
                let mut keep = body.clone();
                keep.id = format!("{folder}|{}", body.id);
                for a in &mut keep.atts {
                    a.data.clear();
                }
                self.saved.cache_mut(acc).remember_body(&keep);
                if acc == self.saved.selected && self.sel_mail.as_deref() == Some(body.id.as_str()) {
                    self.open = Some(body);
                }
            }
            Ev::Attachment { name, data, .. } => {
                self.pending = self.pending.saturating_sub(1);
                save::save_bytes(&textutil::safe_filename(&name), &data);
            }
            Ev::Done { what, note, .. } => {
                self.pending = self.pending.saturating_sub(1);
                if what == "send" {
                    self.compose = None;
                    self.set_status(if note.is_empty() { t("Nachricht gesendet.").to_string() } else { note }, false);
                    self.refresh();
                } else {
                    if what.starts_with("move:") || what.starts_with("delete:") {
                        self.cmd(Cmd::Folders);
                    }
                    if !note.is_empty() {
                        self.set_status(note, false);
                    }
                }
            }
            Ev::Failed { acc, what, err } => {
                self.pending = self.pending.saturating_sub(1);
                let name = self.saved.accounts.iter().find(|a| a.id == acc).map(|a| a.name.clone()).unwrap_or_default();
                if what == "send" {
                    if let Some(c) = self.compose.as_mut() {
                        c.sending = false;
                        c.error = err.clone();
                    }
                }
                self.set_status(format!("{name}: {err}"), true);
                // Bei Fehlern in Änderungen stimmt die lokale Ansicht nicht mehr: neu laden.
                if what.starts_with("move:") || what.starts_with("delete:") || what.starts_with("seen:") || what.starts_with("flag:") {
                    if acc == self.saved.selected {
                        self.refresh();
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ Aktionen

    fn act(&mut self, a: Act) {
        match a {
            Act::SelectAccount(id) => self.select_account(id),
            Act::SelectFolder(id) => {
                self.folder_id = id;
                self.sel_mail = None;
                self.open = None;
                self.show_nav = false;
                self.load_headers();
            }
            Act::SelectMail(id) => {
                self.sel_mail = Some(id.clone());
                let key = format!("{}|{}", self.folder_id, id);
                let acc = self.saved.selected;
                self.open = self.saved.cache(acc).and_then(|c| c.bodies.iter().find(|b| b.id == key)).cloned().map(|mut b| {
                    b.id = id.clone();
                    b
                });
                let was_unread = self.row_mut(&id).map(|r| std::mem::replace(&mut r.unread, false)).unwrap_or(false);
                let folder = self.folder_id.clone();
                if was_unread {
                    self.bump_unread(-1);
                    self.cmd(Cmd::SetSeen { folder: folder.clone(), id: id.clone(), seen: true });
                }
                self.cmd(Cmd::Body { folder, id });
            }
            Act::Back => {
                self.sel_mail = None;
                self.open = None;
            }
            Act::Refresh => {
                self.refresh();
                self.status = None;
            }
            Act::Compose(mode) => self.start_compose(mode),
            Act::ToggleFlag => {
                if let Some(id) = self.sel_mail.clone() {
                    let folder = self.folder_id.clone();
                    let flagged = self.row_mut(&id).map(|r| {
                        r.flagged = !r.flagged;
                        r.flagged
                    });
                    if let Some(flagged) = flagged {
                        self.cmd(Cmd::SetFlagged { folder, id, flagged });
                    }
                }
            }
            Act::MarkUnread => {
                if let Some(id) = self.sel_mail.take() {
                    self.open = None;
                    let folder = self.folder_id.clone();
                    let changed = self.row_mut(&id).map(|r| std::mem::replace(&mut r.unread, true) == false).unwrap_or(false);
                    if changed {
                        self.bump_unread(1);
                    }
                    self.cmd(Cmd::SetSeen { folder, id, seen: false });
                }
            }
            Act::Archive => {
                if let (Some(id), Some(target)) = (self.sel_mail.take(), self.folder_by_role(Role::Archive)) {
                    self.open = None;
                    self.remove_row(&id);
                    let folder = self.folder_id.clone();
                    self.cmd(Cmd::MoveTo { folder, id, target });
                }
            }
            Act::Delete => {
                if let Some(id) = self.sel_mail.take() {
                    self.open = None;
                    self.remove_row(&id);
                    let folder = self.folder_id.clone();
                    let trash = if self.cur_role() == Role::Trash { None } else { self.folder_by_role(Role::Trash) };
                    self.cmd(Cmd::Delete { folder, id, trash });
                }
            }
            Act::SaveAtt(i) => {
                let Some(att) = self.open.as_ref().and_then(|b| b.atts.get(i)).cloned() else { return };
                if !att.data.is_empty() {
                    save::save_bytes(&textutil::safe_filename(&att.name), &att.data);
                } else if !att.blob.is_empty() {
                    self.cmd(Cmd::Attachment { blob: att.blob, name: att.name, content_type: att.content_type });
                } else {
                    self.set_status(t("Der Anhang ist noch nicht geladen. Bitte einen Moment warten und nochmals versuchen."), false);
                }
            }
            Act::AddAccount => {
                let id = self.saved.new_id();
                let mut acc = Account::blank(id);
                if cfg!(target_arch = "wasm32") {
                    acc.kind = Kind::Jmap;
                }
                self.setup = Some(Setup { acc, is_new: true, hint: String::new(), error: String::new() });
            }
            Act::AddDemo => {
                let id = self.saved.new_id();
                self.saved.accounts.push(Account::demo(id));
                self.select_account(id);
            }
            Act::EditAccount(id) => {
                if let Some(acc) = self.saved.accounts.iter().find(|a| a.id == id).cloned() {
                    self.setup = Some(Setup { acc, is_new: false, hint: String::new(), error: String::new() });
                }
            }
            Act::AskDeleteAccount(id) => self.confirm_delete = Some(id),
            Act::CancelDelete => self.confirm_delete = None,
            Act::DeleteAccount(id) => {
                self.confirm_delete = None;
                self.setup = None;
                self.saved.remove_account(id);
                if self.saved.selected == id {
                    match self.saved.accounts.first().map(|a| a.id) {
                        Some(n) => self.select_account(n),
                        None => {
                            self.saved.selected = 0;
                            self.folder_id.clear();
                            self.sel_mail = None;
                            self.open = None;
                        }
                    }
                }
            }
            Act::SuggestServers => {
                if let Some(s) = self.setup.as_mut() {
                    s.hint = presets::apply(&mut s.acc).map(|h| t(h.note).to_string()).unwrap_or_default();
                    if s.acc.user.is_empty() {
                        s.acc.user = s.acc.email.clone();
                    }
                    if s.acc.name.is_empty() {
                        s.acc.name = s.acc.email.split('@').nth(1).unwrap_or("").to_string();
                    }
                }
            }
            Act::SaveSetup => {
                let Some(mut s) = self.setup.take() else { return };
                s.acc.name = s.acc.name.trim().to_string();
                s.acc.email = s.acc.email.trim().to_string();
                if s.acc.person.trim().is_empty() {
                    s.acc.person = s.acc.name.clone();
                }
                if cfg!(target_arch = "wasm32") && s.acc.kind == Kind::Imap {
                    s.error = t("Im Browser gehen nur Anbieter mit JMAP. Für dieses Konto bitte die Desktop-Version verwenden.").into();
                    self.setup = Some(s);
                    return;
                }
                if let Err(e) = s.acc.validate() {
                    s.error = e;
                    self.setup = Some(s);
                    return;
                }
                let id = s.acc.id;
                if s.is_new {
                    self.saved.accounts.push(s.acc);
                } else if let Some(slot) = self.saved.accounts.iter_mut().find(|a| a.id == id) {
                    *slot = s.acc;
                }
                self.select_account(id);
            }
            Act::CancelSetup => self.setup = None,
            Act::SendCompose => self.send_compose(),
            Act::CloseCompose => self.compose = None,
            Act::PickAttachment => {
                let files = save::pick_files();
                if let Some(c) = self.compose.as_mut() {
                    c.atts.extend(files);
                }
            }
            Act::RemoveAttachment(i) => {
                if let Some(c) = self.compose.as_mut() {
                    if i < c.atts.len() {
                        c.atts.remove(i);
                    }
                }
            }
            Act::ClearStatus => self.status = None,
            Act::ToggleNav => self.show_nav = !self.show_nav,
        }
    }

    fn start_compose(&mut self, mode: Mode) {
        let Some(acc) = self.cur_acc().cloned() else { return };
        let mut c = Compose::new(acc.id);
        let sig = if acc.signature.trim().is_empty() { String::new() } else { format!("\n\n-- \n{}", acc.signature.trim()) };
        if !matches!(mode, Mode::New) && self.open.is_none() {
            return;
        }
        match mode {
            Mode::New => c.body = sig,
            Mode::Reply(all) => {
                let b = self.open.clone().unwrap();
                let (to, cc) = mime::reply_recipients(&b.as_parsed(), &acc.email, all);
                c.to = addr_list_text(&to);
                c.cc = addr_list_text(&cc);
                c.show_cc = !cc.is_empty();
                c.subject = mime::reply_subject(&b.subject);
                let who = b.from.first().map(|a| a.display()).unwrap_or_default();
                let when = long_date(b.ts, timeutil::now_ts());
                let intro = tf("\n\nAm {when} schrieb {who}:\n", &[("when", &when), ("who", &who)]);
                c.body = format!("{intro}{}{sig}", mime::quote_text(&b.text));
                c.in_reply_to = b.message_id.clone();
                c.references = format!("{} {}", b.references, b.message_id).trim().to_string();
            }
            Mode::Forward => {
                let b = self.open.clone().unwrap();
                c.subject = mime::forward_subject(&b.subject);
                let head = tf("\n\n---------- Weitergeleitete Nachricht ----------\nVon: {from}\nDatum: {date}\nBetreff: {subject}\nAn: {to}\n\n",
                    &[
                        ("date", &long_date(b.ts, timeutil::now_ts())),
                        ("from", &addr_list_text(&b.from)),
                        ("to", &addr_list_text(&b.to)),
                        ("subject", &b.subject),
                    ],
                );
                c.body = format!("{sig}{head}{}", b.text);
            }
        }
        self.compose = Some(c);
    }

    fn build_outgoing(&self, acc: &Account) -> Result<mime::Outgoing, String> {
        let c = self.compose.as_ref().ok_or(t("Kein Entwurf offen."))?;
        let to = Address::parse_list(&c.to);
        let cc = Address::parse_list(&c.cc);
        let bcc = Address::parse_list(&c.bcc);
        if to.is_empty() && cc.is_empty() && bcc.is_empty() {
            return Err(t("Bitte mindestens einen Empfänger eingeben.").into());
        }
        if let Some(bad) = to.iter().chain(cc.iter()).chain(bcc.iter()).find(|a| !a.email.contains('@')) {
            return Err(tf("«{email}» ist keine gültige E-Mail-Adresse.", &[("email", &bad.email)]));
        }
        if acc.kind == Kind::Jmap && !c.atts.is_empty() {
            return Err(t("Anhänge beim Senden über JMAP sind noch nicht möglich. Bitte ohne Anhang senden.").into());
        }
        Ok(mime::Outgoing {
            from: acc.from_address(),
            to,
            cc,
            bcc,
            subject: c.subject.clone(),
            body: c.body.clone(),
            attachments: c
                .atts
                .iter()
                .map(|(n, d)| mime::Attachment { filename: n.clone(), content_type: guess_type(n).to_string(), data: d.clone() })
                .collect(),
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
        })
    }

    fn send_compose(&mut self) {
        let Some(acc_id) = self.compose.as_ref().map(|c| c.acc_id) else { return };
        let Some(acc) = self.saved.accounts.iter().find(|a| a.id == acc_id).cloned() else { return };
        if !Self::usable(&acc) {
            if let Some(c) = self.compose.as_mut() {
                c.error = t("Dieses Konto funktioniert nur in der Desktop-Version.").into();
            }
            return;
        }
        match self.build_outgoing(&acc) {
            Err(e) => {
                if let Some(c) = self.compose.as_mut() {
                    c.error = e;
                }
            }
            Ok(msg) => {
                let role_id = |role: Role| self.saved.cache(acc_id).and_then(|c| c.folders.iter().find(|f| f.role == role)).map(|f| f.id.clone());
                let cmd = Cmd::Send { msg, sent_folder: role_id(Role::Sent), drafts_folder: role_id(Role::Drafts) };
                if let Some(c) = self.compose.as_mut() {
                    c.sending = true;
                    c.error.clear();
                }
                self.pending += 1;
                self.backend.send(&acc, cmd);
            }
        }
    }
}

fn addr_list_text(v: &[Address]) -> String {
    v.iter()
        .map(|a| {
            if a.name.contains(',') || a.name.contains(';') {
                format!("\"{}\" <{}>", a.name.replace('"', ""), a.email)
            } else {
                a.full()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn guess_type(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

// ================================================================ Datum in der gewählten Sprache

const EN_WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const EN_MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn en_month(m: i64) -> &'static str {
    EN_MONTHS.get((m - 1).max(0) as usize).copied().unwrap_or("")
}

fn hour_min(loc: i64) -> String {
    let rem = loc.rem_euclid(86400);
    format!("{:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

/// Kurze Zeit für die Liste (Deutsch aus `timeutil`, Englisch hier).
fn short_date(ts: i64, now: i64) -> String {
    if !is_en() {
        return timeutil::short_label(ts, now);
    }
    let loc = ts + timeutil::local_offset(ts);
    let diff = timeutil::local_day(now) - timeutil::local_day(ts);
    if diff <= 0 {
        hour_min(loc)
    } else if diff < 7 {
        let wd = timeutil::weekday(loc.div_euclid(86400));
        EN_WEEKDAYS.get(wd.max(0) as usize).copied().unwrap_or("").to_string()
    } else {
        let (y, m, d) = timeutil::civil_from_days(loc.div_euclid(86400));
        let (ny, _, _) = timeutil::civil_from_days(timeutil::local_day(now));
        if y == ny {
            format!("{d} {}", en_month(m))
        } else {
            format!("{d} {} {y}", en_month(m))
        }
    }
}

/// Langes Datum für die Leseansicht (Deutsch aus `timeutil`, Englisch hier).
fn long_date(ts: i64, now: i64) -> String {
    if !is_en() {
        return timeutil::long_label(ts, now);
    }
    let loc = ts + timeutil::local_offset(ts);
    let hm = hour_min(loc);
    match timeutil::local_day(now) - timeutil::local_day(ts) {
        i64::MIN..=0 => format!("Today, {hm}"),
        1 => format!("Yesterday, {hm}"),
        _ => {
            let (y, m, d) = timeutil::civil_from_days(loc.div_euclid(86400));
            format!("{d} {} {y}, {hm}", en_month(m))
        }
    }
}

// ================================================================ Zeichnen: Bausteine

fn avatar_color(key: &str) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(0x0B, 0x7A, 0x70),
        Color32::from_rgb(0x5B, 0x8D, 0xEF),
        Color32::from_rgb(0x7A, 0x5A, 0xC9),
        Color32::from_rgb(0xB5, 0x54, 0x1C),
        Color32::from_rgb(0xC0, 0x39, 0x6B),
        Color32::from_rgb(0x4F, 0x8F, 0x3F),
    ];
    let sum: usize = key.bytes().map(|b| b as usize).sum();
    PALETTE[sum % PALETTE.len()]
}

/// Text auf eine Breite kürzen («…»), einzeilig.
fn fit(painter: &egui::Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let text = text.replace('\n', " ");
    let g = painter.layout_no_wrap(text.clone(), font.clone(), color);
    if g.size().x <= max_w || text.is_empty() {
        return g;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = ((chars.len() as f32) * max_w / g.size().x.max(1.0)) as usize;
    n = n.min(chars.len()).saturating_sub(1);
    loop {
        let mut t: String = chars[..n].iter().collect();
        t = t.trim_end().to_string();
        t.push('…');
        let g2 = painter.layout_no_wrap(t, font.clone(), color);
        if g2.size().x <= max_w || n == 0 {
            return g2;
        }
        n -= (n / 20).max(1);
    }
}

fn avatar(painter: &egui::Painter, center: egui::Pos2, radius: f32, name: &str, key: &str) {
    painter.circle_filled(center, radius, avatar_color(key));
    painter.text(center, Align2::CENTER_CENTER, textutil::initials(name), FontId::proportional(radius * 0.78), Color32::WHITE);
}

/// Zeile in der Seitenleiste über die volle Breite, optional mit Zahl rechts.
fn nav_row(ui: &mut egui::Ui, selected: bool, text: &str, badge: u32, indent: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let vis = ui.visuals();
        let p = ui.painter_at(rect);
        if selected {
            p.rect_filled(rect, 6.0, vis.selection.bg_fill.gamma_multiply(0.25));
        } else if resp.hovered() {
            p.rect_filled(rect, 6.0, vis.widgets.hovered.weak_bg_fill);
        }
        let color = if selected { vis.strong_text_color() } else { vis.text_color() };
        let right = rect.max.x - 10.0;
        let mut text_w = right - rect.min.x - 10.0 - indent;
        if badge > 0 {
            let g = p.layout_no_wrap(badge.to_string(), FontId::proportional(12.5), ACCENT);
            p.galley(pos2(right - g.size().x, rect.center().y - g.size().y / 2.0), g.clone(), ACCENT);
            text_w -= g.size().x + 8.0;
        }
        let g = fit(&p, text, FontId::proportional(14.5), color, text_w);
        p.galley(pos2(rect.min.x + 10.0 + indent, rect.center().y - g.size().y / 2.0), g, color);
    }
    resp
}

fn mail_row(ui: &mut egui::Ui, h: &HeaderRow, selected: bool, now: i64) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let vis = ui.visuals();
    let p = ui.painter_at(rect);
    if selected {
        p.rect_filled(rect, 0.0, vis.selection.bg_fill.gamma_multiply(0.25));
    } else if resp.hovered() {
        p.rect_filled(rect, 0.0, vis.widgets.hovered.weak_bg_fill);
    }
    if h.unread {
        p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, ACCENT);
    }
    avatar(&p, pos2(rect.min.x + 32.0, rect.center().y), 18.0, &h.from_display(), &h.from_email);

    let x0 = rect.min.x + 60.0;
    let right = rect.max.x - 10.0;
    let strong = vis.strong_text_color();
    let normal = vis.text_color();
    let weak = vis.weak_text_color();
    let main = if h.unread { strong } else { normal };

    let tg = p.layout_no_wrap(short_date(h.ts, now), FontId::proportional(12.0), weak);
    let tw = tg.size().x;
    p.galley(pos2(right - tw, rect.min.y + 11.0), tg, weak);

    let star = if h.flagged { "★ " } else { "" };
    let name = format!("{star}{}", h.from_display());
    let g = fit(&p, &name, FontId::proportional(14.5), main, right - tw - 10.0 - x0);
    p.galley(pos2(x0, rect.min.y + 9.0), g, main);

    let subj = if h.has_att { format!("{}  [{}]", h.subject, t("Anhang")) } else { h.subject.clone() };
    let g = fit(&p, if subj.trim().is_empty() { t("(ohne Betreff)") } else { &subj }, FontId::proportional(13.5), main, right - x0);
    p.galley(pos2(x0, rect.min.y + 28.0), g, main);

    if !h.preview.is_empty() {
        let g = fit(&p, &h.preview, FontId::proportional(12.5), weak, right - x0);
        p.galley(pos2(x0, rect.min.y + 46.0), g, weak);
    }
    p.line_segment(
        [pos2(rect.min.x, rect.max.y - 0.5), pos2(rect.max.x, rect.max.y - 0.5)],
        egui::Stroke::new(1.0, vis.widgets.noninteractive.bg_stroke.color),
    );
    resp
}

fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(ACCENT).min_size(vec2(0.0, 30.0)))
}

fn hint_box(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(RichText::new(text).weak());
    });
}

// ================================================================ Zeichnen: Bereiche

impl App {
    fn ui_nav(&self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.heading(RichText::new("Mailwerk").color(ACCENT));
        });
        ui.add_space(8.0);
        let w = ui.available_width();
        if ui.add_enabled(self.cur_acc().is_some(), egui::Button::new(RichText::new(t("Neue Nachricht")).color(Color32::WHITE)).fill(ACCENT).min_size(vec2(w, 34.0))).clicked() {
            acts.push(Act::Compose(Mode::New));
        }
        ui.add_space(10.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for a in &self.saved.accounts {
                let sel = a.id == self.saved.selected;
                if nav_row(ui, sel && self.folder_id.is_empty(), &a.name, 0, 0.0).clicked() {
                    acts.push(Act::SelectAccount(a.id));
                }
                if sel {
                    if let Some(c) = self.saved.cache(a.id) {
                        for f in &c.folders {
                            if nav_row(ui, f.id == self.folder_id, &f.label(), f.unread, 14.0).clicked() {
                                acts.push(Act::SelectFolder(f.id.clone()));
                            }
                        }
                    }
                    if ui.small_button(t("Konto bearbeiten")).clicked() {
                        acts.push(Act::EditAccount(a.id));
                    }
                }
                ui.add_space(4.0);
            }
            ui.add_space(8.0);
            if ui.button(t("Konto hinzufügen")).clicked() {
                acts.push(Act::AddAccount);
            }
        });
    }

    fn ui_list(&self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let Some(acc) = self.cur_acc() else { return };
        if !Self::usable(acc) {
            hint_box(
                ui,
                t("Nur in der Desktop-Version"),
                t("Dieses Konto verwendet IMAP/SMTP. Das kann ein Browser nicht.\nIm Browser gehen das Beispielkonto und Anbieter mit JMAP (z. B. Fastmail)."),
            );
            return;
        }
        let now = timeutil::now_ts();
        let cache = self.saved.cache(acc.id);
        let have = cache.map_or(false, |c| c.folder_id == self.folder_id && !self.folder_id.is_empty());
        let q = self.search.trim().to_lowercase();
        let rows: Vec<&HeaderRow> = if have {
            cache
                .unwrap()
                .headers
                .iter()
                .filter(|h| !self.only_unread || h.unread)
                .filter(|h| q.is_empty() || h.subject.to_lowercase().contains(&q) || h.from_display().to_lowercase().contains(&q) || h.from_email.to_lowercase().contains(&q) || h.preview.to_lowercase().contains(&q))
                .collect()
        } else {
            Vec::new()
        };
        if !have {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                if self.pending > 0 {
                    ui.spinner();
                    ui.label(RichText::new(t("Lade Nachrichten …")).weak());
                } else {
                    ui.label(RichText::new(t("Keinen Ordner gewählt.")).weak());
                }
            });
            return;
        }
        if rows.is_empty() {
            let msg = if q.is_empty() && !self.only_unread { t("Dieser Ordner ist leer.") } else { t("Keine passenden Nachrichten.") };
            hint_box(ui, "", msg);
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut last_group = "";
            for h in rows {
                let g = timeutil::group_label(h.ts, now);
                if g != last_group {
                    last_group = g;
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(t(g)).small().weak());
                    });
                    ui.add_space(2.0);
                }
                let selected = self.sel_mail.as_deref() == Some(h.id.as_str());
                if mail_row(ui, h, selected, now).clicked() {
                    acts.push(Act::SelectMail(h.id.clone()));
                }
            }
        });
    }

    fn ui_reader(&self, ui: &mut egui::Ui, acts: &mut Vec<Act>, narrow: bool) {
        let Some(id) = self.sel_mail.as_deref() else {
            hint_box(ui, t("Keine Nachricht ausgewählt"), t("Wähle links eine Nachricht aus."));
            return;
        };
        if narrow && ui.button(t("Zurück")).clicked() {
            acts.push(Act::Back);
        }
        let Some(b) = self.open.as_ref().filter(|b| b.id == id) else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(RichText::new(t("Lade Nachricht …")).weak());
            });
            return;
        };
        let row = self.cur_acc().and_then(|a| self.saved.cache(a.id)).and_then(|c| c.headers.iter().find(|h| h.id == id));
        let flagged = row.map_or(false, |r| r.flagged);
        let has_archive = self.folder_by_role(Role::Archive).is_some() && self.cur_role() != Role::Archive;

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if primary_button(ui, t("Antworten")).clicked() {
                acts.push(Act::Compose(Mode::Reply(false)));
            }
            if ui.button(t("Allen antworten")).clicked() {
                acts.push(Act::Compose(Mode::Reply(true)));
            }
            if ui.button(t("Weiterleiten")).clicked() {
                acts.push(Act::Compose(Mode::Forward));
            }
            ui.separator();
            if ui.button(if flagged { t("★ Markiert") } else { t("☆ Markieren") }).clicked() {
                acts.push(Act::ToggleFlag);
            }
            if ui.button(t("Ungelesen")).clicked() {
                acts.push(Act::MarkUnread);
            }
            if ui.add_enabled(has_archive, egui::Button::new(t("Archivieren"))).clicked() {
                acts.push(Act::Archive);
            }
            let del = if self.cur_role() == Role::Trash { t("Endgültig löschen") } else { t("Löschen") };
            if ui.button(del).clicked() {
                acts.push(Act::Delete);
            }
        });
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(4.0);
            ui.heading(if b.subject.trim().is_empty() { t("(ohne Betreff)") } else { b.subject.as_str() });
            ui.add_space(8.0);
            let from = b.from.first().cloned().unwrap_or_default();
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                avatar(ui.painter(), r.center(), 20.0, &from.display(), &from.email);
                ui.vertical(|ui| {
                    ui.label(RichText::new(from.display()).strong());
                    if !from.name.is_empty() {
                        ui.label(RichText::new(&from.email).weak().small());
                    }
                    ui.label(RichText::new(long_date(b.ts, timeutil::now_ts())).weak().small());
                });
            });
            ui.add_space(4.0);
            if !b.to.is_empty() {
                ui.label(RichText::new(tf("An: {to}", &[("to", &addr_list_text(&b.to))])).weak().small());
            }
            if !b.cc.is_empty() {
                ui.label(RichText::new(format!("Cc: {}", addr_list_text(&b.cc))).weak().small());
            }
            if !b.atts.is_empty() {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for (i, a) in b.atts.iter().enumerate() {
                        let size = if a.size > 0 { format!(" ({})", textutil::human_size(a.size)) } else { String::new() };
                        if ui.button(format!("{}{}", a.name, size)).on_hover_text(t("Speichern")).clicked() {
                            acts.push(Act::SaveAtt(i));
                        }
                    }
                });
            }
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            if b.text.trim().is_empty() {
                ui.label(RichText::new(t("(Diese Nachricht hat keinen Text.)")).weak());
            } else {
                ui.label(&b.text);
            }
            ui.add_space(20.0);
        });
    }

    fn ui_welcome(&self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| {
            ui.heading(RichText::new("Mailwerk").color(ACCENT).size(32.0));
            ui.add_space(6.0);
            ui.label(t("Dein eigenes Mailprogramm. Ohne Server, ohne Konto bei uns."));
            ui.add_space(24.0);
            if primary_button(ui, t("Konto hinzufügen")).clicked() {
                acts.push(Act::AddAccount);
            }
            ui.add_space(8.0);
            if ui.button(t("Zuerst mit dem Beispielkonto ausprobieren")).clicked() {
                acts.push(Act::AddDemo);
            }
            ui.add_space(24.0);
            let info = if cfg!(target_arch = "wasm32") {
                t("Im Browser gehen Anbieter mit JMAP (z. B. Fastmail). Für Gmail, Outlook, Bluewin & Co. gibt es die Desktop-Version.")
            } else {
                t("Normale Postfächer (IMAP/SMTP) funktionieren direkt. In der Browser-Version gehen nur Anbieter mit JMAP.")
            };
            ui.label(RichText::new(info).weak().small());
        });
    }

    fn ui_setup(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(mut s) = self.setup.take() else { return };
        let width = (ctx.screen_rect().width() - 24.0).clamp(280.0, 520.0);
        let wasm = cfg!(target_arch = "wasm32");
        egui::Window::new(if s.is_new { t("Konto hinzufügen") } else { t("Konto bearbeiten") })
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .default_width(width)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() - 140.0).max(200.0)).show(ui, |ui| {
                    ui.set_width(width - 24.0);
                    if s.acc.kind != Kind::Demo {
                        ui.radio_value(&mut s.acc.kind, Kind::Imap, if wasm { t("Normales Postfach (IMAP/SMTP), nur Desktop") } else { t("Normales Postfach (IMAP/SMTP)") });
                        ui.radio_value(&mut s.acc.kind, Kind::Jmap, t("Anbieter mit JMAP (z. B. Fastmail), auch im Browser"));
                        ui.add_space(6.0);
                    }
                    egui::Grid::new("setup_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label(t("Kontoname"));
                        ui.add(egui::TextEdit::singleline(&mut s.acc.name).hint_text(t("z. B. Privat")).desired_width(f32::INFINITY));
                        ui.end_row();
                        ui.label(t("Dein Name"));
                        ui.add(egui::TextEdit::singleline(&mut s.acc.person).hint_text(t("erscheint als Absender")).desired_width(f32::INFINITY));
                        ui.end_row();
                        ui.label(t("E-Mail"));
                        ui.add(egui::TextEdit::singleline(&mut s.acc.email).hint_text(t("name@beispiel.ch")).desired_width(f32::INFINITY));
                        ui.end_row();
                        if s.acc.kind != Kind::Demo {
                            ui.label(t("Benutzername"));
                            ui.add(egui::TextEdit::singleline(&mut s.acc.user).hint_text(t("leer = E-Mail-Adresse")).desired_width(f32::INFINITY));
                            ui.end_row();
                            ui.label(t("Passwort"));
                            ui.add(egui::TextEdit::singleline(&mut s.acc.password).password(true).desired_width(f32::INFINITY));
                            ui.end_row();
                        }
                    });
                    if s.acc.kind != Kind::Demo {
                        ui.add_space(4.0);
                        if ui.button(t("Server aus der Adresse vorschlagen")).clicked() {
                            acts.push(Act::SuggestServers);
                        }
                        if !s.hint.is_empty() {
                            ui.label(RichText::new(&s.hint).small());
                        }
                        ui.add_space(6.0);
                    }
                    egui::Grid::new("setup_servers").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| match s.acc.kind {
                        Kind::Imap => {
                            ui.label(t("Empfangen (IMAP)"));
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut s.acc.imap_host).desired_width(220.0));
                                ui.add(egui::DragValue::new(&mut s.acc.imap_port).range(1..=65535));
                            });
                            ui.end_row();
                            ui.label(t("Senden (SMTP)"));
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut s.acc.smtp_host).desired_width(220.0));
                                ui.add(egui::DragValue::new(&mut s.acc.smtp_port).range(1..=65535));
                            });
                            ui.end_row();
                            ui.label(t("Verschlüsselung"));
                            egui::ComboBox::from_id_salt("smtp_sec")
                                .selected_text(if s.acc.smtp_security == Security::Tls { "SSL/TLS (Port 465)" } else { "STARTTLS (Port 587)" })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut s.acc.smtp_security, Security::Tls, "SSL/TLS (Port 465)");
                                    ui.selectable_value(&mut s.acc.smtp_security, Security::StartTls, "STARTTLS (Port 587)");
                                });
                            ui.end_row();
                        }
                        Kind::Jmap => {
                            ui.label(t("JMAP-Adresse"));
                            ui.add(egui::TextEdit::singleline(&mut s.acc.jmap_url).hint_text("https://…/jmap/session").desired_width(f32::INFINITY));
                            ui.end_row();
                        }
                        Kind::Demo => {}
                    });
                    ui.add_space(6.0);
                    ui.label(t("Signatur"));
                    ui.add(egui::TextEdit::multiline(&mut s.acc.signature).desired_rows(3).desired_width(f32::INFINITY));
                    if s.acc.kind != Kind::Demo {
                        ui.add_space(6.0);
                        ui.label(RichText::new(t("Hinweis: Das Passwort wird auf diesem Gerät unverschlüsselt gespeichert.")).weak().small());
                    }
                    if wasm && s.acc.kind == Kind::Imap {
                        ui.add_space(4.0);
                        ui.colored_label(ERR, t("Im Browser geht das nicht. Bitte die Desktop-Version verwenden oder einen Anbieter mit JMAP wählen."));
                    }
                    if !s.error.is_empty() {
                        ui.add_space(4.0);
                        ui.colored_label(ERR, &s.error);
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, t("Speichern")).clicked() {
                        acts.push(Act::SaveSetup);
                    }
                    if ui.button(t("Abbrechen")).clicked() {
                        acts.push(Act::CancelSetup);
                    }
                    if !s.is_new && ui.button(RichText::new(t("Konto entfernen")).color(ERR)).clicked() {
                        acts.push(Act::AskDeleteAccount(s.acc.id));
                    }
                });
            });
        // «Speichern» braucht die Eingaben: setup bleibt bis zum Ausführen der Aktion erhalten.
        self.setup = Some(s);
    }

    fn ui_compose(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>, narrow: bool) {
        let Some(mut c) = self.compose.take() else { return };
        let from = self
            .saved
            .accounts
            .iter()
            .find(|a| a.id == c.acc_id)
            .map(|a| format!("{} <{}>", a.person, a.email))
            .unwrap_or_default();
        let width = (ctx.screen_rect().width() - 24.0).clamp(280.0, 620.0);
        let mut win = egui::Window::new(t("Neue Nachricht")).collapsible(false).resizable(false).default_width(width);
        win = if narrow { win.anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)) } else { win.anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -16.0)) };
        win.show(ctx, |ui| {
            ui.set_width(width - 24.0);
            egui::Grid::new("compose_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(t("Von"));
                ui.label(RichText::new(&from).weak());
                ui.end_row();
                ui.label(t("An"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut c.to).desired_width((width - 150.0).max(120.0)));
                    if !c.show_cc && ui.small_button("Cc/Bcc").clicked() {
                        c.show_cc = true;
                    }
                });
                ui.end_row();
                if c.show_cc {
                    ui.label("Cc");
                    ui.add(egui::TextEdit::singleline(&mut c.cc).desired_width(f32::INFINITY));
                    ui.end_row();
                    ui.label("Bcc");
                    ui.add(egui::TextEdit::singleline(&mut c.bcc).desired_width(f32::INFINITY));
                    ui.end_row();
                }
                ui.label(t("Betreff"));
                ui.add(egui::TextEdit::singleline(&mut c.subject).desired_width(f32::INFINITY));
                ui.end_row();
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() - 330.0).max(120.0)).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut c.body).desired_rows(12).desired_width(f32::INFINITY));
            });
            if !c.atts.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (i, (n, d)) in c.atts.iter().enumerate() {
                        if ui.small_button(format!("{} ({})  ✕", n, textutil::human_size(d.len()))).on_hover_text(t("Entfernen")).clicked() {
                            acts.push(Act::RemoveAttachment(i));
                        }
                    }
                });
            }
            if !c.error.is_empty() {
                ui.add_space(4.0);
                ui.colored_label(ERR, &c.error);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!c.sending, egui::Button::new(RichText::new(t("Senden")).color(Color32::WHITE)).fill(ACCENT).min_size(vec2(90.0, 30.0))).clicked() {
                    acts.push(Act::SendCompose);
                }
                if !cfg!(target_arch = "wasm32") && ui.add_enabled(!c.sending, egui::Button::new(t("Anhang …"))).clicked() {
                    acts.push(Act::PickAttachment);
                }
                if ui.add_enabled(!c.sending, egui::Button::new(t("Verwerfen"))).clicked() {
                    acts.push(Act::CloseCompose);
                }
                if c.sending {
                    ui.spinner();
                    ui.label(RichText::new(t("Wird gesendet …")).weak());
                }
            });
        });
        self.compose = Some(c);
    }

    fn ui_confirm_delete(&self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(id) = self.confirm_delete else { return };
        let name = self.saved.accounts.iter().find(|a| a.id == id).map(|a| a.name.clone()).unwrap_or_default();
        egui::Window::new(t("Konto entfernen?")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.label(tf("«{name}» wird aus Mailwerk entfernt. Die Nachrichten auf dem Server bleiben erhalten.", &[("name", &name)]));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(t("Entfernen")).color(ERR)).clicked() {
                    acts.push(Act::DeleteAccount(id));
                }
                if ui.button(t("Abbrechen")).clicked() {
                    acts.push(Act::CancelDelete);
                }
            });
        });
    }
}

// ================================================================ eframe

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_KEY, self.saved.to_json_string());
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Farben: Hervorhebung in der Programmfarbe (einmal je Hell/Dunkel).
        let dark = ctx.style().visuals.dark_mode;
        if self.styled_dark != Some(dark) {
            self.styled_dark = Some(dark);
            ctx.style_mut(|s| {
                s.visuals.selection.bg_fill = ACCENT;
                s.visuals.hyperlink_color = ACCENT;
            });
        }

        // Neue Ereignisse einlesen.
        for ev in self.backend.poll() {
            self.on_event(ev);
        }

        // Alle fünf Minuten automatisch aktualisieren.
        let time = ctx.input(|i| i.time);
        if time - self.last_refresh > REFRESH_SECS {
            self.last_refresh = time;
            if self.pending == 0 && self.compose.as_ref().map_or(true, |c| !c.sending) {
                self.refresh();
            }
        }
        ctx.request_repaint_after(Duration::from_secs(30));

        let narrow = ctx.screen_rect().width() < 820.0;
        let mut acts: Vec<Act> = Vec::new();
        let has_acc = self.cur_acc().is_some();

        if has_acc {
            egui::TopBottomPanel::top("leiste").show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if narrow && ui.button(t("Menü")).clicked() {
                        acts.push(Act::ToggleNav);
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(t("Suchen …")).desired_width(if narrow { 140.0 } else { 260.0 }));
                    ui.checkbox(&mut self.only_unread, t("Ungelesen"));
                    if ui.button(t("Aktualisieren")).clicked() {
                        acts.push(Act::Refresh);
                    }
                    if self.pending > 0 {
                        ui.spinner();
                    }
                });
                ui.add_space(4.0);
            });
        }

        if self.status.is_some() {
            egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                if let Some((text, err)) = &self.status {
                    ui.horizontal_wrapped(|ui| {
                        let r = RichText::new(text);
                        ui.label(if *err { r.color(ERR) } else { r });
                        if ui.small_button("OK").clicked() {
                            acts.push(Act::ClearStatus);
                        }
                    });
                }
            });
        }

        if !has_acc {
            egui::CentralPanel::default().show(ctx, |ui| self.ui_welcome(ui, &mut acts));
        } else if narrow {
            egui::CentralPanel::default().show(ctx, |ui| {
                if self.show_nav {
                    self.ui_nav(ui, &mut acts);
                } else if self.sel_mail.is_some() {
                    self.ui_reader(ui, &mut acts, true);
                } else {
                    self.ui_list(ui, &mut acts);
                }
            });
        } else {
            egui::SidePanel::left("nav").resizable(false).exact_width(220.0).show(ctx, |ui| self.ui_nav(ui, &mut acts));
            egui::SidePanel::left("liste")
                .resizable(true)
                .default_width(380.0)
                .width_range(300.0..=560.0)
                .show(ctx, |ui| self.ui_list(ui, &mut acts));
            egui::CentralPanel::default().show(ctx, |ui| self.ui_reader(ui, &mut acts, false));
        }

        self.ui_setup(ctx, &mut acts);
        self.ui_compose(ctx, &mut acts, narrow);
        self.ui_confirm_delete(ctx, &mut acts);

        for a in acts {
            self.act(a);
        }
    }
}
