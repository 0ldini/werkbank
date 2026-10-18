//! JMAP (RFC 8620/8621): Anfragen bauen und Antworten lesen. Nur Standardbibliothek
//! (mit dem eigenen `json`); die Netzwerk-Verbindung steckt in `jmap.rs`.

use crate::json::{self, arr, b, obj, s, Json};
use crate::mime::{self, Address};
use crate::role::{guess_role, Role};
use crate::textutil;
use crate::timeutil;
use crate::types::{AttMeta, FolderInfo, HeaderRow, MailBody};
use crate::b64;
use crate::sprache::{t, tf};

pub const CAP_CORE: &str = "urn:ietf:params:jmap:core";
pub const CAP_MAIL: &str = "urn:ietf:params:jmap:mail";
pub const CAP_SUBMISSION: &str = "urn:ietf:params:jmap:submission";

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub api_url: String,
    pub account_id: String,
    pub download_url: String,
}

pub fn parse_session(j: &Json) -> Result<Session, String> {
    let api_url = j.get("apiUrl").as_str().ok_or(t("Die JMAP-Sitzung enthält keine apiUrl."))?.to_string();
    let account_id = match j.get("primaryAccounts").get(CAP_MAIL).as_str() {
        Some(a) => a.to_string(),
        None => j
            .get("accounts")
            .entries()
            .first()
            .map(|(k, _)| k.clone())
            .ok_or(t("Die JMAP-Sitzung enthält kein Mail-Konto."))?,
    };
    Ok(Session { api_url, account_id, download_url: j.get("downloadUrl").str_or_empty() })
}

/// Wert für die Kopfzeile «Authorization»: mit Benutzername «Basic», sonst «Bearer» (Zugangsschlüssel).
pub fn auth_header(user: &str, pass: &str) -> String {
    if user.trim().is_empty() {
        format!("Bearer {pass}")
    } else {
        format!("Basic {}", b64::encode(format!("{user}:{pass}").as_bytes()))
    }
}

pub fn request(with_submission: bool, calls: Vec<Json>) -> String {
    let mut using = vec![s(CAP_CORE), s(CAP_MAIL)];
    if with_submission {
        using.push(s(CAP_SUBMISSION));
    }
    obj(vec![("using", arr(using)), ("methodCalls", arr(calls))]).to_string()
}

fn call(name: &str, args: Json, id: &str) -> Json {
    arr(vec![s(name), args, s(id)])
}

// ================================================================ Anfragen

pub fn call_folders(acc: &str) -> Json {
    call("Mailbox/get", obj(vec![("accountId", s(acc))]), "m")
}

/// Die neuesten `limit` Nachrichten eines Postfachs (Kopfdaten und Vorschau).
pub fn calls_headers(acc: &str, mailbox: &str, limit: u32) -> Vec<Json> {
    vec![
        call(
            "Email/query",
            obj(vec![
                ("accountId", s(acc)),
                ("filter", obj(vec![("inMailbox", s(mailbox))])),
                ("sort", arr(vec![obj(vec![("property", s("receivedAt")), ("isAscending", b(false))])])),
                ("position", json::n(0)),
                ("limit", json::n(limit as i64)),
                ("calculateTotal", b(true)),
            ]),
            "q",
        ),
        call(
            "Email/get",
            obj(vec![
                ("accountId", s(acc)),
                ("#ids", obj(vec![("resultOf", s("q")), ("name", s("Email/query")), ("path", s("/ids"))])),
                (
                    "properties",
                    arr(["id", "receivedAt", "from", "subject", "keywords", "hasAttachment", "preview"].iter().map(|p| s(p)).collect()),
                ),
            ]),
            "g",
        ),
    ]
}

pub fn call_body(acc: &str, id: &str) -> Json {
    call(
        "Email/get",
        obj(vec![
            ("accountId", s(acc)),
            ("ids", arr(vec![s(id)])),
            (
                "properties",
                arr([
                    "id", "subject", "from", "to", "cc", "replyTo", "receivedAt", "sentAt", "messageId", "inReplyTo", "references", "bodyValues",
                    "textBody", "htmlBody", "attachments",
                ]
                .iter()
                .map(|p| s(p))
                .collect()),
            ),
            ("fetchTextBodyValues", b(true)),
            ("fetchHTMLBodyValues", b(true)),
            ("maxBodyValueBytes", json::n(300_000)),
        ]),
        "g",
    )
}

