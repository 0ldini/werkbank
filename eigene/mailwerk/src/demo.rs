//! Beispielkonto: Übungsmails im Speicher, ohne Netzwerk. Antwortet wie ein echter Server.

use crate::mime::{self, Address};
use crate::role::Role;
use crate::types::{AttMeta, Cmd, Ev, FolderInfo, HeaderRow, MailBody};

struct Msg {
    folder: String,
    row: HeaderRow,
    body: MailBody,
}

pub struct DemoStore {
    msgs: Vec<Msg>,
    next: u64,
    now: i64,
}

const FOLDERS: [(&str, &str, Role); 6] = [
    ("inbox", "INBOX", Role::Inbox),
    ("drafts", "Drafts", Role::Drafts),
    ("sent", "Sent", Role::Sent),
    ("archive", "Archive", Role::Archive),
    ("junk", "Junk", Role::Junk),
    ("trash", "Trash", Role::Trash),
];

impl DemoStore {
    pub fn new(now: i64) -> DemoStore {
        let mut d = DemoStore { msgs: Vec::new(), next: 1, now };
        let h = 3600;
        let day = 86_400;
        d.add("inbox", "Bea Keller", "bea@keller-treuhand.example", "Offerte Hundekurs Frühling", now - h, true,
            "Grüezi Oli\n\nVielen Dank für die Offerte. Wir würden den Kurs gerne im Frühling starten. Könnten Sie uns drei mögliche Termine nennen?\n\nFreundliche Grüsse\nBea Keller", None);
        d.add("inbox", "Marco Rossi", "marco@rossi-druck.example", "Rechnung 2026-114", now - 5 * h, true,
            "Hallo Oli\n\nIm Anhang die Rechnung für die Flyer. Zahlbar innert 30 Tagen.\n\nBesten Dank und Gruss\nMarco", Some(("Rechnung-2026-114.pdf", "application/pdf", 48_213)));
        d.add("inbox", "Werkbank", "info@werkbank.example", "Willkommen bei Mailwerk", now - 26 * h, false,
            "Das ist ein Beispielkonto. Hier kannst du Mailwerk ausprobieren, ohne ein echtes Postfach zu verbinden.\n\nKlicke links auf «Konto hinzufügen», um dein eigenes Postfach einzurichten.", None);
        d.add("inbox", "Sandra Meier", "sandra@meier-physio.example", "Re: Termin nächste Woche", now - 2 * day - h, false,
            "Hallo Oli\n\nDienstag um 16 Uhr würde bei mir passen. Passt das auch für dich?\n\nLiebe Grüsse\nSandra", None);
        d.add("inbox", "Tim Brunner", "tim@brunner.example", "Foto vom Wochenende", now - 3 * day, false,
            "Schau mal, was ich gefunden habe. War ein super Ausflug!\n\nTim", Some(("ausflug.jpg", "image/jpeg", 1_204_551)));
        d.add("inbox", "Gemeinde Musterlingen", "kanzlei@musterlingen.example", "Abfallkalender 2027", now - 6 * day, false,
            "Guten Tag\n\nDer neue Abfallkalender liegt zum Download bereit.\n\nGemeindekanzlei Musterlingen", None);
        d.add("archive", "Lisa Frei", "lisa@frei.example", "Danke für den Tipp", now - 20 * day, false, "Hat super geklappt, danke!\n\nLisa", None);
        d.add("sent", "Oli", "oli@beispiel.example", "Offerte Hundekurs", now - 2 * day, false, "Guten Tag Bea\n\nIm Anhang meine Offerte.\n\nFreundliche Grüsse\nOli", None);
        d.add("junk", "Gewinnspiel", "gewinn@spam.example", "Sie haben gewonnen!!!", now - 4 * day, true, "Klicken Sie hier …", None);
        d.add("trash", "Alt", "alt@beispiel.example", "Alte Notiz", now - 40 * day, false, "Kann weg.", None);
        d
    }

    fn add(&mut self, folder: &str, name: &str, email: &str, subject: &str, ts: i64, unread: bool, text: &str, att: Option<(&str, &str, usize)>) {
        let id = self.next.to_string();
        self.next += 1;
        let atts: Vec<AttMeta> = att
            .map(|(n, t, s)| AttMeta { name: n.into(), content_type: t.into(), size: s, blob: String::new(), data: format!("Beispieldatei {n}\n").into_bytes() })
            .into_iter()
            .collect();
        let body = MailBody {
            id: id.clone(),
            subject: subject.into(),
            from: vec![Address::new(name, email)],
            to: vec![Address::new("Oli", "oli@beispiel.example")],
            ts,
            message_id: format!("<demo{id}@beispiel.example>"),
            text: text.into(),
            atts,
            ..Default::default()
        };
        let row = HeaderRow {
            id,
            from_name: name.into(),
            from_email: email.into(),
            subject: subject.into(),
            ts,
            unread,
            flagged: false,
            has_att: !body.atts.is_empty(),
            preview: mime::make_preview(text, 110),
        };
        self.msgs.push(Msg { folder: folder.into(), row, body });
    }

    fn folders(&self) -> Vec<FolderInfo> {
        FOLDERS
            .iter()
            .map(|(id, name, role)| {
                let in_f = self.msgs.iter().filter(|m| m.folder == *id);
                FolderInfo {
                    id: (*id).into(),
                    name: (*name).into(),
                    role: *role,
                    unread: in_f.clone().filter(|m| m.row.unread).count() as u32,
                    total: in_f.count() as u32,
                }
            })
            .collect()
    }

