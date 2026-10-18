//! SMTP-Client (RFC 5321) über einen beliebigen Datenstrom. Nur Standardbibliothek.
//!
//! Ablauf: Begrüssung → EHLO → (STARTTLS → neue Verbindung nach dem TLS-Aufbau → EHLO)
//! → AUTH → MAIL/RCPT/DATA → QUIT. Die TLS-Verbindung stellt der Aufrufer her.

use std::io::{Read, Write};

use crate::b64;

pub struct SmtpReply {
    pub code: u16,
    pub lines: Vec<String>,
}

impl SmtpReply {
    pub fn text(&self) -> String {
        self.lines.join(" ")
    }
}

pub struct Smtp<S: Read + Write> {
    s: S,
    buf: Vec<u8>,
    pos: usize,
    /// Fähigkeiten aus der letzten EHLO-Antwort (Grossbuchstaben).
    pub caps: Vec<String>,
}

fn check_addr(a: &str) -> Result<(), String> {
    if a.is_empty() || a.chars().any(|c| c.is_control() || c == '<' || c == '>' || c == ' ') {
        return Err(format!("Ungültige Adresse: «{}»", a.escape_debug()));
    }
    Ok(())
}

/// Bereitet die Nachricht für DATA vor: CRLF-Zeilenenden, Punkte am Zeilenanfang
/// verdoppeln, Abschluss «CRLF . CRLF».
pub fn dot_stuff(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut start = 0;
    let n = data.len();
    while start < n {
        let end = data[start..].iter().position(|&b| b == b'\n').map(|p| start + p).unwrap_or(n);
        let mut line = &data[start..end];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if line.first() == Some(&b'.') {
            out.push(b'.');
        }
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
        start = end + 1;
    }
    out.extend_from_slice(b".\r\n");
    out
}

impl<S: Read + Write> Smtp<S> {
    pub fn new(s: S) -> Self {
        Smtp { s, buf: Vec::new(), pos: 0, caps: Vec::new() }
    }

    /// Gibt den Strom zurück (für den Wechsel auf TLS nach STARTTLS).
    pub fn into_inner(self) -> Result<S, String> {
        if self.pos < self.buf.len() {
            return Err("Unerwartete Daten vor dem TLS-Aufbau.".to_string());
        }
        Ok(self.s)
    }

