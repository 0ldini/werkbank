//! IMAP-Client (RFC 3501) über einen beliebigen Datenstrom. Nur Standardbibliothek.
//!
//! Die Verbindung (TLS) stellt der Aufrufer her und übergibt den Strom; so lässt
//! sich das Protokoll hier mit einem Schein-Server prüfen.

use std::io::{Read, Write};

use crate::b64;
use crate::role::{guess_role, Role};

// ================================================================ Antwort-Teile

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Nil,
    Atom(String),
    Str(Vec<u8>),
    List(Vec<Tok>),
}

impl Tok {
    pub fn as_str_lossy(&self) -> Option<String> {
        match self {
            Tok::Atom(a) => Some(a.clone()),
            Tok::Str(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Tok::Atom(a) => a.parse().ok(),
            _ => None,
        }
    }
}

pub struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Parser { b, i: 0 }
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] == b' ' || self.b[self.i] == b'\r' || self.b[self.i] == b'\n' || self.b[self.i] == b'\t') {
            self.i += 1;
        }
    }

    pub fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.i >= self.b.len()
    }

    pub fn parse(&mut self) -> Option<Tok> {
        self.skip_ws();
        let c = *self.b.get(self.i)?;
        match c {
            b'(' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.b.get(self.i) {
                        None => return Some(Tok::List(items)),
                        Some(b')') => {
                            self.i += 1;
                            return Some(Tok::List(items));
                        }
                        _ => items.push(self.parse()?),
                    }
                }
            }
            b'"' => {
                self.i += 1;
                let mut out = Vec::new();
                while self.i < self.b.len() {
                    let ch = self.b[self.i];
                    self.i += 1;
                    match ch {
                        b'\\' => {
                            if let Some(&n) = self.b.get(self.i) {
                                out.push(n);
                                self.i += 1;
                            }
                        }
                        b'"' => return Some(Tok::Str(out)),
                        _ => out.push(ch),
                    }
                }
                Some(Tok::Str(out))
            }
            b'{' => {
                let close = self.b[self.i..].iter().position(|&x| x == b'}')? + self.i;
                let num: String = self.b[self.i + 1..close].iter().map(|&x| x as char).filter(|c| c.is_ascii_digit()).collect();
                let n: usize = num.parse().ok()?;
                let mut start = close + 1;
                if self.b.get(start) == Some(&b'\r') {
                    start += 1;
                }
                if self.b.get(start) == Some(&b'\n') {
                    start += 1;
                }
                let end = (start + n).min(self.b.len());
                self.i = end;
                Some(Tok::Str(self.b[start..end].to_vec()))
            }
            _ => {
                let start = self.i;
                let mut depth = 0;
                while self.i < self.b.len() {
                    let ch = self.b[self.i];
                    if ch == b'[' {
                        depth += 1;
                    } else if ch == b']' && depth > 0 {
                        depth -= 1;
                    } else if depth == 0 && (ch == b' ' || ch == b'\r' || ch == b'\n' || ch == b'(' || ch == b')') {
                        break;
                    }
                    self.i += 1;
                }
                let atom = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
                if atom.eq_ignore_ascii_case("NIL") {
                    Some(Tok::Nil)
                } else {
                    Some(Tok::Atom(atom))
                }
            }
        }
    }
}

// ================================================================ Ordnernamen (modifiziertes UTF-7)

