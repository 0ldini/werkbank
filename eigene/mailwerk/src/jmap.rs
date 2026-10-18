//! JMAP über HTTP (ehttp: gleiche Aufrufe im Browser und auf dem Desktop).
//! Alles läuft über Rückruf-Funktionen, weil der Browser nicht blockieren darf.

use crate::backend::Out;
use crate::jmap_proto::{self as jp, Session};
use crate::json::Json;
use crate::types::{Account, Cmd, Ev};

/// Antwort auf eine HTTP-Anfrage prüfen und die Bytes liefern.
fn check(res: Result<ehttp::Response, String>) -> Result<Vec<u8>, String> {
    let r = res.map_err(|e| format!("Verbindung fehlgeschlagen: {e}"))?;
    match r.status {
        200..=299 => Ok(r.bytes),
        401 | 403 => Err("Anmeldung abgelehnt. Bitte Benutzername und Passwort (oder Zugangsschlüssel) prüfen.".into()),
        s => Err(format!("Der Server antwortet mit Fehler {s} {}.", r.status_text)),
    }
}

fn request(url: &str, auth: &str, body: Option<String>) -> ehttp::Request {
    let mut req = match body {
        Some(b) => {
            let mut r = ehttp::Request::post(url, b.into_bytes());
            r.headers.insert("Content-Type", "application/json");
            r
        }
        None => ehttp::Request::get(url),
    };
    req.headers.insert("Authorization", auth);
    req
}

fn fetch_json(url: &str, auth: &str, body: Option<String>, cb: impl FnOnce(Result<Json, String>) + Send + 'static) {
    ehttp::fetch(request(url, auth, body), move |res| {
        let r = check(res).and_then(|bytes| Json::parse(&String::from_utf8_lossy(&bytes)));
        cb(r)
    });
}

/// Gemeinsamer Zustand eines laufenden Befehls.
#[derive(Clone)]
struct Job {
    out: Out,
    acc: u64,
    what: String,
    sess: Session,
    auth: String,
}

impl Job {
    fn fail(&self, e: String) {
        self.out.emit(Ev::Failed { acc: self.acc, what: self.what.clone(), err: e });
    }
    fn done(&self, note: &str) {
        self.out.emit(Ev::Done { acc: self.acc, what: self.what.clone(), note: note.to_string() });
    }
    fn api(&self, with_submission: bool, calls: Vec<Json>, cb: impl FnOnce(&Job, Json) + Send + 'static) {
        let job = self.clone();
        fetch_json(&self.sess.api_url, &self.auth, Some(jp::request(with_submission, calls)), move |r| match r {
            Ok(j) => cb(&job, j),
            Err(e) => job.fail(e),
        });
    }
    /// Schreib-Befehl: Antwort prüfen, dann «fertig» melden.
    fn api_set(&self, call: Json, what: &'static str) {
        self.api(false, vec![call], move |job, resp| match jp::check_set(&resp, "s", what) {
            Ok(()) => job.done(""),
            Err(e) => job.fail(e),
        });
    }
}

pub fn run(acc: &Account, cmd: Cmd, out: Out) {
    let acc = acc.clone();
    let what = cmd.what();
    let auth = jp::auth_header(&acc.user, &acc.password);
    let (out2, what2, id) = (out.clone(), what.clone(), acc.id);
    let fail = move |e: String| out2.emit(Ev::Failed { acc: id, what: what2.clone(), err: e });
    if !acc.jmap_url.starts_with("https://") {
        return fail("Die JMAP-Adresse muss mit https:// beginnen.".into());
    }
    let auth2 = auth.clone();
    let url = acc.jmap_url.clone();
    fetch_json(&url, &auth, None, move |r| {
        let sess = match r.and_then(|j| jp::parse_session(&j)) {
            Ok(s) => s,
            Err(e) => return fail(e),
        };
        let job = Job { out, acc: acc.id, what, sess, auth: auth2 };
        dispatch(job, acc, cmd);
    });
}

fn dispatch(job: Job, acc: Account, cmd: Cmd) {
    let aid = job.sess.account_id.clone();
    match cmd {
        Cmd::Folders => job.api(false, vec![jp::call_folders(&aid)], |job, resp| match jp::parse_folders(&resp) {
            Ok(mut f) => {
                f.sort_by(|x, y| x.role.sort_key().cmp(&y.role.sort_key()).then(x.name.to_lowercase().cmp(&y.name.to_lowercase())));
                job.out.emit(Ev::Folders { acc: job.acc, folders: f });
            }
            Err(e) => job.fail(e),
        }),
        Cmd::Headers { folder, count } => {
            let calls = jp::calls_headers(&aid, &folder, count);
            job.api(false, calls, move |job, resp| match jp::parse_headers(&resp) {
                Ok((rows, total)) => job.out.emit(Ev::Headers { acc: job.acc, folder, rows, total }),
                Err(e) => job.fail(e),
            });
        }
        Cmd::Body { folder, id } => job.api(false, vec![jp::call_body(&aid, &id)], move |job, resp| match jp::parse_body(&resp) {
            Ok(body) => job.out.emit(Ev::Body { acc: job.acc, folder, body }),
            Err(e) => job.fail(e),
        }),
        Cmd::SetSeen { id, seen, .. } => job.api_set(jp::call_set_keyword(&aid, &id, "$seen", seen), "Markierung ändern"),
        Cmd::SetFlagged { id, flagged, .. } => job.api_set(jp::call_set_keyword(&aid, &id, "$flagged", flagged), "Markierung ändern"),
        Cmd::MoveTo { folder, id, target } => job.api_set(jp::call_move(&aid, &id, &folder, &target), "Verschieben"),
        Cmd::Delete { folder, id, trash } => match trash {
            Some(t) if t != folder => job.api_set(jp::call_move(&aid, &id, &folder, &t), "Löschen"),
            _ => job.api_set(jp::call_destroy(&aid, &id), "Löschen"),
        },
        Cmd::Send { msg, sent_folder, drafts_folder } => {
            let Some(draft_box) = drafts_folder.or_else(|| sent_folder.clone()) else {
                return job.fail("Beim Anbieter wurde kein Ordner für Entwürfe oder Gesendet gefunden.".into());
            };
            let my_email = acc.email.clone();
            job.api(true, vec![jp::call_identity(&aid)], move |job, resp| {
                let identity = match jp::parse_identity(&resp, &my_email) {
                    Ok(i) => i,
                    Err(e) => return job.fail(e),
                };
                let aid = job.sess.account_id.clone();
                let calls = jp::calls_send(&aid, &identity, &msg, &draft_box, sent_folder.as_deref());
                job.api(true, calls, |job, resp| {
                    match jp::check_set(&resp, "e", "Nachricht anlegen").and_then(|_| jp::check_set(&resp, "x", "Senden")) {
                        Ok(()) => job.done(""),
                        Err(e) => job.fail(e),
                    }
                });
            });
        }
        Cmd::Attachment { blob, name, content_type } => {
            let url = jp::download_url(&job.sess, &blob, &name, &content_type);
            let j = job.clone();
            ehttp::fetch(request(&url, &job.auth, None), move |res| match check(res) {
                Ok(data) => {
                    j.out.emit(Ev::Attachment { acc: j.acc, name, data });
                    j.done("");
                }
                Err(e) => j.fail(e),
            });
        }
    }
}