    fn fill(&mut self) -> Result<(), String> {
        if self.pos > 0 && self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        }
        let mut tmp = [0u8; 4096];
        match self.s.read(&mut tmp) {
            Ok(0) => Err("Der Server hat die Verbindung beendet.".to_string()),
            Ok(n) => {
                self.buf.extend_from_slice(&tmp[..n]);
                Ok(())
            }
            Err(e) => Err(format!("Lesen fehlgeschlagen: {e}")),
        }
    }

    fn read_line(&mut self) -> Result<String, String> {
        loop {
            if let Some(p) = self.buf[self.pos..].iter().position(|&b| b == b'\n') {
                let end = self.pos + p + 1;
                let line = String::from_utf8_lossy(&self.buf[self.pos..end]).trim_end().to_string();
                self.pos = end;
                return Ok(line);
            }
            self.fill()?;
        }
    }

    /// Liest eine (auch mehrzeilige) Antwort.
    pub fn read_reply(&mut self) -> Result<SmtpReply, String> {
        let mut lines = Vec::new();
        loop {
            let l = self.read_line()?;
            if l.len() < 3 {
                return Err(format!("Ungültige Antwort: «{l}»"));
            }
            let code: u16 = l[..3].parse().map_err(|_| format!("Ungültige Antwort: «{l}»"))?;
            let more = l.as_bytes().get(3) == Some(&b'-');
            lines.push(l.get(4..).unwrap_or("").to_string());
            if !more {
                return Ok(SmtpReply { code, lines });
            }
        }
    }

    fn write_all(&mut self, data: &[u8]) -> Result<(), String> {
        self.s.write_all(data).and_then(|_| self.s.flush()).map_err(|e| format!("Senden fehlgeschlagen: {e}"))
    }

    fn cmd(&mut self, line: &str) -> Result<SmtpReply, String> {
        self.write_all(format!("{line}\r\n").as_bytes())?;
        self.read_reply()
    }

    fn expect(r: SmtpReply, ok: &[u16], what: &str) -> Result<SmtpReply, String> {
        if ok.contains(&r.code) {
            Ok(r)
        } else {
            Err(format!("{what}: {} {}", r.code, r.text()))
        }
    }

    pub fn read_greeting(&mut self) -> Result<(), String> {
        let r = self.read_reply()?;
        Self::expect(r, &[220], "Begrüssung").map(|_| ())
    }

    pub fn ehlo(&mut self, name: &str) -> Result<(), String> {
        let r = self.cmd(&format!("EHLO {name}"))?;
        let r = Self::expect(r, &[250], "EHLO")?;
        self.caps = r.lines.iter().skip(1).map(|l| l.to_ascii_uppercase()).collect();
        Ok(())
    }

    pub fn has_cap(&self, name: &str) -> bool {
        self.caps.iter().any(|c| c == name || c.starts_with(&format!("{name} ")))
    }

    pub fn auth_mechs(&self) -> Vec<String> {
        self.caps
            .iter()
            .filter_map(|c| c.strip_prefix("AUTH ").or_else(|| c.strip_prefix("AUTH=")))
            .flat_map(|s| s.split_whitespace().map(|x| x.to_string()).collect::<Vec<_>>())
            .collect()
    }

    /// Verlangt TLS. Danach muss der Aufrufer `into_inner` aufrufen, TLS aufbauen und
    /// mit einem neuen `Smtp` weitermachen (inkl. neuem `ehlo`).
    pub fn starttls(&mut self) -> Result<(), String> {
        let r = self.cmd("STARTTLS")?;
        Self::expect(r, &[220], "STARTTLS").map(|_| ())
    }

    pub fn auth(&mut self, user: &str, pass: &str) -> Result<(), String> {
        let mechs = self.auth_mechs();
        let plain_ok = mechs.iter().any(|m| m == "PLAIN") || mechs.is_empty();
        if plain_ok {
            let blob = b64::encode(format!("\0{user}\0{pass}").as_bytes());
            let r = self.cmd(&format!("AUTH PLAIN {blob}"))?;
            return Self::expect(r, &[235], "Anmeldung fehlgeschlagen").map(|_| ());
        }
        if mechs.iter().any(|m| m == "LOGIN") {
            let r = self.cmd("AUTH LOGIN")?;
            Self::expect(r, &[334], "Anmeldung")?;
            let r = self.cmd(&b64::encode(user.as_bytes()))?;
            Self::expect(r, &[334], "Anmeldung")?;
            let r = self.cmd(&b64::encode(pass.as_bytes()))?;
            return Self::expect(r, &[235], "Anmeldung fehlgeschlagen").map(|_| ());
        }
        Err(format!("Der Server bietet kein unterstütztes Anmeldeverfahren (nur: {}).", mechs.join(", ")))
    }

    /// Sendet eine fertige Nachricht (`message` = rohe Mail ohne Bcc-Kopfzeile).
    pub fn send(&mut self, from: &str, rcpts: &[String], message: &[u8]) -> Result<(), String> {
        check_addr(from)?;
        if rcpts.is_empty() {
            return Err("Kein Empfänger angegeben.".to_string());
        }
        let r = self.cmd(&format!("MAIL FROM:<{from}>"))?;
        Self::expect(r, &[250], "Absender abgelehnt")?;
        for rc in rcpts {
            check_addr(rc)?;
            let r = self.cmd(&format!("RCPT TO:<{rc}>"))?;
            Self::expect(r, &[250, 251], &format!("Empfänger {rc} abgelehnt"))?;
        }
        let r = self.cmd("DATA")?;
        Self::expect(r, &[354], "DATA")?;
        let body = dot_stuff(message);
        self.write_all(&body)?;
        let r = self.read_reply()?;
        Self::expect(r, &[250], "Senden abgelehnt").map(|_| ())
    }

    pub fn quit(&mut self) {
        let _ = self.cmd("QUIT");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Mock {
        out: VecDeque<u8>,
        line: Vec<u8>,
        data_mode: bool,
        script: VecDeque<(String, String)>,
        pub received_data: Vec<u8>,
        pub seen: Vec<String>,
    }

    impl Mock {
        fn new(greeting: &str, script: Vec<(&str, &str)>) -> Self {
            Mock {
                out: greeting.bytes().collect(),
                line: Vec::new(),
                data_mode: false,
                script: script.into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
                received_data: Vec::new(),
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
                self.line.push(b);
                if self.data_mode {
                    if self.line.ends_with(b"\r\n.\r\n") {
                        self.data_mode = false;
                        self.received_data = std::mem::take(&mut self.line);
                        let (_, reply) = self.script.pop_front().expect("Skript zu Ende");
                        self.out.extend(reply.bytes());
                    }
                } else if b == b'\n' {
                    let cmd = String::from_utf8_lossy(&self.line).trim_end().to_string();
                    self.line.clear();
                    let (expect, reply) = self.script.pop_front().expect("unerwarteter Befehl");
                    assert!(cmd.starts_with(&expect), "Befehl {cmd:?} beginnt nicht mit {expect:?}");
                    self.seen.push(cmd.clone());
                    if cmd == "DATA" {
                        self.data_mode = true;
                    }
                    self.out.extend(reply.bytes());
                }
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn punkte_verdoppeln() {
        assert_eq!(dot_stuff(b"a\n.b\r\n..c"), b"a\r\n..b\r\n...c\r\n.\r\n");
        assert_eq!(dot_stuff(b""), b".\r\n");
    }

    #[test]
    fn versand_mit_anmeldung() {
        let script = vec![
            ("EHLO mailwerk.local", "250-mail.example\r\n250-SIZE 35882577\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n"),
            ("AUTH PLAIN AG9saQBnZWhlaW0=", "235 2.7.0 angemeldet\r\n"),
            ("MAIL FROM:<oli@example.org>", "250 ok\r\n"),
            ("RCPT TO:<b@x.ch>", "250 ok\r\n"),
            ("RCPT TO:<c@y.ch>", "250 ok\r\n"),
            ("DATA", "354 los\r\n"),
            ("", "250 2.0.0 angenommen\r\n"),
            ("QUIT", "221 tschüss\r\n"),
        ];
        let mut c = Smtp::new(Mock::new("220 mail.example bereit\r\n", script));
        c.read_greeting().unwrap();
        c.ehlo("mailwerk.local").unwrap();
        assert!(c.has_cap("SIZE") && c.has_cap("8BITMIME"));
        assert_eq!(c.auth_mechs(), vec!["PLAIN", "LOGIN"]);
        c.auth("oli", "geheim").unwrap();
        c.send("oli@example.org", &["b@x.ch".into(), "c@y.ch".into()], b"Subject: Hi\n\n.Punkt\n").unwrap();
        c.quit();
        let inner = c.into_inner().unwrap();
        assert_eq!(inner.received_data, b"Subject: Hi\r\n\r\n..Punkt\r\n.\r\n");
    }

    #[test]
    fn login_verfahren_als_ersatz() {
        let script = vec![
            ("EHLO x", "250-h\r\n250 AUTH LOGIN\r\n"),
            ("AUTH LOGIN", "334 VXNlcm5hbWU6\r\n"),
            ("b2xp", "334 UGFzc3dvcmQ6\r\n"),
            ("Z2VoZWlt", "235 ok\r\n"),
        ];
        let mut c = Smtp::new(Mock::new("220 hi\r\n", script));
        c.read_greeting().unwrap();
        c.ehlo("x").unwrap();
        c.auth("oli", "geheim").unwrap();
    }

    #[test]
    fn abgelehnter_empfaenger_und_starttls() {
        let script = vec![
            ("EHLO x", "250-h\r\n250 STARTTLS\r\n"),
            ("STARTTLS", "220 bereit\r\n"),
            ("MAIL FROM:<a@b.ch>", "250 ok\r\n"),
            ("RCPT TO:<zz@x.ch>", "550 5.1.1 unbekannt\r\n"),
        ];
        let mut c = Smtp::new(Mock::new("220 hi\r\n", script));
        c.read_greeting().unwrap();
        c.ehlo("x").unwrap();
        assert!(c.has_cap("STARTTLS"));
        c.starttls().unwrap();
        let e = c.send("a@b.ch", &["zz@x.ch".into()], b"x").unwrap_err();
        assert!(e.contains("zz@x.ch") && e.contains("550"), "{e}");
    }

    #[test]
    fn adressen_werden_geprueft() {
        let mut c = Smtp::new(Mock::new("220 hi\r\n", vec![]));
        c.read_greeting().unwrap();
        assert!(c.send("a@b.ch\r\nRCPT TO:<x@y.ch>", &["c@d.ch".into()], b"x").is_err());
        assert!(c.send("a@b.ch", &[], b"x").is_err());
    }

    #[test]
    fn falsches_passwort() {
        let script = vec![("EHLO x", "250-h\r\n250 AUTH PLAIN\r\n"), ("AUTH PLAIN", "535 5.7.8 Anmeldung abgelehnt\r\n")];
        let mut c = Smtp::new(Mock::new("220 hi\r\n", script));
        c.read_greeting().unwrap();
        c.ehlo("x").unwrap();
        let e = c.auth("a", "b").unwrap_err();
        assert!(e.contains("535"), "{e}");
    }
}