    pub fn handle(&mut self, acc: u64, cmd: Cmd) -> Vec<Ev> {
        let what = cmd.what();
        let done = |note: &str| Ev::Done { acc, what: what.clone(), note: note.to_string() };
        let fail = |e: &str| Ev::Failed { acc, what: what.clone(), err: e.to_string() };
        match cmd {
            Cmd::Folders => vec![Ev::Folders { acc, folders: self.folders() }],
            Cmd::Headers { folder, count } => {
                let mut rows: Vec<HeaderRow> = self.msgs.iter().filter(|m| m.folder == folder).map(|m| m.row.clone()).collect();
                rows.sort_by(|a, b| b.ts.cmp(&a.ts));
                let total = rows.len() as u32;
                rows.truncate(count as usize);
                vec![Ev::Headers { acc, folder, rows, total }]
            }
            Cmd::Body { folder, id } => match self.msgs.iter().find(|m| m.folder == folder && m.row.id == id) {
                Some(m) => vec![Ev::Body { acc, folder, body: m.body.clone() }],
                None => vec![fail("Nachricht nicht gefunden.")],
            },
            Cmd::SetSeen { folder, id, seen } => {
                if let Some(m) = self.msgs.iter_mut().find(|m| m.folder == folder && m.row.id == id) {
                    m.row.unread = !seen;
                }
                vec![done(""), Ev::Folders { acc, folders: self.folders() }]
            }
            Cmd::SetFlagged { folder, id, flagged } => {
                if let Some(m) = self.msgs.iter_mut().find(|m| m.folder == folder && m.row.id == id) {
                    m.row.flagged = flagged;
                }
                vec![done("")]
            }
            Cmd::MoveTo { folder, id, target } => {
                if let Some(m) = self.msgs.iter_mut().find(|m| m.folder == folder && m.row.id == id) {
                    m.folder = target;
                }
                vec![done(""), Ev::Folders { acc, folders: self.folders() }]
            }
            Cmd::Delete { folder, id, trash } => {
                match trash {
                    Some(t) if t != folder => {
                        if let Some(m) = self.msgs.iter_mut().find(|m| m.folder == folder && m.row.id == id) {
                            m.folder = t;
                        }
                    }
                    _ => self.msgs.retain(|m| !(m.folder == folder && m.row.id == id)),
                }
                vec![done(""), Ev::Folders { acc, folders: self.folders() }]
            }
            Cmd::Send { msg, .. } => {
                if msg.envelope_recipients().is_empty() {
                    return vec![fail("Kein Empfänger angegeben.")];
                }
                let first = msg.to.first().cloned().unwrap_or_default();
                self.add("sent", &first.name, &first.email, &msg.subject, self.now, false, &msg.body, None);
                vec![done("Im Beispielkonto wird nichts wirklich versendet."), Ev::Folders { acc, folders: self.folders() }]
            }
            Cmd::Attachment { name, .. } => {
                let data = self.msgs.iter().flat_map(|m| m.body.atts.iter()).find(|a| a.name == name).map(|a| a.data.clone()).unwrap_or_default();
                vec![Ev::Attachment { acc, name, data }, done("")]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> DemoStore {
        DemoStore::new(1_800_000_000)
    }

    #[test]
    fn ordner_und_kopfdaten() {
        let mut s = d();
        let Ev::Folders { folders, .. } = &s.handle(1, Cmd::Folders)[0] else { panic!() };
        assert_eq!(folders[0].role, Role::Inbox);
        assert_eq!(folders[0].unread, 2);
        let Ev::Headers { rows, total, .. } = &s.handle(1, Cmd::Headers { folder: "inbox".into(), count: 3 })[0] else { panic!() };
        assert_eq!(rows.len(), 3);
        assert_eq!(*total, 6);
        assert!(rows[0].ts >= rows[1].ts);
    }

    #[test]
    fn lesen_verschieben_loeschen() {
        let mut s = d();
        let evs = s.handle(1, Cmd::SetSeen { folder: "inbox".into(), id: "1".into(), seen: true });
        let Ev::Folders { folders, .. } = &evs[1] else { panic!() };
        assert_eq!(folders[0].unread, 1);
        s.handle(1, Cmd::Delete { folder: "inbox".into(), id: "1".into(), trash: Some("trash".into()) });
        let Ev::Headers { rows, .. } = &s.handle(1, Cmd::Headers { folder: "trash".into(), count: 50 })[0] else { panic!() };
        assert_eq!(rows.len(), 2);
        s.handle(1, Cmd::Delete { folder: "trash".into(), id: "1".into(), trash: Some("trash".into()) });
        let Ev::Headers { rows, .. } = &s.handle(1, Cmd::Headers { folder: "trash".into(), count: 50 })[0] else { panic!() };
        assert_eq!(rows.len(), 1);
        assert!(matches!(s.handle(1, Cmd::Body { folder: "inbox".into(), id: "99".into() })[0], Ev::Failed { .. }));
    }

    #[test]
    fn senden() {
        let mut s = d();
        let mut m = mime::Outgoing::default();
        assert!(matches!(s.handle(1, Cmd::Send { msg: m.clone(), sent_folder: None, drafts_folder: None })[0], Ev::Failed { .. }));
        m.to = vec![Address::new("Bea", "bea@x.ch")];
        m.subject = "Hallo".into();
        m.body = "Test".into();
        assert!(matches!(s.handle(1, Cmd::Send { msg: m, sent_folder: None, drafts_folder: None })[0], Ev::Done { .. }));
        let Ev::Headers { total, .. } = &s.handle(1, Cmd::Headers { folder: "sent".into(), count: 50 })[0] else { panic!() };
        assert_eq!(*total, 2);
    }
}