/// Kennzeichen setzen oder entfernen, z. B. «$seen» oder «$flagged».
pub fn call_set_keyword(acc: &str, id: &str, keyword: &str, on: bool) -> Json {
    let patch = obj(vec![(&format!("keywords/{keyword}"), if on { b(true) } else { Json::Null })]);
    call("Email/set", obj(vec![("accountId", s(acc)), ("update", obj(vec![(id, patch)]))]), "s")
}

pub fn call_move(acc: &str, id: &str, from: &str, to: &str) -> Json {
    let patch = obj(vec![(&format!("mailboxIds/{from}"), Json::Null), (&format!("mailboxIds/{to}"), b(true))]);
    call("Email/set", obj(vec![("accountId", s(acc)), ("update", obj(vec![(id, patch)]))]), "s")
}

pub fn call_destroy(acc: &str, id: &str) -> Json {
    call("Email/set", obj(vec![("accountId", s(acc)), ("destroy", arr(vec![s(id)]))]), "s")
}

pub fn call_identity(acc: &str) -> Json {
    call("Identity/get", obj(vec![("accountId", s(acc))]), "i")
}

fn addr_json(a: &Address) -> Json {
    if a.name.is_empty() {
        obj(vec![("email", s(&a.email))])
    } else {
        obj(vec![("name", s(&a.name)), ("email", s(&a.email))])
    }
}

fn id_list(refs: &str) -> Json {
    // «<a@x> <b@y>» → ["a@x","b@y"]
    arr(refs.split_whitespace().map(|r| s(r.trim_matches(|c| c == '<' || c == '>'))).filter(|j| j.as_str() != Some("")).collect())
}

/// Entwurf anlegen und gleich abschicken. `draft_box`: Postfach, in dem der Entwurf kurz liegt;
/// `sent_box`: Ziel nach dem Senden (falls vorhanden).
pub fn calls_send(acc: &str, identity: &str, m: &mime::Outgoing, draft_box: &str, sent_box: Option<&str>) -> Vec<Json> {
    let mut email = vec![
        ("mailboxIds", obj(vec![(draft_box, b(true))])),
        ("keywords", obj(vec![("$draft", b(true)), ("$seen", b(true))])),
        ("from", arr(vec![addr_json(&m.from)])),
        ("to", arr(m.to.iter().map(addr_json).collect())),
        ("subject", s(m.subject.trim())),
        ("bodyStructure", obj(vec![("partId", s("1")), ("type", s("text/plain"))])),
        ("bodyValues", obj(vec![("1", obj(vec![("value", s(&m.body.replace("\r\n", "\n")))]))])),
    ];
    if !m.cc.is_empty() {
        email.push(("cc", arr(m.cc.iter().map(addr_json).collect())));
    }
    if !m.bcc.is_empty() {
        email.push(("bcc", arr(m.bcc.iter().map(addr_json).collect())));
    }
    if !m.in_reply_to.is_empty() {
        email.push(("inReplyTo", id_list(&m.in_reply_to)));
    }
    if !m.references.is_empty() {
        email.push(("references", id_list(&m.references)));
    }
    let mut update = vec![("keywords/$draft", Json::Null)];
    let mut update_owned: Vec<(String, Json)> = update.drain(..).map(|(k, v)| (k.to_string(), v)).collect();
    if let Some(sent) = sent_box {
        if sent != draft_box {
            update_owned.push((format!("mailboxIds/{draft_box}"), Json::Null));
            update_owned.push((format!("mailboxIds/{sent}"), b(true)));
        }
    }
    let upd = Json::Obj(update_owned);
    vec![
        call("Email/set", obj(vec![("accountId", s(acc)), ("create", obj(vec![("draft", obj(email))]))]), "e"),
        call(
            "EmailSubmission/set",
            obj(vec![
                ("accountId", s(acc)),
                ("create", obj(vec![("sub", obj(vec![("identityId", s(identity)), ("emailId", s("#draft"))]))])),
                ("onSuccessUpdateEmail", obj(vec![("#sub", upd)])),
            ]),
            "x",
        ),
    ]
}

pub fn download_url(sess: &Session, blob: &str, name: &str, content_type: &str) -> String {
    sess.download_url
        .replace("{accountId}", &textutil::percent_encode(&sess.account_id))
        .replace("{blobId}", &textutil::percent_encode(blob))
        .replace("{name}", &textutil::percent_encode(name))
        .replace("{type}", &textutil::percent_encode(content_type))
}

