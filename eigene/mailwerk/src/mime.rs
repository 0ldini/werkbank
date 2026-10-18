//! Mails lesen und schreiben (RFC 5322 / MIME). Nur Standardbibliothek.
//!
//! * `parse` zerlegt eine rohe Mail in Absender, Betreff, Text und Anhänge.
//! * `build` setzt eine ausgehende Mail zusammen (Text, Umlaute, Anhänge).

use crate::{b64, textutil, timeutil};

// ================================================================ Adressen

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Address {
    pub name: String,
    pub email: String,
}

impl Address {
    pub fn new(name: &str, email: &str) -> Self {
        Address { name: name.trim().to_string(), email: email.trim().to_string() }
    }

    /// Anzeigename, sonst die Adresse.
    pub fn display(&self) -> String {
        if self.name.is_empty() {
            self.email.clone()
        } else {
            self.name.clone()
        }
    }

    /// «Name <adresse>» für Kopfzeilen (Umlaute kodiert, Sonderzeichen in Anführungszeichen).
    pub fn header_form(&self) -> String {
        if self.name.is_empty() {
            return self.email.clone();
        }
        let ascii = self.name.chars().all(|c| (' '..='~').contains(&c));
        let name = if !ascii {
            textutil::encode_header_text(&self.name)
        } else if self.name.chars().any(|c| "()<>[]:;@\\,.\"".contains(c)) {
            format!("\"{}\"", self.name.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            self.name.clone()
        };
        format!("{} <{}>", name, self.email)
    }

    /// Zum Anzeigen im Programm: «Name <adresse>» oder nur die Adresse.
    pub fn full(&self) -> String {
        if self.name.is_empty() {
            self.email.clone()
        } else {
            format!("{} <{}>", self.name, self.email)
        }
    }

    /// Liest eine Adressliste («A <a@x>, "B, C" <b@x>, c@x»).
    pub fn parse_list(s: &str) -> Vec<Address> {
        let mut out = Vec::new();
        for piece in split_top_level(s, &[',', ';']) {
            let p = piece.trim();
            if p.is_empty() {
                continue;
            }
            // Gruppen-Anfang «Gruppe: a@x» → Namen vor dem Doppelpunkt verwerfen
            let p = match (p.find(':'), p.find('<'), p.find('@')) {
                (Some(c), lt, at) if lt.map_or(true, |l| c < l) && at.map_or(true, |a| c < a) && !p.starts_with('"') => {
                    p[c + 1..].trim()
                }
                _ => p,
            };
            if p.is_empty() {
                continue;
            }
            if let (Some(lt), Some(gt)) = (p.find('<'), p.rfind('>')) {
                if gt > lt {
                    let name_raw = p[..lt].trim();
                    let name = textutil::decode_encoded_words(&unquote(name_raw));
                    let email = p[lt + 1..gt].trim().to_string();
                    out.push(Address { name: name.trim().to_string(), email });
                    continue;
                }
            }
            // nur Adresse, eventuell mit Kommentar «a@x (Name)»
            let (addr, comment) = match (p.find('('), p.rfind(')')) {
                (Some(a), Some(b)) if b > a => (p[..a].trim(), p[a + 1..b].trim()),
                _ => (p, ""),
            };
            if addr.contains('@') {
                out.push(Address { name: textutil::decode_encoded_words(comment), email: addr.to_string() });
            }
        }
        out
    }
}

/// Teilt an `seps`, aber nicht innerhalb von "…", <…> oder (…).
fn split_top_level(s: &str, seps: &[char]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut in_q, mut angle, mut paren) = (false, 0i32, 0i32);
    let mut esc = false;
    for c in s.chars() {
        if esc {
            cur.push(c);
            esc = false;
            continue;
        }
        match c {
            '\\' if in_q => {
                cur.push(c);
                esc = true;
            }
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
            '<' if !in_q => {
                angle += 1;
                cur.push(c);
            }
            '>' if !in_q => {
                angle -= 1;
                cur.push(c);
            }
            '(' if !in_q => {
                paren += 1;
                cur.push(c);
            }
            ')' if !in_q => {
                paren -= 1;
                cur.push(c);
            }
            c if seps.contains(&c) && !in_q && angle <= 0 && paren <= 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        let inner = &t[1..t.len() - 1];
        let mut out = String::new();
        let mut esc = false;
        for c in inner.chars() {
            if esc {
                out.push(c);
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else {
                out.push(c);
            }
        }
        out
    } else {
        t.to_string()
    }
}

// ================================================================ Kopfzeilen

#[derive(Clone, Debug, Default)]
pub struct Headers(pub Vec<(String, String)>);

impl Headers {
    pub fn get(&self, name: &str) -> Option<&str> {
        let n = name.to_ascii_lowercase();
        self.0.iter().find(|(k, _)| *k == n).map(|(_, v)| v.as_str())
    }
}

/// Teilt rohe Mail in (Kopf, Rumpf). Fehlt die Leerzeile, ist alles Kopf.
pub fn split_head_body(raw: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\n' {
            // Leerzeile direkt danach?
            if raw.get(i + 1) == Some(&b'\n') {
                return (&raw[..i + 1], &raw[i + 2..]);
            }
            if raw.get(i + 1) == Some(&b'\r') && raw.get(i + 2) == Some(&b'\n') {
                return (&raw[..i + 1], &raw[i + 3..]);
            }
        }
        i += 1;
    }
    (raw, &[])
}