pub fn utf7_decode(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let tail = &rest[p + 1..];
        match tail.find('-') {
            Some(e) => {
                let chunk = &tail[..e];
                if chunk.is_empty() {
                    out.push('&');
                } else {
                    let bytes = b64::decode(&chunk.replace(',', "/"));
                    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                rest = &tail[e + 1..];
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn utf7_encode(s: &str) -> String {
    let mut out = String::new();
    let mut run: Vec<u16> = Vec::new();
    let flush = |run: &mut Vec<u16>, out: &mut String| {
        if run.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(run.len() * 2);
        for u in run.iter() {
            bytes.extend_from_slice(&u.to_be_bytes());
        }
        let enc = b64::encode(&bytes).trim_end_matches('=').replace('/', ",");
        out.push('&');
        out.push_str(&enc);
        out.push('-');
        run.clear();
    };
    for ch in s.chars() {
        if ch == '&' {
            flush(&mut run, &mut out);
            out.push_str("&-");
        } else if (' '..='~').contains(&ch) {
            flush(&mut run, &mut out);
            out.push(ch);
        } else {
            let mut buf = [0u16; 2];
            run.extend_from_slice(ch.encode_utf16(&mut buf));
        }
    }
    flush(&mut run, &mut out);
    out
}

// ================================================================ Datentypen

#[derive(Debug, Clone, PartialEq)]
pub struct FolderEntry {
    /// Name, wie der Server ihn liefert (für Befehle).
    pub raw: String,
    /// Lesbarer Name (UTF-8).
    pub name: String,
    pub delim: Option<char>,
    pub flags: Vec<String>,
    pub role: Role,
    pub selectable: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SelectInfo {
    pub exists: u32,
    pub uid_validity: u32,
    pub uid_next: u32,
}

#[derive(Debug, Clone, Default)]
pub struct FetchItem {
    pub seq: u32,
    pub uid: u32,
    pub flags: Vec<String>,
    pub size: u32,
    pub internal_date: String,
    pub header: Vec<u8>,
    pub body: Vec<u8>,
}

impl FetchItem {
    pub fn has_flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x.eq_ignore_ascii_case(f))
    }
}

pub enum Arg<'a> {
    Raw(&'a str),
    /// Zeichenkette: wird, wenn nötig, als Literal gesendet.
    Str(&'a str),
    /// Immer als Literal (z. B. Mail beim APPEND).
    Bytes(&'a [u8]),
}

pub struct Reply {
    pub untagged: Vec<Vec<u8>>,
    pub ok: bool,
    pub status: String,
    pub text: String,
}

// ================================================================ Verbindung

pub struct Imap<S: Read + Write> {
    s: S,
    buf: Vec<u8>,
    pos: usize,
    tag: u32,
    pub caps: Vec<String>,
}

fn needs_literal(s: &str) -> bool {
    s.bytes().any(|b| b >= 0x80 || b < 0x20 || b == 0x7f)
}

fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            o.push('\\');
        }
        o.push(c);
    }
    o.push('"');
    o
}

fn literal_len(line: &[u8]) -> Option<usize> {
    let mut t = line;
    while t.last() == Some(&b'\n') || t.last() == Some(&b'\r') {
        t = &t[..t.len() - 1];
    }
    if t.last() != Some(&b'}') {
        return None;
    }
    let open = t.iter().rposition(|&b| b == b'{')?;
    let inner = &t[open + 1..t.len() - 1];
    let digits: Vec<u8> = inner.iter().copied().filter(|b| *b != b'+').collect();
    if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    String::from_utf8_lossy(&digits).parse().ok()
}

impl<S: Read + Write> Imap<S> {
    pub fn new(s: S) -> Self {
        Imap { s, buf: Vec::new(), pos: 0, tag: 0, caps: Vec::new() }
    }

    fn fill(&mut self) -> Result<(), String> {
        if self.pos > 0 && self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        }
        let mut tmp = [0u8; 16 * 1024];
        match self.s.read(&mut tmp) {
            Ok(0) => Err("Der Server hat die Verbindung beendet.".to_string()),
            Ok(n) => {
                self.buf.extend_from_slice(&tmp[..n]);
                Ok(())
            }
            Err(e) => Err(format!("Lesen fehlgeschlagen: {e}")),
        }
    }

    fn read_line(&mut self) -> Result<Vec<u8>, String> {
        loop {
            if let Some(p) = self.buf[self.pos..].iter().position(|&b| b == b'\n') {
                let end = self.pos + p + 1;
                let line = self.buf[self.pos..end].to_vec();
                self.pos = end;
                return Ok(line);
            }
            self.fill()?;
        }
    }

    fn read_exact_n(&mut self, n: usize) -> Result<Vec<u8>, String> {
        while self.buf.len() - self.pos < n {
            self.fill()?;
        }
        let v = self.buf[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(v)
    }

    /// Eine vollständige Antwort: Zeile samt eingebetteter Literale.
    fn read_response(&mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        loop {
            let line = self.read_line()?;
            out.extend_from_slice(&line);
            if let Some(n) = literal_len(&line) {
                let lit = self.read_exact_n(n)?;
                out.extend_from_slice(&lit);
                continue;
            }
            return Ok(out);
        }
    }

    fn write_all(&mut self, data: &[u8]) -> Result<(), String> {
        self.s.write_all(data).and_then(|_| self.s.flush()).map_err(|e| format!("Senden fehlgeschlagen: {e}"))
    }

    /// Liest die Begrüssung. Gibt deren Text zurück.
    pub fn read_greeting(&mut self) -> Result<String, String> {
        let r = self.read_response()?;
        let t = String::from_utf8_lossy(&r).trim().to_string();
        if t.starts_with("* OK") || t.starts_with("* PREAUTH") {
            Ok(t)
        } else {
            Err(format!("Unerwartete Begrüssung: {t}"))
        }
    }

    fn next_tag(&mut self) -> String {
        self.tag += 1;
        format!("A{:04}", self.tag)
    }

    /// Befehl senden und bis zur Abschlussantwort lesen.
    pub fn exec(&mut self, args: &[Arg]) -> Result<Reply, String> {
        let tag = self.next_tag();
        let mut pending: Vec<u8> = tag.clone().into_bytes();
        for a in args {
            pending.push(b' ');
            match a {
                Arg::Raw(s) => pending.extend_from_slice(s.as_bytes()),
                Arg::Str(s) if !needs_literal(s) => pending.extend_from_slice(quote(s).as_bytes()),
                Arg::Str(s) => {
                    self.send_literal_header(&mut pending, s.len(), &tag)?;
                    pending.extend_from_slice(s.as_bytes());
                }
                Arg::Bytes(b) => {
                    self.send_literal_header(&mut pending, b.len(), &tag)?;
                    pending.extend_from_slice(b);
                }
            }
        }
        pending.extend_from_slice(b"\r\n");
        self.write_all(&pending)?;

        let mut untagged = Vec::new();
        let prefix = format!("{tag} ");
        loop {
            let r = self.read_response()?;
            if r.starts_with(prefix.as_bytes()) {
                let rest = String::from_utf8_lossy(&r[prefix.len()..]).trim().to_string();
                let (status, text) = match rest.split_once(' ') {
                    Some((s, t)) => (s.to_ascii_uppercase(), t.to_string()),
                    None => (rest.to_ascii_uppercase(), String::new()),
                };
                return Ok(Reply { untagged, ok: status == "OK", status, text });
            }
            if r.starts_with(b"+") {
                continue; // unerwartete Fortsetzung ignorieren
            }
            untagged.push(r);
        }
    }

    fn send_literal_header(&mut self, pending: &mut Vec<u8>, len: usize, tag: &str) -> Result<(), String> {
        pending.extend_from_slice(format!("{{{len}}}\r\n").as_bytes());
        let data = std::mem::take(pending);
        self.write_all(&data)?;
        // Auf «+» warten
        loop {
            let r = self.read_response()?;
            if r.starts_with(b"+") {
                return Ok(());
            }
            if r.starts_with(tag.as_bytes()) {
                return Err(format!("Server lehnt ab: {}", String::from_utf8_lossy(&r).trim()));
            }
        }
    }

    fn run(&mut self, args: &[Arg], what: &str) -> Result<Reply, String> {
        let r = self.exec(args)?;
        if r.ok {
            Ok(r)
        } else {
            Err(format!("{what}: {} {}", r.status, r.text))
        }
    }

    // ------------------------------------------------------------ Befehle

    pub fn login(&mut self, user: &str, pass: &str) -> Result<(), String> {
        self.run(&[Arg::Raw("LOGIN"), Arg::Str(user), Arg::Str(pass)], "Anmeldung fehlgeschlagen")?;
        self.refresh_capabilities()?;
        Ok(())
    }

    pub fn refresh_capabilities(&mut self) -> Result<(), String> {
        let r = self.run(&[Arg::Raw("CAPABILITY")], "CAPABILITY")?;
        self.caps.clear();
        for u in &r.untagged {
            let t = String::from_utf8_lossy(u).to_ascii_uppercase();
            if let Some(rest) = t.trim().strip_prefix("* CAPABILITY ") {
                self.caps = rest.split_whitespace().map(|s| s.to_string()).collect();
            }
        }
        Ok(())
    }

    pub fn has_cap(&self, c: &str) -> bool {
        self.caps.iter().any(|x| x.eq_ignore_ascii_case(c))
    }

    pub fn list_folders(&mut self) -> Result<Vec<FolderEntry>, String> {
        let r = self.run(&[Arg::Raw("LIST"), Arg::Str(""), Arg::Str("*")], "Ordnerliste")?;
        let mut out = Vec::new();
        for u in &r.untagged {
            let mut p = Parser::new(u);
            let star = p.parse();
            let kw = p.parse();
            if star != Some(Tok::Atom("*".into())) || kw.as_ref().and_then(|t| t.as_str_lossy()).map(|s| s.to_ascii_uppercase()) != Some("LIST".into()) {
                continue;
            }
            let flags_tok = p.parse();
            let delim_tok = p.parse();
            let name_tok = p.parse();
            let (Some(Tok::List(fl)), Some(dt), Some(nt)) = (flags_tok, delim_tok, name_tok) else { continue };
            let flags: Vec<String> = fl.iter().filter_map(|t| t.as_str_lossy()).collect();
            let delim = dt.as_str_lossy().and_then(|s| s.chars().next());
            let raw = nt.as_str_lossy().unwrap_or_default();
            let name = utf7_decode(&raw);
            let selectable = !flags.iter().any(|f| f.eq_ignore_ascii_case("\\Noselect") || f.eq_ignore_ascii_case("\\NonExistent"));
            let role = guess_role(&name, &flags);
            out.push(FolderEntry { raw, name, delim, flags, role, selectable });
        }
        Ok(out)
    }

    pub fn select(&mut self, raw_name: &str, read_only: bool) -> Result<SelectInfo, String> {
        let cmd = if read_only { "EXAMINE" } else { "SELECT" };
        let r = self.run(&[Arg::Raw(cmd), Arg::Str(raw_name)], "Ordner öffnen")?;
        let mut info = SelectInfo::default();
        for u in &r.untagged {
            let t = String::from_utf8_lossy(u);
            let t = t.trim();
            let up = t.to_ascii_uppercase();
            if let Some(rest) = up.strip_prefix("* ") {
                let mut it = rest.split_whitespace();
                if let (Some(n), Some("EXISTS")) = (it.next(), it.next()) {
                    info.exists = n.parse().unwrap_or(0);
                }
            }
            if let Some(i) = up.find("[UIDVALIDITY ") {
                info.uid_validity = up[i + 13..].split(']').next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
            }
            if let Some(i) = up.find("[UIDNEXT ") {
                info.uid_next = up[i + 9..].split(']').next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
            }
        }
        Ok(info)
    }

    /// (Nachrichten, Ungelesene) eines Ordners, ohne ihn zu öffnen.
    pub fn status(&mut self, raw_name: &str) -> Result<(u32, u32), String> {
        let r = self.run(&[Arg::Raw("STATUS"), Arg::Str(raw_name), Arg::Raw("(MESSAGES UNSEEN)")], "Ordnerstand")?;
        let (mut total, mut unseen) = (0, 0);
        for u in &r.untagged {
            let mut p = Parser::new(u);
            let _ = p.parse(); // *
            let kw = p.parse();
            if kw.and_then(|t| t.as_str_lossy()).map(|s| s.eq_ignore_ascii_case("STATUS")) != Some(true) {
                continue;
            }
            let _ = p.parse(); // Name
            if let Some(Tok::List(items)) = p.parse() {
                let mut i = 0;
                while i + 1 < items.len() {
                    let k = items[i].as_str_lossy().unwrap_or_default().to_ascii_uppercase();
                    let v = items[i + 1].as_u64().unwrap_or(0) as u32;
                    if k == "MESSAGES" {
                        total = v;
                    } else if k == "UNSEEN" {
                        unseen = v;
                    }
                    i += 2;
                }
            }
        }
        Ok((total, unseen))
    }

    /// Kopfdaten der Nachrichten mit den Folgenummern `from..=to` (1-basiert).
    pub fn fetch_headers_range(&mut self, from: u32, to: u32) -> Result<Vec<FetchItem>, String> {
        let set = format!("{from}:{to}");
        let items = "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID CONTENT-TYPE)])";
        let r = self.run(&[Arg::Raw("FETCH"), Arg::Raw(&set), Arg::Raw(items)], "Nachrichten laden")?;
        Ok(parse_fetch_items(&r.untagged))
    }

    /// Ganze Nachricht (ohne sie als gelesen zu markieren).
    pub fn fetch_message(&mut self, uid: u32) -> Result<FetchItem, String> {
        let set = uid.to_string();
        let r = self.run(&[Arg::Raw("UID FETCH"), Arg::Raw(&set), Arg::Raw("(UID FLAGS BODY.PEEK[])")], "Nachricht laden")?;
        parse_fetch_items(&r.untagged)
            .into_iter()
            .find(|f| f.uid == uid || f.uid == 0)
            .ok_or_else(|| "Nachricht nicht gefunden (vielleicht gelöscht).".to_string())
    }

    pub fn set_flag(&mut self, uid: u32, flag: &str, on: bool) -> Result<(), String> {
        let set = uid.to_string();
        let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
        let fl = format!("({flag})");
        self.run(&[Arg::Raw("UID STORE"), Arg::Raw(&set), Arg::Raw(op), Arg::Raw(&fl)], "Markierung ändern")?;
        Ok(())
    }

    /// Verschiebt eine Nachricht (MOVE, sonst Kopieren + Löschen).
    pub fn move_to(&mut self, uid: u32, target_raw: &str) -> Result<(), String> {
        let set = uid.to_string();
        if self.has_cap("MOVE") {
            self.run(&[Arg::Raw("UID MOVE"), Arg::Raw(&set), Arg::Str(target_raw)], "Verschieben")?;
            return Ok(());
        }
        self.run(&[Arg::Raw("UID COPY"), Arg::Raw(&set), Arg::Str(target_raw)], "Kopieren")?;
        self.delete_here(uid)
    }

    /// Löscht in diesem Ordner endgültig (Markierung + Aufräumen).
    pub fn delete_here(&mut self, uid: u32) -> Result<(), String> {
        self.set_flag(uid, "\\Deleted", true)?;
        let set = uid.to_string();
        if self.has_cap("UIDPLUS") {
            self.run(&[Arg::Raw("UID EXPUNGE"), Arg::Raw(&set)], "Aufräumen")?;
        } else {
            self.run(&[Arg::Raw("EXPUNGE")], "Aufräumen")?;
        }
        Ok(())
    }

    /// Legt eine Nachricht in einem Ordner ab (z. B. «Gesendet»).
    pub fn append(&mut self, raw_name: &str, flags: &str, message: &[u8]) -> Result<(), String> {
        let fl = format!("({flags})");
        self.run(&[Arg::Raw("APPEND"), Arg::Str(raw_name), Arg::Raw(&fl), Arg::Bytes(message)], "Ablegen")?;
        Ok(())
    }

    pub fn logout(&mut self) {
        let _ = self.exec(&[Arg::Raw("LOGOUT")]);
    }
}

// ================================================================ FETCH lesen

pub fn parse_fetch_items(untagged: &[Vec<u8>]) -> Vec<FetchItem> {
    let mut out = Vec::new();
    for u in untagged {
        let mut p = Parser::new(u);
        if p.parse() != Some(Tok::Atom("*".into())) {
            continue;
        }
        let seq = match p.parse().and_then(|t| t.as_u64()) {
            Some(n) => n as u32,
            None => continue,
        };
        match p.parse() {
            Some(Tok::Atom(a)) if a.eq_ignore_ascii_case("FETCH") => {}
            _ => continue,
        }
        let list = match p.parse() {
            Some(Tok::List(l)) => l,
            _ => continue,
        };
        let mut item = FetchItem { seq, ..Default::default() };
        let mut i = 0;
        while i + 1 < list.len() {
            let key = list[i].as_str_lossy().unwrap_or_default().to_ascii_uppercase();
            let val = &list[i + 1];
            if key == "UID" {
                item.uid = val.as_u64().unwrap_or(0) as u32;
            } else if key == "FLAGS" {
                if let Tok::List(fl) = val {
                    item.flags = fl.iter().filter_map(|t| t.as_str_lossy()).collect();
                }
            } else if key == "RFC822.SIZE" {
                item.size = val.as_u64().unwrap_or(0) as u32;
            } else if key == "INTERNALDATE" {
                item.internal_date = val.as_str_lossy().unwrap_or_default();
            } else if key.starts_with("BODY[HEADER") || key == "RFC822.HEADER" {
                if let Tok::Str(b) = val {
                    item.header = b.clone();
                }
            } else if key.starts_with("BODY[]") || key == "RFC822" {
                if let Tok::Str(b) = val {
                    item.body = b.clone();
                }
            }
            i += 2;
        }
        out.push(item);
    }
    out
}

// ================================================================ Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Mock {
        out: VecDeque<u8>,
        cmd: Vec<u8>,
        lit: usize,
        script: VecDeque<(String, String)>,
        seen: Vec<String>,
    }

    impl Mock {
        fn new(greeting: &str, script: Vec<(&str, String)>) -> Self {
            Mock {
                out: greeting.bytes().collect(),
                cmd: Vec::new(),
                lit: 0,
                script: script.into_iter().map(|(a, b)| (a.to_string(), b)).collect(),
                seen: Vec::new(),
            }
        }
    }

    impl Read for Mock {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut n = 0;
            while n < buf.len() {
                match self.out.pop_front() {
                    Some(b) => {
                        buf[n] = b;
                        n += 1;
                    }
                    None => break,
                }
            }
            Ok(n)
        }
    }

    impl Write for Mock {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            for &b in data {
                self.cmd.push(b);
                if self.lit > 0 {
                    self.lit -= 1;
                    continue;
                }
                if b == b'\n' {
                    if let Some(n) = literal_len(&self.cmd) {
                        self.out.extend(b"+ bereit\r\n".iter());
                        self.lit = n;
                        continue;
                    }
                    let full = String::from_utf8_lossy(&self.cmd).into_owned();
                    self.cmd.clear();
                    let tag = full.split(' ').next().unwrap().to_string();
                    let (expect, reply) = self.script.pop_front().expect("unerwarteter Befehl");
                    assert!(full.contains(&expect), "Befehl {full:?} enthält nicht {expect:?}");
                    self.seen.push(full);
                    self.out.extend(reply.replace("{TAG}", &tag).bytes());
                }
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn utf7() {
        assert_eq!(utf7_decode("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(utf7_encode("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(utf7_decode("A &- B"), "A & B");
        assert_eq!(utf7_encode("A & B"), "A &- B");
        let s = "日本語/Büro €";
        assert_eq!(utf7_decode(&utf7_encode(s)), s);
        assert_eq!(utf7_decode("INBOX"), "INBOX");
    }

    #[test]
    fn parser_literale_und_listen() {
        let raw = b"* 3 FETCH (UID 77 FLAGS (\\Seen \\Flagged) BODY[HEADER.FIELDS (FROM SUBJECT)] {13}\r\nSubject: Hi\r\n NIL)\r\n";
        let items = parse_fetch_items(&[raw.to_vec()]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].seq, 3);
        assert_eq!(items[0].uid, 77);
        assert!(items[0].has_flag("\\seen") && items[0].has_flag("\\Flagged"));
        assert_eq!(items[0].header, b"Subject: Hi\r\n");
        let mut p = Parser::new(b"(\"a b\" NIL {2}\r\nxy atom[1 2] (x))");
        assert_eq!(
            p.parse(),
            Some(Tok::List(vec![
                Tok::Str(b"a b".to_vec()),
                Tok::Nil,
                Tok::Str(b"xy".to_vec()),
                Tok::Atom("atom[1 2]".into()),
                Tok::List(vec![Tok::Atom("x".into())])
            ]))
        );
    }

    #[test]
    fn sitzung_von_login_bis_nachricht() {
        let hdr = "From: Bea <bea@x.ch>\r\nSubject: =?UTF-8?Q?Gr=C3=BCezi?=\r\n\r\n";
        let body = "From: Bea <bea@x.ch>\r\nSubject: Hi\r\n\r\nHallo\r\n";
        let script = vec![
            ("LOGIN \"oli\" \"geheim\"", "{TAG} OK angemeldet\r\n".to_string()),
            ("CAPABILITY", "* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n{TAG} OK ok\r\n".to_string()),
            (
                "LIST \"\" \"*\"",
                "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n* LIST (\\HasNoChildren \\Sent) \"/\" \"Gesendet\"\r\n* LIST (\\Noselect \\HasChildren) \"/\" \"[Gmail]\"\r\n* LIST () \"/\" \"Entw&APw-rfe\"\r\n{TAG} OK fertig\r\n".to_string(),
            ),
            (
                "SELECT \"INBOX\"",
                "* 12 EXISTS\r\n* OK [UIDVALIDITY 3857529045] ok\r\n* OK [UIDNEXT 4392] ok\r\n{TAG} OK [READ-WRITE] ok\r\n".to_string(),
            ),
            (
                "FETCH 11:12 (UID FLAGS",
                format!(
                    "* 11 FETCH (UID 4390 FLAGS (\\Seen) RFC822.SIZE 900 INTERNALDATE \"14-Oct-2026 08:42:13 +0200\" BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{})\r\n* 12 FETCH (UID 4391 FLAGS () RFC822.SIZE 100 INTERNALDATE \"14-Oct-2026 09:00:00 +0200\" BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{})\r\n{{TAG}} OK fertig\r\n",
                    hdr.len(), hdr, hdr.len(), hdr
                ),
            ),
            (
                "UID FETCH 4391 (UID FLAGS BODY.PEEK[])",
                format!("* 12 FETCH (UID 4391 FLAGS () BODY[] {{{}}}\r\n{})\r\n{{TAG}} OK fertig\r\n", body.len(), body),
            ),
            ("UID STORE 4391 +FLAGS.SILENT (\\Seen)", "{TAG} OK ok\r\n".to_string()),
            ("UID MOVE 4391 \"Gesendet\"", "{TAG} OK verschoben\r\n".to_string()),
            ("STATUS \"INBOX\" (MESSAGES UNSEEN)", "* STATUS \"INBOX\" (MESSAGES 12 UNSEEN 3)\r\n{TAG} OK ok\r\n".to_string()),
            ("LOGOUT", "* BYE tschüss\r\n{TAG} OK ok\r\n".to_string()),
        ];
        let mock = Mock::new("* OK IMAP bereit\r\n", script);
        let mut c = Imap::new(mock);
        assert!(c.read_greeting().unwrap().contains("bereit"));
        c.login("oli", "geheim").unwrap();
        assert!(c.has_cap("move"));
        let folders = c.list_folders().unwrap();
        assert_eq!(folders.len(), 4);
        assert_eq!(folders[0].role, Role::Inbox);
        assert_eq!(folders[1].role, Role::Sent);
        assert!(!folders[2].selectable);
        assert_eq!(folders[3].name, "Entwürfe");
        assert_eq!(folders[3].raw, "Entw&APw-rfe");
        let info = c.select("INBOX", false).unwrap();
        assert_eq!(info, SelectInfo { exists: 12, uid_validity: 3857529045, uid_next: 4392 });
        let h = c.fetch_headers_range(11, 12).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].uid, 4390);
        assert!(h[0].has_flag("\\Seen") && !h[1].has_flag("\\Seen"));
        assert_eq!(h[0].header, hdr.as_bytes());
        assert_eq!(h[1].internal_date, "14-Oct-2026 09:00:00 +0200");
        let m = c.fetch_message(4391).unwrap();
        assert_eq!(m.body, body.as_bytes());
        c.set_flag(4391, "\\Seen", true).unwrap();
        c.move_to(4391, "Gesendet").unwrap();
        assert_eq!(c.status("INBOX").unwrap(), (12, 3));
        c.logout();
    }

    #[test]
    fn login_mit_umlaut_passwort_nutzt_literal() {
        let script = vec![("LOGIN \"oli\" {10}", "{TAG} OK ok\r\n".to_string()), ("CAPABILITY", "* CAPABILITY IMAP4rev1\r\n{TAG} OK\r\n".to_string())];
        let mut c = Imap::new(Mock::new("* OK hi\r\n", script));
        c.read_greeting().unwrap();
        c.login("oli", "pässwörd").unwrap(); // 8 Zeichen = 10 Bytes
    }

    #[test]
    fn fehler_wird_gemeldet() {
        let script = vec![("LOGIN", "{TAG} NO [AUTHENTICATIONFAILED] Falsches Passwort\r\n".to_string())];
        let mut c = Imap::new(Mock::new("* OK hi\r\n", script));
        c.read_greeting().unwrap();
        let e = c.login("a", "b").unwrap_err();
        assert!(e.contains("Falsches Passwort"), "{e}");
    }

    #[test]
    fn move_ohne_move_faehigkeit() {
        let script = vec![
            ("UID COPY 5 \"Archiv\"", "{TAG} OK\r\n".to_string()),
            ("UID STORE 5 +FLAGS.SILENT (\\Deleted)", "{TAG} OK\r\n".to_string()),
            ("EXPUNGE", "* 3 EXPUNGE\r\n{TAG} OK\r\n".to_string()),
        ];
        let mut c = Imap::new(Mock::new("* OK hi\r\n", script));
        c.read_greeting().unwrap();
        c.move_to(5, "Archiv").unwrap();
    }

    #[test]
    fn append_sendet_literal() {
        let msg = b"Subject: x\r\n\r\ny\r\n";
        let script = vec![("APPEND \"Gesendet\" (\\Seen) {17}", "{TAG} OK abgelegt\r\n".to_string())];
        let mut c = Imap::new(Mock::new("* OK hi\r\n", script));
        c.read_greeting().unwrap();
        c.append("Gesendet", "\\Seen", msg).unwrap();
    }
}