// ================================================================ Antworten

/// Argumente der Antwort zu `call_id`; Fehlerantworten werden zu einem Text.
pub fn method_args<'a>(resp: &'a Json, call_id: &str) -> Result<&'a Json, String> {
    if let Some(t) = resp.get("type").as_str() {
        // Fehler auf Anfrage-Ebene (kein methodResponses)
        let detail = format!("{} {}", t, resp.get("detail").str_or_empty());
        return Err(tf("Server meldet Fehler: {detail}", &[("detail", &detail)]));
    }
    for r in resp.get("methodResponses").as_arr() {
        if r.at(2).as_str() == Some(call_id) {
            if r.at(0).as_str() == Some("error") {
                let e = r.at(1);
                let detail = format!("{} {}", e.get("type").str_or_empty(), e.get("description").str_or_empty());
                return Err(tf("Server meldet Fehler: {detail}", &[("detail", &detail)]).trim().to_string());
            }
            return Ok(r.at(1));
        }
    }
    Err(tf("Antwort «{call_id}» fehlt in der Server-Antwort.", &[("call_id", &call_id)]))
}

pub fn parse_folders(resp: &Json) -> Result<Vec<FolderInfo>, String> {
    let a = method_args(resp, "m")?;
    let mut out: Vec<FolderInfo> = a
        .get("list")
        .as_arr()
        .iter()
        .filter_map(|m| {
            let id = m.get("id").as_str()?.to_string();
            let name = m.get("name").str_or_empty();
            let role = match m.get("role").as_str() {
                Some("inbox") => Role::Inbox,
                Some("drafts") => Role::Drafts,
                Some("sent") => Role::Sent,
                Some("trash") => Role::Trash,
                Some("junk") => Role::Junk,
                Some("archive") | Some("all") => Role::Archive,
                Some("flagged") => Role::Flagged,
                Some(_) => Role::Other,
                None => guess_role(&name, &[]),
            };
            Some(FolderInfo {
                id,
                name,
                role,
                unread: m.get("unreadEmails").as_i64().unwrap_or(0).max(0) as u32,
                total: m.get("totalEmails").as_i64().unwrap_or(0).max(0) as u32,
            })
        })
        .filter(|f| f.role != Role::Flagged)
        .collect();
    out.sort_by(|x, y| x.role.sort_key().cmp(&y.role.sort_key()).then(x.name.to_lowercase().cmp(&y.name.to_lowercase())));
    Ok(out)
}

pub fn parse_headers(resp: &Json) -> Result<(Vec<HeaderRow>, u32), String> {
    let total = method_args(resp, "q")?.get("total").as_i64().unwrap_or(0).max(0) as u32;
    let g = method_args(resp, "g")?;
    let mut rows: Vec<HeaderRow> = g
        .get("list")
        .as_arr()
        .iter()
        .filter_map(|e| {
            let id = e.get("id").as_str()?.to_string();
            let from = e.get("from").at(0);
            let kw = e.get("keywords");
            Some(HeaderRow {
                id,
                from_name: from.get("name").str_or_empty(),
                from_email: from.get("email").str_or_empty(),
                subject: e.get("subject").str_or_empty(),
                ts: e.get("receivedAt").as_str().and_then(timeutil::parse_iso8601).unwrap_or(0),
                unread: !kw.get("$seen").as_bool(),
                flagged: kw.get("$flagged").as_bool(),
                has_att: e.get("hasAttachment").as_bool(),
                preview: e.get("preview").str_or_empty(),
            })
        })
        .collect();
    rows.sort_by(|a, b| b.ts.cmp(&a.ts));
    Ok((rows, total))
}

fn addr_list(j: &Json) -> Vec<Address> {
    j.as_arr().iter().map(|a| Address { name: a.get("name").str_or_empty(), email: a.get("email").str_or_empty() }).collect()
}

fn ids_to_header(j: &Json) -> String {
    j.as_arr().iter().filter_map(|x| x.as_str()).map(|x| format!("<{x}>")).collect::<Vec<_>>().join(" ")
}

