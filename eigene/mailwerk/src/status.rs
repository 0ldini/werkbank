//! Statusdatei für das «Mail»-Widget im Werkbank-Launcher (nur Desktop).
//!
//! Nach jedem Abruf schreibt Mailwerk `mailwerk-status.json` in den Einstellungsordner der
//! Werkbank (neben `sprache.txt`):
//! `{"unread": n, "items": [{"from": "…", "subject": "…", "when": ts}], "updated": ts}`.
//! `unread` zählt die Ungelesenen im Posteingang über alle Konten, `items` sind die neuesten bis
//! zu fünf ungelesenen Nachrichten (nur Absender, Betreff, Zeit – keine Inhalte, keine Passwörter).
//! Geschrieben wird atomar (erst `.tmp`, dann umbenennen); Fehler werden still übergangen.

use crate::json::{self, Json};
use crate::role::Role;
use crate::store::Saved;

const MAX_ITEMS: usize = 5;

/// Inhalt der Statusdatei aus dem Zwischenspeicher aller Konten.
pub fn build(saved: &Saved, now: i64) -> String {
    let mut unread: i64 = 0;
    let mut rows: Vec<(i64, String, String)> = Vec::new();
    for acc in &saved.accounts {
        let Some(c) = saved.cache(acc.id) else { continue };
        let Some(inbox) = c.folders.iter().find(|f| f.role == Role::Inbox) else { continue };
        unread = unread.saturating_add(i64::from(inbox.unread));
        // Die Kopfzeilen im Zwischenspeicher gehören zu genau einem Ordner – nur wenn es der
        // Posteingang ist, kommen sie in die Liste.
        if c.folder_id == inbox.id {
            for h in c.headers.iter().filter(|h| h.unread) {
                rows.push((h.ts, h.from_display(), h.subject.clone()));
            }
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    let items: Vec<Json> = rows
        .into_iter()
        .take(MAX_ITEMS)
        .map(|(ts, from, subject)| json::obj(vec![("from", json::s(&from)), ("subject", json::s(&subject)), ("when", json::n(ts))]))
        .collect();
    json::obj(vec![("unread", json::n(unread)), ("items", Json::Arr(items)), ("updated", json::n(now))]).to_string()
}

/// Schreibt die Statusdatei (Desktop). Im Browser tut es nichts.
pub fn write(saved: &Saved, now: i64) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(dir) = crate::sprache::werkbank_file().and_then(|p| p.parent().map(|d| d.to_path_buf())) else { return };
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let text = build(saved, now);
        let tmp = dir.join("mailwerk-status.json.tmp");
        let dest = dir.join("mailwerk-status.json");
        if std::fs::write(&tmp, text.as_bytes()).is_ok() && std::fs::rename(&tmp, &dest).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (saved, now);
    }
}