pub fn parse_headers(block: &[u8]) -> Headers {
    let text = String::from_utf8_lossy(block);
    let mut list: Vec<(String, String)> = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = list.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
            continue;
        }
        if let Some(c) = line.find(':') {
            let name = line[..c].trim().to_ascii_lowercase();
            if !name.is_empty() && !name.contains(' ') {
                list.push((name, line[c + 1..].trim().to_string()));
            }
        }
    }
    Headers(list)
}

// ================================================================ Content-Type & Co.

#[derive(Clone, Debug, Default)]
struct CType {
    main: String,
    sub: String,
    params: Vec<(String, String)>,
}

impl CType {
    fn param(&self, k: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

fn parse_ctype(value: &str) -> CType {
    let parts = split_top_level_semicolons(value);
    let mut it = parts.into_iter();
    let first = it.next().unwrap_or_default();
    let first = first.trim().to_ascii_lowercase();
    let (main, sub) = match first.split_once('/') {
        Some((m, s)) => (m.trim().to_string(), s.trim().to_string()),
        None => ("text".to_string(), "plain".to_string()),
    };
    CType { main, sub, params: parse_params(it) }
}

fn split_top_level_semicolons(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut esc = false;
    for c in s.chars() {
        if esc {
            cur.push(c);
            esc = false;
            continue;
        }
        match c {
            '\\' if in_q => {
                cur.push(c);
                esc = true;
            }
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
            ';' if !in_q => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn parse_params(items: impl Iterator<Item = String>) -> Vec<(String, String)> {
    let mut raw: Vec<(String, String)> = Vec::new();
    for item in items {
        if let Some((k, v)) = item.split_once('=') {
            let k = k.trim().to_ascii_lowercase();
            if !k.is_empty() {
                raw.push((k, unquote(v.trim())));
            }
        }
    }
    merge_rfc2231(raw)
}

fn decode_ext_value(v: &str) -> String {
    // charset'sprache'%-kodierter Text
    let mut it = v.splitn(3, '\'');
    match (it.next(), it.next(), it.next()) {
        (Some(cs), Some(_), Some(val)) => textutil::to_string_charset(&textutil::percent_decode(val), cs),
        _ => v.to_string(),
    }
}

fn merge_rfc2231(raw: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut plain: Vec<(String, String)> = Vec::new();
    let mut ext: Vec<(String, String)> = Vec::new();
    let mut multi: Vec<(String, usize, bool, String)> = Vec::new();
    for (k, v) in raw {
        match k.find('*') {
            None => plain.push((k, v)),
            Some(pos) => {
                let base = k[..pos].to_string();
                let suffix = &k[pos + 1..];
                if suffix.is_empty() {
                    ext.push((base, decode_ext_value(&v)));
                } else {
                    let is_ext = suffix.ends_with('*');
                    let idx = suffix.trim_end_matches('*').parse::<usize>().unwrap_or(0);
                    multi.push((base, idx, is_ext, v));
                }
            }
        }
    }
    let mut bases: Vec<String> = Vec::new();
    for (b, _, _, _) in &multi {
        if !bases.contains(b) {
            bases.push(b.clone());
        }
    }
    for base in bases {
        let mut pieces: Vec<&(String, usize, bool, String)> = multi.iter().filter(|m| m.0 == base).collect();
        pieces.sort_by_key(|m| m.1);
        let mut charset = String::from("utf-8");
        let mut bytes: Vec<u8> = Vec::new();
        for (n, p) in pieces.iter().enumerate() {
            let mut val = p.3.as_str();
            if p.2 {
                if n == 0 {
                    let mut it = val.splitn(3, '\'');
                    if let (Some(cs), Some(_), Some(rest)) = (it.next(), it.next(), it.next()) {
                        charset = cs.to_string();
                        val = rest;
                    }
                }
                bytes.extend(textutil::percent_decode(val));
            } else {
                bytes.extend_from_slice(val.as_bytes());
            }
        }
        ext.push((base, textutil::to_string_charset(&bytes, &charset)));
    }
    // Die erweiterte Form (RFC 2231) geht der einfachen Rückfallform vor.
    let mut out: Vec<(String, String)> = plain.into_iter().filter(|(k, _)| !ext.iter().any(|(e, _)| e == k)).collect();
    out.extend(ext);
    out
}

fn decode_transfer(body: &[u8], encoding: &str) -> Vec<u8> {
    match encoding.trim().to_ascii_lowercase().as_str() {
        "base64" => b64::decode_bytes(body),
        "quoted-printable" => textutil::decode_qp(body, false),
        _ => body.to_vec(),
    }
}

fn bytes_to_text(bytes: &[u8], charset: Option<&str>) -> String {
    match charset {
        Some(cs) if !cs.eq_ignore_ascii_case("utf-8") && !cs.eq_ignore_ascii_case("us-ascii") => {
            textutil::to_string_charset(bytes, cs)
        }
        _ => match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => textutil::to_string_charset(bytes, "windows-1252"),
        },
    }
}

// ================================================================ Ergebnis

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub subject: String,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub date_ts: Option<i64>,
    pub message_id: String,
    pub in_reply_to: String,
    pub references: String,
    pub text: String,
    pub had_html: bool,
    pub attachments: Vec<Attachment>,
    /// Der Kopf nennt multipart/mixed o. ä. – es gibt wahrscheinlich Anhänge.
    pub looks_like_attachments: bool,
}

/// Liest die wichtigsten Kopfzeilen (auch aus einem reinen Kopf-Abruf).
pub fn parse_head(h: &Headers) -> Parsed {
    let dec = |s: &str| textutil::decode_encoded_words(s);
    let mut p = Parsed::default();
    p.subject = dec(h.get("subject").unwrap_or("")).trim().to_string();
    p.from = Address::parse_list(h.get("from").unwrap_or(""));
    p.to = Address::parse_list(h.get("to").unwrap_or(""));
    p.cc = Address::parse_list(h.get("cc").unwrap_or(""));
    p.reply_to = Address::parse_list(h.get("reply-to").unwrap_or(""));
    p.date_ts = h.get("date").and_then(timeutil::parse_rfc2822);
    p.message_id = h.get("message-id").unwrap_or("").trim().to_string();
    p.in_reply_to = h.get("in-reply-to").unwrap_or("").trim().to_string();
    p.references = h.get("references").unwrap_or("").trim().to_string();
    if let Some(ct) = h.get("content-type") {
        let c = parse_ctype(ct);
        p.looks_like_attachments =
            c.main == "multipart" && matches!(c.sub.as_str(), "mixed" | "signed" | "report" | "encrypted");
    }
    p
}

struct Walk {
    plain: Vec<String>,
    html: Vec<String>,
    atts: Vec<Attachment>,
}

impl Walk {
    fn new() -> Self {
        Walk { plain: Vec::new(), html: Vec::new(), atts: Vec::new() }
    }
    fn merge(&mut self, other: Walk) {
        self.plain.extend(other.plain);
        self.html.extend(other.html);
        self.atts.extend(other.atts);
    }
}

/// Zerlegt eine komplette Mail.
pub fn parse(raw: &[u8]) -> Parsed {
    let (head, body) = split_head_body(raw);
    let headers = parse_headers(head);
    let mut p = parse_head(&headers);
    let mut w = Walk::new();
    walk(&headers, body, &mut w, 0);
    p.had_html = !w.html.is_empty();
    let text = if !w.plain.is_empty() {
        w.plain.join("\n\n")
    } else if !w.html.is_empty() {
        textutil::html_to_text(&w.html.join("\n"))
    } else {
        String::new()
    };
    p.text = text.replace("\r\n", "\n").replace('\r', "\n").trim_end().to_string();
    p.attachments = w.atts;
    p
}

fn walk(h: &Headers, body: &[u8], w: &mut Walk, depth: usize) {
    if depth > 12 {
        return;
    }
    let ct = h.get("content-type").map(parse_ctype).unwrap_or_else(|| CType {
        main: "text".into(),
        sub: "plain".into(),
        params: Vec::new(),
    });

    if ct.main == "multipart" {
        let boundary = match ct.param("boundary") {
            Some(b) if !b.is_empty() => b.to_string(),
            _ => return,
        };
        let parts = split_multipart(body, &boundary);
        if ct.sub == "alternative" {
            // Die beste Darstellung wählen: Text vor HTML.
            let mut best_plain: Option<Walk> = None;
            let mut best_html: Option<Walk> = None;
            for part in parts {
                let (ph, pb) = split_head_body(part);
                let hh = parse_headers(ph);
                let mut sub = Walk::new();
                walk(&hh, pb, &mut sub, depth + 1);
                if !sub.plain.is_empty() && best_plain.is_none() {
                    best_plain = Some(sub);
                } else if !sub.html.is_empty() && best_html.is_none() {
                    best_html = Some(sub);
                }
            }
            if let Some(mut b) = best_plain {
                // Auch hier das HTML merken, damit «had_html» stimmt.
                if let Some(hm) = best_html {
                    b.html.extend(hm.html);
                    b.atts.extend(hm.atts);
                }
                w.merge(b);
            } else if let Some(b) = best_html {
                w.merge(b);
            }
        } else {
            for part in parts {
                let (ph, pb) = split_head_body(part);
                let hh = parse_headers(ph);
                walk(&hh, pb, w, depth + 1);
            }
        }
        return;
    }

    let enc = h.get("content-transfer-encoding").unwrap_or("7bit");
    let disp_raw = h.get("content-disposition").unwrap_or("");
    let disp_parts = split_top_level_semicolons(disp_raw);
    let mut dit = disp_parts.into_iter();
    let disp = dit.next().unwrap_or_default().trim().to_ascii_lowercase();
    let dparams = parse_params(dit);
    let filename_raw = dparams
        .iter()
        .find(|(k, _)| k == "filename")
        .map(|(_, v)| v.clone())
        .or_else(|| ct.param("name").map(|s| s.to_string()));
    let filename = filename_raw.map(|f| textutil::decode_encoded_words(&f));

    let is_text = ct.main == "text" && (ct.sub == "plain" || ct.sub == "html");
    if is_text && disp != "attachment" && filename.is_none() {
        let bytes = decode_transfer(body, enc);
        let text = bytes_to_text(&bytes, ct.param("charset"));
        if ct.sub == "html" {
            w.html.push(text);
        } else {
            w.plain.push(text);
        }
        return;
    }

    // Signaturen blenden wir aus.
    let full = format!("{}/{}", ct.main, ct.sub);
    if matches!(full.as_str(), "application/pgp-signature" | "application/pkcs7-signature" | "application/x-pkcs7-signature") {
        return;
    }
    let data = decode_transfer(body, enc);
    if full == "message/rfc822" {
        let inner = parse(&data);
        let name = filename.unwrap_or_else(|| {
            if inner.subject.is_empty() {
                "Nachricht.eml".to_string()
            } else {
                format!("{}.eml", textutil::safe_filename(&inner.subject))
            }
        });
        w.atts.push(Attachment { filename: name, content_type: full, data });
        return;
    }
    let name = filename.unwrap_or_else(|| default_name(&full, w.atts.len() + 1));
    w.atts.push(Attachment { filename: name, content_type: full, data });
}

fn default_name(content_type: &str, n: usize) -> String {
    let ext = match content_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "application/pdf" => "pdf",
        "text/calendar" => "ics",
        "text/plain" => "txt",
        "text/html" => "html",
        _ => "bin",
    };
    format!("anhang{n}.{ext}")
}

/// Teilt einen Multipart-Rumpf an den Begrenzern.
fn split_multipart<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let delim = format!("--{boundary}").into_bytes();
    let mut starts: Vec<(usize, usize, bool)> = Vec::new(); // (Beginn der Begrenzerzeile, Ende der Zeile, letzter)
    let mut i = 0;
    while i < body.len() {
        let at_line_start = i == 0 || body[i - 1] == b'\n';
        if at_line_start && body[i..].starts_with(&delim) {
            let after = i + delim.len();
            let last = body[after..].starts_with(b"--");
            let eol = body[after..].iter().position(|&b| b == b'\n').map(|p| after + p + 1).unwrap_or(body.len());
            starts.push((i, eol, last));
            if last {
                break;
            }
            i = eol;
            continue;
        }
        // zur nächsten Zeile
        match body[i..].iter().position(|&b| b == b'\n') {
            Some(p) => i += p + 1,
            None => break,
        }
    }
    let mut parts = Vec::new();
    for w in 0..starts.len() {
        let (_, eol, last) = starts[w];
        if last {
            break;
        }
        let end_raw = if w + 1 < starts.len() { starts[w + 1].0 } else { body.len() };
        let mut end = end_raw;
        // der Zeilenumbruch vor dem nächsten Begrenzer gehört nicht zum Teil
        if end > eol && body[end - 1] == b'\n' {
            end -= 1;
            if end > eol && body[end - 1] == b'\r' {
                end -= 1;
            }
        }
        if eol <= end {
            parts.push(&body[eol..end]);
        }
    }
    parts
}

/// Kurzvorschau: erste ~`max` Zeichen ohne Zeilenumbrüche und Zitate.
pub fn make_preview(text: &str, max: usize) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('>') {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(l);
        if out.chars().count() >= max {
            break;
        }
    }
    let mut cut: String = out.chars().take(max).collect();
    if out.chars().count() > max {
        cut.push('…');
    }
    cut
}