pub fn parse_body(resp: &Json) -> Result<MailBody, String> {
    let g = method_args(resp, "g")?;
    let e = g.get("list").at(0);
    let id = e.get("id").as_str().ok_or(t("Nachricht nicht gefunden (vielleicht gelöscht)."))?.to_string();
    let values = e.get("bodyValues");
    let collect_parts = |parts: &Json| -> Vec<String> {
        parts
            .as_arr()
            .iter()
            .filter_map(|p| {
                let pid = p.get("partId").as_str()?;
                let v = values.get(pid).get("value").as_str()?;
                Some(if p.get("type").as_str() == Some("text/html") { textutil::html_to_text(v) } else { v.to_string() })
            })
            .collect()
    };
    let mut pieces = collect_parts(e.get("textBody"));
    if pieces.iter().all(|p| p.trim().is_empty()) {
        pieces = collect_parts(e.get("htmlBody"));
    }
    let text = pieces.join("\n\n").replace("\r\n", "\n").trim_end().to_string();
    let ts = e
        .get("sentAt")
        .as_str()
        .or_else(|| e.get("receivedAt").as_str())
        .and_then(timeutil::parse_iso8601)
        .unwrap_or(0);
    Ok(MailBody {
        id,
        subject: e.get("subject").str_or_empty(),
        from: addr_list(e.get("from")),
        to: addr_list(e.get("to")),
        cc: addr_list(e.get("cc")),
        reply_to: addr_list(e.get("replyTo")),
        ts,
        message_id: ids_to_header(e.get("messageId")),
        in_reply_to: ids_to_header(e.get("inReplyTo")),
        references: ids_to_header(e.get("references")),
        text,
        atts: e
            .get("attachments")
            .as_arr()
            .iter()
            .enumerate()
            .map(|(i, a)| AttMeta {
                name: a.get("name").as_str().map(|x| x.to_string()).unwrap_or_else(|| format!("anhang{}", i + 1)),
                content_type: a.get("type").str_or_empty(),
                size: a.get("size").as_i64().unwrap_or(0).max(0) as usize,
                blob: a.get("blobId").str_or_empty(),
                data: Vec::new(),
            })
            .collect(),
    })
}

/// Wählt die Absender-Identität: die mit unserer Adresse, sonst die erste.
pub fn parse_identity(resp: &Json, my_email: &str) -> Result<String, String> {
    let a = method_args(resp, "i")?;
    let list = a.get("list").as_arr();
    let pick = list
        .iter()
        .find(|i| i.get("email").as_str().map_or(false, |e| e.eq_ignore_ascii_case(my_email)))
        .or_else(|| list.first());
    pick.and_then(|i| i.get("id").as_str()).map(|x| x.to_string()).ok_or_else(|| t("Keine Absender-Identität beim Anbieter gefunden.").to_string())
}

