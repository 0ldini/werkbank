//! Datentypen von Mailwerk (Konten, Ordner, Nachrichten, Befehle) samt
//! Umwandlung von/nach JSON für die lokale Speicherung. Nur Standardbibliothek.

use crate::json::{self, Json};
use crate::mime::{self, Address};
use crate::role::Role;
use crate::sprache::t;

// ================================================================ Konten

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Beispielkonto mit Übungsmails, ohne Netzwerk.
    Demo,
    /// Normales Postfach über IMAP (lesen) und SMTP (senden). Nur Desktop.
    Imap,
    /// JMAP (z. B. Fastmail): geht auch im Browser.
    Jmap,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Demo => "demo",
            Kind::Imap => "imap",
            Kind::Jmap => "jmap",
        }
    }
    fn parse(s: &str) -> Kind {
        match s {
            "imap" => Kind::Imap,
            "jmap" => Kind::Jmap,
            _ => Kind::Demo,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// Verschlüsselt von Anfang an (Port 465 / 993).
    Tls,
    /// Erst unverschlüsselt, dann mit STARTTLS umschalten (Port 587).
    StartTls,
}

#[derive(Clone, Debug)]
pub struct Account {
    pub id: u64,
    /// Name des Kontos in der Seitenleiste («Hundeschule»).
    pub name: String,
    /// Absendername («Oli»).
    pub person: String,
    pub email: String,
    pub kind: Kind,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    pub user: String,
    pub password: String,
    /// Adresse der JMAP-Sitzung, z. B. https://api.fastmail.com/jmap/session
    pub jmap_url: String,
    pub signature: String,
}

impl Account {
    pub fn blank(id: u64) -> Account {
        Account {
            id,
            name: String::new(),
            person: String::new(),
            email: String::new(),
            kind: Kind::Imap,
            imap_host: String::new(),
            imap_port: 993,
            smtp_host: String::new(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            user: String::new(),
            password: String::new(),
            jmap_url: String::new(),
            signature: String::new(),
        }
    }

    pub fn demo(id: u64) -> Account {
        Account {
            name: t("Beispielkonto").into(),
            person: "Oli".into(),
            email: "oli@beispiel.example".into(),
            kind: Kind::Demo,
            ..Account::blank(id)
        }
    }

    pub fn from_address(&self) -> Address {
        Address::new(&self.person, &self.email)
    }

    /// Läuft dieses Konto auch in der Browser-Version?
    pub fn works_in_browser(&self) -> bool {
        matches!(self.kind, Kind::Demo | Kind::Jmap)
    }

    /// Name für EHLO: Domain der eigenen Adresse.
    pub fn helo_name(&self) -> String {
        self.email.split('@').nth(1).filter(|d| d.contains('.')).unwrap_or("mailwerk.local").to_string()
    }

    /// Fehlt noch etwas Wichtiges? Meldung für den Einrichtungsdialog.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err(t("Bitte einen Namen für das Konto eingeben.").into());
        }
        if !self.email.contains('@') {
            return Err(t("Bitte eine gültige E-Mail-Adresse eingeben.").into());
        }
        match self.kind {
            Kind::Demo => Ok(()),
            Kind::Imap => {
                if self.imap_host.trim().is_empty() || self.smtp_host.trim().is_empty() {
                    return Err(t("Bitte Server für Empfangen (IMAP) und Senden (SMTP) eingeben.").into());
                }
                if self.password.is_empty() {
                    return Err(t("Bitte das Passwort eingeben.").into());
                }
                Ok(())
            }
            Kind::Jmap => {
                if !self.jmap_url.starts_with("https://") {
                    return Err(t("Die JMAP-Adresse muss mit https:// beginnen.").into());
                }
                if self.password.is_empty() {
                    return Err(t("Bitte das Passwort oder den Zugangsschlüssel eingeben.").into());
                }
                Ok(())
            }
        }
    }

    pub fn to_json(&self) -> Json {
        json::obj(vec![
            ("id", Json::Num(self.id as f64)),
            ("name", json::s(&self.name)),
            ("person", json::s(&self.person)),
            ("email", json::s(&self.email)),
            ("kind", json::s(self.kind.as_str())),
            ("imap_host", json::s(&self.imap_host)),
            ("imap_port", json::n(self.imap_port as i64)),
            ("smtp_host", json::s(&self.smtp_host)),
            ("smtp_port", json::n(self.smtp_port as i64)),
            ("smtp_starttls", json::b(self.smtp_security == Security::StartTls)),
            ("user", json::s(&self.user)),
            ("password", json::s(&self.password)),
            ("jmap_url", json::s(&self.jmap_url)),
            ("signature", json::s(&self.signature)),
        ])
    }

    pub fn from_json(j: &Json) -> Option<Account> {
        let id = j.get("id").as_i64()? as u64;
        let port = |k: &str, d: u16| j.get(k).as_i64().filter(|p| (1..=65535).contains(p)).map(|p| p as u16).unwrap_or(d);
        Some(Account {
            id,
            name: j.get("name").str_or_empty(),
            person: j.get("person").str_or_empty(),
            email: j.get("email").str_or_empty(),
            kind: Kind::parse(j.get("kind").as_str().unwrap_or("")),
            imap_host: j.get("imap_host").str_or_empty(),
            imap_port: port("imap_port", 993),
            smtp_host: j.get("smtp_host").str_or_empty(),
            smtp_port: port("smtp_port", 465),
            smtp_security: if j.get("smtp_starttls").as_bool() { Security::StartTls } else { Security::Tls },
            user: j.get("user").str_or_empty(),
            password: j.get("password").str_or_empty(),
            jmap_url: j.get("jmap_url").str_or_empty(),
            signature: j.get("signature").str_or_empty(),
        })
    }
}