// ================================================================ Antworten und Weiterleiten

fn strip_reply_prefixes(subject: &str) -> &str {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let mut cut = 0;
        for p in ["re:", "aw:", "fwd:", "fw:", "wg:", "antw:"] {
            if lower.starts_with(p) {
                cut = p.len();
                break;
            }
        }
        if cut == 0 {
            return s;
        }
        s = s[cut..].trim_start();
    }
}

pub fn reply_subject(subject: &str) -> String {
    format!("Re: {}", strip_reply_prefixes(subject))
}

pub fn forward_subject(subject: &str) -> String {
    format!("Fwd: {}", strip_reply_prefixes(subject))
}

/// Text mit «> » vor jeder Zeile.
pub fn quote_text(text: &str) -> String {
    text.lines()
        .map(|l| if l.starts_with('>') { format!(">{l}") } else { format!("> {l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Empfänger für «Antworten» bzw. «Allen antworten». Rückgabe: (An, Cc).
pub fn reply_recipients(p: &Parsed, my_email: &str, all: bool) -> (Vec<Address>, Vec<Address>) {
    let me = my_email.to_ascii_lowercase();
    let primary: Vec<Address> = if !p.reply_to.is_empty() { p.reply_to.clone() } else { p.from.clone() };
    let mut to: Vec<Address> = Vec::new();
    for a in primary {
        if a.email.to_ascii_lowercase() != me && !to.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email)) {
            to.push(a);
        }
    }
    let mut cc: Vec<Address> = Vec::new();
    if all {
        for a in p.to.iter().chain(p.cc.iter()) {
            let e = a.email.to_ascii_lowercase();
            if e != me && !to.iter().chain(cc.iter()).any(|x| x.email.eq_ignore_ascii_case(&e)) {
                cc.push(a.clone());
            }
        }
    }
    (to, cc)
}

// ================================================================ Mail zusammenbauen

#[derive(Clone, Debug, Default)]
pub struct Outgoing {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<Attachment>,
    pub in_reply_to: String,
    pub references: String,
}

impl Outgoing {
    /// Alle Empfänger-Adressen für den SMTP-Umschlag (An, Cc und Bcc).
    pub fn envelope_recipients(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for a in self.to.iter().chain(self.cc.iter()).chain(self.bcc.iter()) {
            if !a.email.is_empty() && !v.iter().any(|x| x.eq_ignore_ascii_case(&a.email)) {
                v.push(a.email.clone());
            }
        }
        v
    }
}

fn addr_header(name: &str, list: &[Address]) -> String {
    if list.is_empty() {
        return String::new();
    }
    let items: Vec<String> = list.iter().map(|a| a.header_form()).collect();
    format!("{}: {}\r\n", name, items.join(",\r\n "))
}

fn mix(mut x: u64) -> u64 {
    // SplitMix64: aus Zeit und Zähler eine gut gestreute Zahl machen
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Baut die Mail. `seed` soll sich bei jedem Aufruf ändern (z. B. Zeit in
/// Nanosekunden). Rückgabe: (rohe Mail, Message-ID).
pub fn build(o: &Outgoing, now_ts: i64, seed: u64) -> (Vec<u8>, String) {
    let domain = o.from.email.split('@').nth(1).filter(|d| !d.is_empty()).unwrap_or("mailwerk.local");
    let id_num = mix(seed ^ (now_ts as u64));
    let message_id = format!("<{:016x}.{:08x}@{}>", id_num, (mix(id_num) >> 32) as u32, domain);

    let mut head = String::new();
    head.push_str(&format!("Date: {}\r\n", timeutil::format_rfc2822(now_ts)));
    head.push_str(&format!("From: {}\r\n", o.from.header_form()));
    head.push_str(&addr_header("To", &o.to));
    head.push_str(&addr_header("Cc", &o.cc));
    head.push_str(&format!("Subject: {}\r\n", textutil::encode_header_text(o.subject.trim())));
    head.push_str(&format!("Message-ID: {message_id}\r\n"));
    if !o.in_reply_to.is_empty() {
        head.push_str(&format!("In-Reply-To: {}\r\n", o.in_reply_to));
    }
    if !o.references.is_empty() {
        head.push_str(&format!("References: {}\r\n", o.references));
    }
    head.push_str("MIME-Version: 1.0\r\n");
    head.push_str("X-Mailer: Mailwerk\r\n");

    // Text: reines ASCII mit kurzen Zeilen bleibt 7bit, sonst Quoted-Printable.
    let body = o.body.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n");
    let ascii = body.is_ascii() && body.split("\r\n").all(|l| l.len() <= 900);
    let (cte, text_part) = if ascii { ("7bit", body.clone()) } else { ("quoted-printable", textutil::encode_qp(body.as_bytes())) };
    let mut text_block = String::new();
    text_block.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    text_block.push_str(&format!("Content-Transfer-Encoding: {cte}\r\n\r\n"));
    text_block.push_str(&text_part);
    if !text_block.ends_with("\r\n") {
        text_block.push_str("\r\n");
    }

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(head.as_bytes());
    if o.attachments.is_empty() {
        out.extend_from_slice(text_block.as_bytes());
        return (out, message_id);
    }

    let boundary = format!("=_mw_{:016x}{:08x}", id_num, (mix(id_num ^ 0x55) >> 32) as u32);
    out.extend_from_slice(format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n").as_bytes());
    out.extend_from_slice(b"Dies ist eine mehrteilige Nachricht im MIME-Format.\r\n");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(text_block.as_bytes());
    for a in &o.attachments {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let ct = if a.content_type.is_empty() { "application/octet-stream" } else { a.content_type.as_str() };
        let ascii_name: String = a
            .filename
            .chars()
            .map(|c| if (' '..='~').contains(&c) && c != '"' && c != '\\' { c } else { '_' })
            .collect();
        let mut part = String::new();
        part.push_str(&format!("Content-Type: {ct}; name=\"{ascii_name}\"\r\n"));
        part.push_str("Content-Transfer-Encoding: base64\r\n");
        if a.filename.is_ascii() {
            part.push_str(&format!("Content-Disposition: attachment; filename=\"{ascii_name}\"\r\n\r\n"));
        } else {
            part.push_str(&format!(
                "Content-Disposition: attachment; filename=\"{}\"; filename*=UTF-8''{}\r\n\r\n",
                ascii_name,
                textutil::percent_encode(&a.filename)
            ));
        }
        out.extend_from_slice(part.as_bytes());
        out.extend_from_slice(b64::encode_wrapped(&a.data, 76).as_bytes());
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (out, message_id)
}

// ================================================================ Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn crlf(s: &str) -> Vec<u8> {
        s.replace('\n', "\r\n").into_bytes()
    }

    #[test]
    fn adressen() {
        let l = Address::parse_list(
            "\"Keller, Beatrice\" <b@x.ch>, oli@y.ch, =?UTF-8?Q?M=C3=BCller?= <m@x.ch>, Hans Meier <h@z.ch>, a@b.ch (Anna Blau)",
        );
        assert_eq!(l.len(), 5);
        assert_eq!(l[0], Address::new("Keller, Beatrice", "b@x.ch"));
        assert_eq!(l[1], Address::new("", "oli@y.ch"));
        assert_eq!(l[2], Address::new("Müller", "m@x.ch"));
        assert_eq!(l[3].display(), "Hans Meier");
        assert_eq!(l[4], Address::new("Anna Blau", "a@b.ch"));
        assert_eq!(Address::new("Keller, B.", "b@x.ch").header_form(), "\"Keller, B.\" <b@x.ch>");
        assert!(Address::new("Müller", "m@x.ch").header_form().starts_with("=?UTF-8?B?"));
    }

    #[test]
    fn einfache_mail() {
        let raw = crlf("From: =?UTF-8?Q?Beatrice_K=C3=A4ller?= <b.keller@example.org>\nTo: oli@example.org\nSubject: =?UTF-8?B?QW5tZWxkdW5nIFdlbHBlbmt1cnM=?=\nDate: Wed, 14 Oct 2026 08:42:13 +0200\nMessage-ID: <abc@x>\nContent-Type: text/plain; charset=iso-8859-1\nContent-Transfer-Encoding: quoted-printable\n\nGr=FCezi mit Umlaut\n");
        let p = parse(&raw);
        assert_eq!(p.subject, "Anmeldung Welpenkurs");
        assert_eq!(p.from[0].name, "Beatrice Käller");
        assert_eq!(p.from[0].email, "b.keller@example.org");
        assert_eq!(p.to[0].email, "oli@example.org");
        assert_eq!(p.text, "Grüezi mit Umlaut");
        assert!(p.date_ts.is_some());
        assert_eq!(p.message_id, "<abc@x>");
        assert!(p.attachments.is_empty());
    }

    #[test]
    fn alternative_bevorzugt_text() {
        let raw = crlf("Subject: x\nContent-Type: multipart/alternative; boundary=\"BB\"\n\n--BB\nContent-Type: text/plain; charset=utf-8\n\nNur Text\n--BB\nContent-Type: text/html; charset=utf-8\n\n<p>Nur <b>HTML</b></p>\n--BB--\n");
        let p = parse(&raw);
        assert_eq!(p.text, "Nur Text");
        assert!(p.had_html);
    }

    #[test]
    fn nur_html() {
        let raw = crlf("Subject: x\nContent-Type: text/html; charset=utf-8\n\n<html><body><p>Hallo&nbsp;Welt</p><p>Zweiter</p></body></html>\n");
        let p = parse(&raw);
        assert_eq!(p.text, "Hallo Welt\n\nZweiter");
    }

    #[test]
    fn anhang_base64_und_rfc2231() {
        let data: Vec<u8> = (0..200u8).collect();
        let b64d = b64::encode_wrapped(&data, 76);
        let raw = crlf(&format!(
            "Subject: Anhang\nContent-Type: multipart/mixed; boundary=\"M1\"\n\n--M1\nContent-Type: text/plain; charset=utf-8\n\nSiehe Anhang.\n--M1\nContent-Type: application/pdf; name=\"x.pdf\"\nContent-Disposition: attachment; filename*0*=UTF-8''R%C3%B6ntgen; filename*1*=%20bild.pdf\nContent-Transfer-Encoding: base64\n\n{}\n--M1--\n",
            b64d.replace("\r\n", "\n")
        ));
        let p = parse(&raw);
        assert_eq!(p.text, "Siehe Anhang.");
        assert_eq!(p.attachments.len(), 1);
        assert_eq!(p.attachments[0].filename, "Röntgen bild.pdf");
        assert_eq!(p.attachments[0].content_type, "application/pdf");
        assert_eq!(p.attachments[0].data, data);
        assert!(p.looks_like_attachments);
    }

    #[test]
    fn verschachtelt_related_alternative() {
        let raw = crlf("Subject: v\nContent-Type: multipart/mixed; boundary=\"A\"\n\n--A\nContent-Type: multipart/alternative; boundary=\"B\"\n\n--B\nContent-Type: text/plain\n\nPlain-Fassung\n--B\nContent-Type: multipart/related; boundary=\"C\"\n\n--C\nContent-Type: text/html\n\n<p>HTML</p>\n--C\nContent-Type: image/png\nContent-ID: <i1>\nContent-Transfer-Encoding: base64\n\niVBORw0=\n--C--\n--B--\n--A\nContent-Type: application/zip; name=\"a.zip\"\nContent-Transfer-Encoding: base64\n\nUEs=\n--A--\n");
        let p = parse(&raw);
        assert_eq!(p.text, "Plain-Fassung");
        // Das Inline-Bild der HTML-Fassung zählt als Anhang, die ZIP-Datei ebenfalls.
        let names: Vec<&str> = p.attachments.iter().map(|a| a.filename.as_str()).collect();
        assert!(names.contains(&"a.zip"), "{names:?}");
    }

    #[test]
    fn kopf_ohne_rumpf() {
        let raw = crlf("From: a@b.ch\nSubject: Nur Kopf\nContent-Type: multipart/mixed; boundary=x\n\n");
        let p = parse(&raw);
        assert_eq!(p.subject, "Nur Kopf");
        assert!(p.looks_like_attachments);
        assert_eq!(p.text, "");
    }

    #[test]
    fn bauen_und_wieder_lesen() {
        let o = Outgoing {
            from: Address::new("Oli Müller", "oli@adilios.example"),
            to: vec![Address::new("Beatrice Keller", "b@x.ch"), Address::new("", "c@y.ch")],
            cc: vec![Address::new("Sandra, Vogel", "s@z.ch")],
            bcc: vec![Address::new("", "geheim@z.ch")],
            subject: "Re: Anmeldung Welpenkurs – Grüsse".into(),
            body: format!("Grüezi Frau Keller\n\nLange Zeile: {}\nMit = Zeichen.\n", "wort ".repeat(60)),
            attachments: vec![
                Attachment { filename: "Kursprogramm Herbst.pdf".into(), content_type: "application/pdf".into(), data: (0..=255u8).cycle().take(1000).collect() },
                Attachment { filename: "Röntgen.png".into(), content_type: "image/png".into(), data: vec![1, 2, 3] },
            ],
            in_reply_to: "<orig@x>".into(),
            references: "<orig@x>".into(),
        };
        let (raw, id) = build(&o, 1_800_000_000, 42);
        assert!(id.starts_with('<') && id.ends_with("@adilios.example>"));
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.contains("geheim@z.ch"), "Bcc darf nicht im Kopf stehen");
        assert!(text.lines().all(|l| l.len() <= 998));
        let p = parse(&raw);
        assert_eq!(p.subject, o.subject);
        assert_eq!(p.from[0], o.from);
        assert_eq!(p.to, o.to);
        assert_eq!(p.cc[0].name, "Sandra, Vogel");
        assert_eq!(p.message_id, id);
        assert_eq!(p.in_reply_to, "<orig@x>");
        assert_eq!(p.text, o.body.trim_end());
        assert_eq!(p.attachments.len(), 2);
        assert_eq!(p.attachments[0].filename, "Kursprogramm Herbst.pdf");
        assert_eq!(p.attachments[0].data, o.attachments[0].data);
        assert_eq!(p.attachments[1].filename, "Röntgen.png");
        assert_eq!(p.attachments[1].data, vec![1, 2, 3]);
        assert_eq!(o.envelope_recipients(), vec!["b@x.ch", "c@y.ch", "s@z.ch", "geheim@z.ch"]);
    }

    #[test]
    fn bauen_ascii_ohne_anhang() {
        let o = Outgoing {
            from: Address::new("", "a@b.ch"),
            to: vec![Address::new("", "c@d.ch")],
            subject: "Hallo".into(),
            body: "Kurz.".into(),
            ..Default::default()
        };
        let (raw, _) = build(&o, 1_800_000_000, 1);
        let t = String::from_utf8(raw.clone()).unwrap();
        assert!(t.contains("Content-Transfer-Encoding: 7bit\r\n\r\nKurz.\r\n"));
        assert_eq!(parse(&raw).text, "Kurz.");
    }

    #[test]
    fn antworten() {
        assert_eq!(reply_subject("Re: AW: Hallo"), "Re: Hallo");
        assert_eq!(forward_subject("Hallo"), "Fwd: Hallo");
        assert_eq!(quote_text("a\n> b"), "> a\n>> b");
        let mut p = Parsed::default();
        p.from = vec![Address::new("Bea", "bea@x.ch")];
        p.to = vec![Address::new("", "oli@y.ch"), Address::new("", "dritter@z.ch")];
        p.cc = vec![Address::new("", "BEA@x.ch")];
        let (to, cc) = reply_recipients(&p, "OLI@y.ch", true);
        assert_eq!(to.len(), 1);
        assert_eq!(cc.len(), 1);
        assert_eq!(cc[0].email, "dritter@z.ch");
        let (to2, cc2) = reply_recipients(&p, "oli@y.ch", false);
        assert_eq!((to2.len(), cc2.len()), (1, 0));
    }

    #[test]
    fn vorschau() {
        assert_eq!(make_preview("> zitat\n\nErste Zeile\nZweite Zeile", 100), "Erste Zeile Zweite Zeile");
        assert_eq!(make_preview("abcdefghij", 5), "abcde…");
    }
}