/// Prüft eine Schreib-Antwort (`Email/set`, `EmailSubmission/set`) auf Fehler.
pub fn check_set(resp: &Json, call_id: &str, what: &str) -> Result<(), String> {
    let a = method_args(resp, call_id)?;
    for key in ["notCreated", "notUpdated", "notDestroyed"] {
        if let Some((k, v)) = a.get(key).entries().first() {
            let t = v.get("type").str_or_empty();
            let d = v.get("description").str_or_empty();
            return Err(format!("{what}: {t} {d} ({k})").replace("  ", " "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(t: &str) -> Json {
        Json::parse(t).unwrap()
    }

    #[test]
    fn sitzung() {
        let j = parse(r#"{"capabilities":{},"accounts":{"u123":{"name":"oli"}},"primaryAccounts":{"urn:ietf:params:jmap:mail":"u123"},"apiUrl":"https://api.example.org/jmap/api/","downloadUrl":"https://dl.example.org/{accountId}/{blobId}/{name}?type={type}"}"#);
        let s = parse_session(&j).unwrap();
        assert_eq!(s.account_id, "u123");
        assert_eq!(s.api_url, "https://api.example.org/jmap/api/");
        assert_eq!(download_url(&s, "B1", "Röntgen bild.pdf", "application/pdf"), "https://dl.example.org/u123/B1/R%C3%B6ntgen%20bild.pdf?type=application%2Fpdf");
        assert!(parse_session(&parse(r#"{"foo":1}"#)).is_err());
        assert_eq!(auth_header("", "tok"), "Bearer tok");
        assert_eq!(auth_header("oli", "pw"), "Basic b2xpOnB3");
    }

    #[test]
    fn anfragen_sind_gueltiges_json() {
        let r = request(false, calls_headers("u1", "mb1", 50));
        let j = parse(&r);
        assert_eq!(j.get("using").at(1).as_str(), Some(CAP_MAIL));
        assert_eq!(j.get("methodCalls").as_arr().len(), 2);
        assert_eq!(j.get("methodCalls").at(0).at(0).as_str(), Some("Email/query"));
        assert_eq!(j.get("methodCalls").at(0).at(1).get("filter").get("inMailbox").as_str(), Some("mb1"));
        assert_eq!(j.get("methodCalls").at(1).at(1).get("#ids").get("path").as_str(), Some("/ids"));
        let m = parse(&request(false, vec![call_move("u1", "e1", "mbA", "mbB")]));
        let patch = m.get("methodCalls").at(0).at(1).get("update").get("e1");
        assert!(patch.get("mailboxIds/mbA").is_null());
        assert_eq!(patch.get("mailboxIds/mbB").as_bool(), true);
        let k = parse(&request(false, vec![call_set_keyword("u1", "e1", "$seen", true)]));
        assert_eq!(k.get("methodCalls").at(0).at(1).get("update").get("e1").get("keywords/$seen").as_bool(), true);
    }

    #[test]
    fn ordner() {
        let r = parse(r#"{"methodResponses":[["Mailbox/get",{"accountId":"u1","list":[
            {"id":"a","name":"Archive","role":"archive","totalEmails":5,"unreadEmails":0},
            {"id":"b","name":"Inbox","role":"inbox","totalEmails":120,"unreadEmails":7},
            {"id":"c","name":"Kunden","role":null,"totalEmails":3,"unreadEmails":1},
            {"id":"d","name":"Sent","role":"sent","totalEmails":9,"unreadEmails":0}
        ]},"m"]]}"#);
        let f = parse_folders(&r).unwrap();
        assert_eq!(f.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["b", "d", "a", "c"]);
        assert_eq!(f[0].role, Role::Inbox);
        assert_eq!(f[0].unread, 7);
        assert_eq!(f[3].role, Role::Other);
    }

    #[test]
    fn kopfdaten() {
        let r = parse(r#"{"methodResponses":[
          ["Email/query",{"accountId":"u1","ids":["e2","e1"],"total":87},"q"],
          ["Email/get",{"accountId":"u1","list":[
            {"id":"e1","receivedAt":"2026-10-17T15:20:00Z","from":[{"name":"Marco Brunner","email":"m@x.ch"}],"subject":"Termin","keywords":{"$seen":true},"hasAttachment":false,"preview":"Wäre Samstag …"},
            {"id":"e2","receivedAt":"2026-10-18T06:42:13Z","from":[{"name":null,"email":"b@x.ch"}],"subject":"Anmeldung","keywords":{"$flagged":true},"hasAttachment":true,"preview":"Guten Morgen"}
          ]},"g"]]}"#);
        let (rows, total) = parse_headers(&r).unwrap();
        assert_eq!(total, 87);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "e2"); // neueste zuerst
        assert!(rows[0].unread && rows[0].flagged && rows[0].has_att);
        assert_eq!(rows[0].from_display(), "b@x.ch");
        assert!(!rows[1].unread);
        assert_eq!(rows[1].from_name, "Marco Brunner");
        assert_eq!(rows[1].ts, timeutil::parse_iso8601("2026-10-17T15:20:00Z").unwrap());
    }

    #[test]
    fn nachricht_text_und_html() {
        let r = parse(r#"{"methodResponses":[["Email/get",{"accountId":"u1","list":[{
            "id":"e1","subject":"Hallo","from":[{"name":"Bea","email":"b@x.ch"}],"to":[{"name":null,"email":"oli@y.ch"}],
            "sentAt":"2026-10-18T08:42:13+02:00","messageId":["abc@x"],"inReplyTo":null,"references":["r1@x","r2@x"],
            "textBody":[{"partId":"1","type":"text/plain"}],"htmlBody":[{"partId":"2","type":"text/html"}],
            "bodyValues":{"1":{"value":"Grüezi\r\nmit Text"},"2":{"value":"<p>HTML</p>"}},
            "attachments":[{"blobId":"B9","name":"Plan.pdf","type":"application/pdf","size":1234},{"blobId":"B10","name":null,"type":"image/png","size":5}]
          }]},"g"]]}"#);
        let m = parse_body(&r).unwrap();
        assert_eq!(m.text, "Grüezi\nmit Text");
        assert_eq!(m.message_id, "<abc@x>");
        assert_eq!(m.references, "<r1@x> <r2@x>");
        assert_eq!(m.in_reply_to, "");
        assert_eq!(m.ts, timeutil::parse_iso8601("2026-10-18T06:42:13Z").unwrap());
        assert_eq!(m.atts.len(), 2);
        assert_eq!(m.atts[0].blob, "B9");
        assert_eq!(m.atts[1].name, "anhang2");

        // nur HTML
        let r2 = parse(r#"{"methodResponses":[["Email/get",{"list":[{"id":"e2","textBody":[{"partId":"1","type":"text/html"}],"htmlBody":[{"partId":"1","type":"text/html"}],"bodyValues":{"1":{"value":"<p>Hallo&nbsp;Welt</p>"}}}]},"g"]]}"#);
        assert_eq!(parse_body(&r2).unwrap().text, "Hallo Welt");
        // nicht gefunden
        let r3 = parse(r#"{"methodResponses":[["Email/get",{"list":[],"notFound":["x"]},"g"]]}"#);
        assert!(parse_body(&r3).is_err());
    }

    #[test]
    fn fehler_und_identitaet() {
        let e = parse(r#"{"methodResponses":[["error",{"type":"invalidArguments","description":"kaputt"},"g"]]}"#);
        let m = method_args(&e, "g").unwrap_err();
        assert!(m.contains("invalidArguments") && m.contains("kaputt"), "{m}");
        let top = parse(r#"{"type":"urn:ietf:params:jmap:error:notRequest","detail":"Kein JSON"}"#);
        assert!(method_args(&top, "g").unwrap_err().contains("Kein JSON"));
        let i = parse(r#"{"methodResponses":[["Identity/get",{"list":[{"id":"i1","email":"andere@x.ch"},{"id":"i2","email":"Oli@Y.ch"}]},"i"]]}"#);
        assert_eq!(parse_identity(&i, "oli@y.ch").unwrap(), "i2");
        assert_eq!(parse_identity(&i, "gibt-es-nicht@y.ch").unwrap(), "i1");
        let ok = parse(r#"{"methodResponses":[["Email/set",{"updated":{"e1":null}},"s"]]}"#);
        assert!(check_set(&ok, "s", "Ändern").is_ok());
        let bad = parse(r#"{"methodResponses":[["Email/set",{"notUpdated":{"e1":{"type":"notFound","description":"weg"}}},"s"]]}"#);
        assert!(check_set(&bad, "s", "Ändern").unwrap_err().contains("notFound"));
    }

    #[test]
    fn senden() {
        let o = mime::Outgoing {
            from: Address::new("Oli", "oli@y.ch"),
            to: vec![Address::new("Bea", "b@x.ch")],
            bcc: vec![Address::new("", "geheim@z.ch")],
            subject: "Hallo ä".into(),
            body: "Zeile 1\r\nZeile 2".into(),
            in_reply_to: "<abc@x>".into(),
            references: "<r1@x> <abc@x>".into(),
            ..Default::default()
        };
        let calls = calls_send("u1", "i1", &o, "drafts1", Some("sent1"));
        let j = parse(&request(true, calls));
        assert_eq!(j.get("using").at(2).as_str(), Some(CAP_SUBMISSION));
        let e = j.get("methodCalls").at(0).at(1).get("create").get("draft");
        assert_eq!(e.get("mailboxIds").get("drafts1").as_bool(), true);
        assert_eq!(e.get("subject").as_str(), Some("Hallo ä"));
        assert_eq!(e.get("bcc").at(0).get("email").as_str(), Some("geheim@z.ch"));
        assert_eq!(e.get("inReplyTo").at(0).as_str(), Some("abc@x"));
        assert_eq!(e.get("references").at(1).as_str(), Some("abc@x"));
        assert_eq!(e.get("bodyValues").get("1").get("value").as_str(), Some("Zeile 1\nZeile 2"));
        let sub = j.get("methodCalls").at(1).at(1);
        assert_eq!(sub.get("create").get("sub").get("emailId").as_str(), Some("#draft"));
        let up = sub.get("onSuccessUpdateEmail").get("#sub");
        assert!(up.get("keywords/$draft").is_null() && up.entries().iter().any(|(k, _)| k == "keywords/$draft"));
        assert_eq!(up.get("mailboxIds/sent1").as_bool(), true);
        assert!(up.entries().iter().any(|(k, v)| k == "mailboxIds/drafts1" && v.is_null()));
    }
}