// ================================================================ Ordner und Nachrichten

#[derive(Clone, Debug, PartialEq)]
pub struct FolderInfo {
    /// IMAP: Ordnername wie vom Server; JMAP: Postfach-Kennung.
    pub id: String,
    pub name: String,
    pub role: Role,
    pub unread: u32,
    pub total: u32,
}

impl FolderInfo {
    pub fn to_json(&self) -> Json {
        json::obj(vec![
            ("id", json::s(&self.id)),
            ("name", json::s(&self.name)),
            ("role", json::s(self.role.as_str())),
            ("unread", json::n(self.unread as i64)),
            ("total", json::n(self.total as i64)),
        ])
    }
    pub fn from_json(j: &Json) -> Option<FolderInfo> {
        Some(FolderInfo {
            id: j.get("id").as_str()?.to_string(),
            name: j.get("name").str_or_empty(),
            role: Role::from_str(j.get("role").as_str().unwrap_or("")),
            unread: j.get("unread").as_i64().unwrap_or(0).max(0) as u32,
            total: j.get("total").as_i64().unwrap_or(0).max(0) as u32,
        })
    }
    /// Anzeigename: Standardordner deutsch, sonst der Servername (letzter Teil).
    pub fn label(&self) -> String {
        match self.role.label() {
            Some(l) => t(l).to_string(),
            None => self.name.rsplit(|c| c == '/' || c == '.').next().unwrap_or(&self.name).to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeaderRow {
    pub id: String,
    pub from_name: String,
    pub from_email: String,
    pub subject: String,
    pub ts: i64,
    pub unread: bool,
    pub flagged: bool,
    pub has_att: bool,
    pub preview: String,
}

impl HeaderRow {
    pub fn from_display(&self) -> String {
        if self.from_name.is_empty() {
            self.from_email.clone()
        } else {
            self.from_name.clone()
        }
    }
    pub fn to_json(&self) -> Json {
        json::obj(vec![
            ("id", json::s(&self.id)),
            ("fn", json::s(&self.from_name)),
            ("fe", json::s(&self.from_email)),
            ("su", json::s(&self.subject)),
            ("ts", Json::Num(self.ts as f64)),
            ("un", json::b(self.unread)),
            ("fl", json::b(self.flagged)),
            ("at", json::b(self.has_att)),
            ("pv", json::s(&self.preview)),
        ])
    }
    pub fn from_json(j: &Json) -> Option<HeaderRow> {
        Some(HeaderRow {
            id: j.get("id").as_str()?.to_string(),
            from_name: j.get("fn").str_or_empty(),
            from_email: j.get("fe").str_or_empty(),
            subject: j.get("su").str_or_empty(),
            ts: j.get("ts").as_i64().unwrap_or(0),
            unread: j.get("un").as_bool(),
            flagged: j.get("fl").as_bool(),
            has_att: j.get("at").as_bool(),
            preview: j.get("pv").str_or_empty(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttMeta {
    pub name: String,
    pub content_type: String,
    pub size: usize,
    /// JMAP: Kennung des Inhalts (zum Herunterladen).
    pub blob: String,
    /// Inhalt, falls schon geladen (IMAP). Wird nicht gespeichert.
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MailBody {
    pub id: String,
    pub subject: String,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub ts: i64,
    pub message_id: String,
    pub in_reply_to: String,
    pub references: String,
    pub text: String,
    pub atts: Vec<AttMeta>,
}

fn addrs_json(v: &[Address]) -> Json {
    Json::Arr(v.iter().map(|a| json::arr(vec![json::s(&a.name), json::s(&a.email)])).collect())
}
fn addrs_from(j: &Json) -> Vec<Address> {
    j.as_arr().iter().map(|a| Address { name: a.at(0).str_or_empty(), email: a.at(1).str_or_empty() }).collect()
}

impl MailBody {
    /// Aus einer geparsten Mail (IMAP).
    pub fn from_parsed(id: &str, p: &mime::Parsed, now: i64) -> MailBody {
        MailBody {
            id: id.to_string(),
            subject: p.subject.clone(),
            from: p.from.clone(),
            to: p.to.clone(),
            cc: p.cc.clone(),
            reply_to: p.reply_to.clone(),
            ts: p.date_ts.unwrap_or(now),
            message_id: p.message_id.clone(),
            in_reply_to: p.in_reply_to.clone(),
            references: p.references.clone(),
            text: p.text.clone(),
            atts: p
                .attachments
                .iter()
                .map(|a| AttMeta { name: a.filename.clone(), content_type: a.content_type.clone(), size: a.data.len(), blob: String::new(), data: a.data.clone() })
                .collect(),
        }
    }

    /// Für das Parsen von Antwort/Weiterleitung wieder in die Mail-Form bringen.
    pub fn as_parsed(&self) -> mime::Parsed {
        mime::Parsed {
            subject: self.subject.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
            cc: self.cc.clone(),
            reply_to: self.reply_to.clone(),
            date_ts: Some(self.ts),
            message_id: self.message_id.clone(),
            in_reply_to: self.in_reply_to.clone(),
            references: self.references.clone(),
            text: self.text.clone(),
            ..Default::default()
        }
    }

    pub fn to_json(&self, max_text: usize) -> Json {
        let text: String = if self.text.chars().count() > max_text { self.text.chars().take(max_text).collect() } else { self.text.clone() };
        json::obj(vec![
            ("id", json::s(&self.id)),
            ("su", json::s(&self.subject)),
            ("fr", addrs_json(&self.from)),
            ("to", addrs_json(&self.to)),
            ("cc", addrs_json(&self.cc)),
            ("rt", addrs_json(&self.reply_to)),
            ("ts", Json::Num(self.ts as f64)),
            ("mi", json::s(&self.message_id)),
            ("ir", json::s(&self.in_reply_to)),
            ("re", json::s(&self.references)),
            ("tx", json::s(&text)),
            (
                "at",
                Json::Arr(
                    self.atts
                        .iter()
                        .map(|a| json::obj(vec![("n", json::s(&a.name)), ("t", json::s(&a.content_type)), ("s", json::n(a.size as i64)), ("b", json::s(&a.blob))]))
                        .collect(),
                ),
            ),
        ])
    }

    pub fn from_json(j: &Json) -> Option<MailBody> {
        Some(MailBody {
            id: j.get("id").as_str()?.to_string(),
            subject: j.get("su").str_or_empty(),
            from: addrs_from(j.get("fr")),
            to: addrs_from(j.get("to")),
            cc: addrs_from(j.get("cc")),
            reply_to: addrs_from(j.get("rt")),
            ts: j.get("ts").as_i64().unwrap_or(0),
            message_id: j.get("mi").str_or_empty(),
            in_reply_to: j.get("ir").str_or_empty(),
            references: j.get("re").str_or_empty(),
            text: j.get("tx").str_or_empty(),
            atts: j
                .get("at")
                .as_arr()
                .iter()
                .map(|a| AttMeta {
                    name: a.get("n").str_or_empty(),
                    content_type: a.get("t").str_or_empty(),
                    size: a.get("s").as_i64().unwrap_or(0).max(0) as usize,
                    blob: a.get("b").str_or_empty(),
                    data: Vec::new(),
                })
                .collect(),
        })
    }
}

// ================================================================ Befehle und Ereignisse

#[derive(Clone, Debug)]
pub enum Cmd {
    Folders,
    /// Die neuesten `count` Nachrichten eines Ordners.
    Headers { folder: String, count: u32 },
    Body { folder: String, id: String },
    SetSeen { folder: String, id: String, seen: bool },
    SetFlagged { folder: String, id: String, flagged: bool },
    MoveTo { folder: String, id: String, target: String },
    /// `trash`: Papierkorb-Ordner; fehlt er (oder liegt die Mail schon dort), wird endgültig gelöscht.
    Delete { folder: String, id: String, trash: Option<String> },
    Send { msg: mime::Outgoing, sent_folder: Option<String>, drafts_folder: Option<String> },
    /// Anhang herunterladen (nur JMAP; bei IMAP liegt der Inhalt schon in der Nachricht).
    Attachment { blob: String, name: String, content_type: String },
}

impl Cmd {
    /// Kurzname des Befehls, kommt in `Ev::Done`/`Ev::Failed` als `what` zurück.
    pub fn what(&self) -> String {
        match self {
            Cmd::Folders => "folders".into(),
            Cmd::Headers { folder, .. } => format!("headers:{folder}"),
            Cmd::Body { id, .. } => format!("body:{id}"),
            Cmd::SetSeen { id, .. } => format!("seen:{id}"),
            Cmd::SetFlagged { id, .. } => format!("flag:{id}"),
            Cmd::MoveTo { id, .. } => format!("move:{id}"),
            Cmd::Delete { id, .. } => format!("delete:{id}"),
            Cmd::Send { .. } => "send".into(),
            Cmd::Attachment { name, .. } => format!("attachment:{name}"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Ev {
    Folders { acc: u64, folders: Vec<FolderInfo> },
    Headers { acc: u64, folder: String, rows: Vec<HeaderRow>, total: u32 },
    Body { acc: u64, folder: String, body: MailBody },
    Attachment { acc: u64, name: String, data: Vec<u8> },
    /// Ein Befehl ist fertig. `what` sagt welcher, `note` ist ein Hinweis (leer = alles gut).
    Done { acc: u64, what: String, note: String },
    Failed { acc: u64, what: String, err: String },
}

// ================================================================ Zwischenspeicher je Konto

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccCache {
    pub folders: Vec<FolderInfo>,
    /// Ordner, zu dem `headers` gehören (meist der Posteingang).
    pub folder_id: String,
    pub headers: Vec<HeaderRow>,
    pub bodies: Vec<MailBody>,
}

pub const CACHE_MAX_HEADERS: usize = 200;
pub const CACHE_MAX_BODIES: usize = 30;
pub const CACHE_MAX_TEXT: usize = 20_000;

impl AccCache {
    pub fn to_json(&self) -> Json {
        json::obj(vec![
            ("folders", Json::Arr(self.folders.iter().map(|f| f.to_json()).collect())),
            ("folder_id", json::s(&self.folder_id)),
            ("headers", Json::Arr(self.headers.iter().take(CACHE_MAX_HEADERS).map(|h| h.to_json()).collect())),
            ("bodies", Json::Arr(self.bodies.iter().take(CACHE_MAX_BODIES).map(|b| b.to_json(CACHE_MAX_TEXT)).collect())),
        ])
    }
    pub fn from_json(j: &Json) -> AccCache {
        AccCache {
            folders: j.get("folders").as_arr().iter().filter_map(FolderInfo::from_json).collect(),
            folder_id: j.get("folder_id").str_or_empty(),
            headers: j.get("headers").as_arr().iter().filter_map(HeaderRow::from_json).collect(),
            bodies: j.get("bodies").as_arr().iter().filter_map(MailBody::from_json).collect(),
        }
    }
    /// Nachricht merken (neueste zuerst, höchstens `CACHE_MAX_BODIES`).
    pub fn remember_body(&mut self, b: &MailBody) {
        self.bodies.retain(|x| x.id != b.id);
        self.bodies.insert(0, b.clone());
        self.bodies.truncate(CACHE_MAX_BODIES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn konto_rundreise() {
        let mut a = Account::blank(7);
        a.name = "Hundeschule".into();
        a.person = "Oli".into();
        a.email = "oli@adilios.example".into();
        a.imap_host = "mail.example.org".into();
        a.smtp_host = "mail.example.org".into();
        a.smtp_port = 587;
        a.smtp_security = Security::StartTls;
        a.password = "geheim \"x\"".into();
        let t = a.to_json().to_string();
        let b = Account::from_json(&Json::parse(&t).unwrap()).unwrap();
        assert_eq!(b.id, 7);
        assert_eq!(b.smtp_port, 587);
        assert_eq!(b.smtp_security, Security::StartTls);
        assert_eq!(b.password, "geheim \"x\"");
        assert_eq!(b.helo_name(), "adilios.example");
        assert!(b.validate().is_ok());
        assert!(!b.works_in_browser());
        assert!(Account::demo(1).works_in_browser());
        let mut c = b.clone();
        c.password.clear();
        assert!(c.validate().is_err());
    }

    #[test]
    fn cache_rundreise_und_grenzen() {
        let mut c = AccCache::default();
        c.folder_id = "INBOX".into();
        c.folders.push(FolderInfo { id: "INBOX".into(), name: "INBOX".into(), role: Role::Inbox, unread: 3, total: 40 });
        for i in 0..250 {
            c.headers.push(HeaderRow { id: i.to_string(), from_name: "Bea".into(), from_email: "b@x.ch".into(), subject: "Grüezi".into(), ts: 1_800_000_000 + i, unread: i % 2 == 0, flagged: false, has_att: i % 3 == 0, preview: "…".into() });
        }
        let mut p = mime::Parsed::default();
        p.subject = "Hallo".into();
        p.from = vec![Address::new("Bea", "b@x.ch")];
        p.text = "ä".repeat(40_000);
        p.date_ts = Some(5);
        let mut body = MailBody::from_parsed("9", &p, 0);
        body.atts.push(AttMeta { name: "a.pdf".into(), content_type: "application/pdf".into(), size: 10, blob: "B1".into(), data: vec![1, 2] });
        c.remember_body(&body);
        c.remember_body(&body); // doppelt → nur einmal
        assert_eq!(c.bodies.len(), 1);
        let t = c.to_json().to_string();
        let d = AccCache::from_json(&Json::parse(&t).unwrap());
        assert_eq!(d.headers.len(), CACHE_MAX_HEADERS);
        assert_eq!(d.folders[0].unread, 3);
        assert_eq!(d.bodies[0].text.chars().count(), CACHE_MAX_TEXT);
        assert_eq!(d.bodies[0].atts[0].blob, "B1");
        assert!(d.bodies[0].atts[0].data.is_empty());
        assert_eq!(d.bodies[0].from[0].email, "b@x.ch");
        assert_eq!(d.headers[0].from_display(), "Bea");
    }

    #[test]
    fn ordner_beschriftung() {
        crate::sprache::set_lang(crate::sprache::Lang::De);
        let f = FolderInfo { id: "a".into(), name: "INBOX".into(), role: Role::Inbox, unread: 0, total: 0 };
        assert_eq!(f.label(), "Posteingang");
        let g = FolderInfo { id: "a".into(), name: "Kunden/Meier".into(), role: Role::Other, unread: 0, total: 0 };
        assert_eq!(g.label(), "Meier");
    }
}
