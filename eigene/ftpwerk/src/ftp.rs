//! Eigener FTP-Client (RFC 959, 2428, 3659, 4217): Steuerverbindung, passive Datenverbindungen,
//! MLSD/LIST, Fortsetzen mit REST/APPE und FTPS (explizit und implizit).
//! Ohne die Funktion «tls» baut sich nur einfaches FTP – so lässt sich der Kern ohne Zusatzpakete testen.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::listparse;
use crate::model::{now_unix, Entry, Proto, RErr, Res, Site};
use crate::remote::{log_line, Prog, Remote, Tick};
use crate::sprache::{t, tf};

const CHUNK: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IO_TIMEOUT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------- TLS-Anbindung (oder Platzhalter)

#[cfg(feature = "tls")]
type TlsHandle = crate::tls::TlsCtx;

#[cfg(not(feature = "tls"))]
#[derive(Clone)]
pub struct TlsHandle;

#[cfg(feature = "tls")]
fn new_tls(host: &str, trusted: &[String]) -> Res<TlsHandle> {
    crate::tls::TlsCtx::new(host, trusted)
}

#[cfg(not(feature = "tls"))]
fn new_tls(_host: &str, _trusted: &[String]) -> Res<TlsHandle> {
    Err(RErr::Msg(t("FTPS ist in dieser Ausführung nicht enthalten.").to_string()))
}

#[cfg(feature = "tls")]
fn wrap_tls(ctx: &TlsHandle, tcp: TcpStream) -> Res<Stream> {
    Ok(Stream::Tls(Box::new(ctx.wrap(tcp)?)))
}

#[cfg(not(feature = "tls"))]
fn wrap_tls(_ctx: &TlsHandle, _tcp: TcpStream) -> Res<Stream> {
    Err(RErr::Msg(t("FTPS ist in dieser Ausführung nicht enthalten.").to_string()))
}

#[cfg(feature = "tls")]
fn untrusted(ctx: &TlsHandle) -> Option<RErr> {
    ctx.untrusted()
}

#[cfg(not(feature = "tls"))]
fn untrusted(_ctx: &TlsHandle) -> Option<RErr> {
    None
}

// ---------------------------------------------------------------- Datenstrom

enum Stream {
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(Box<crate::tls::TlsStream>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(s) => s,
            #[cfg(feature = "tls")]
            Stream::Tls(s) => &s.sock,
        }
    }

    /// Sendeseite sauber beenden (bei TLS mit «close_notify»), damit der Server das Ende erkennt.
    fn finish(&mut self) {
        match self {
            Stream::Plain(s) => {
                let _ = s.flush();
                let _ = s.shutdown(Shutdown::Write);
            }
            #[cfg(feature = "tls")]
            Stream::Tls(s) => {
                s.conn.send_close_notify();
                let _ = s.flush();
                let _ = s.sock.shutdown(Shutdown::Write);
            }
        }
    }

    /// Lesen; ein TLS-Ende ohne «close_notify» (viele FTP-Server) gilt als normales Ende.
    fn read_data(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
            r => r,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => s.flush(),
        }
    }
}

// ---------------------------------------------------------------- Antworten

#[derive(Clone, Debug)]
pub struct Reply {
    pub code: u16,
    /// Alle Zeilen der Antwort ohne Zahlencode, durch Zeilenumbruch getrennt.
    pub text: String,
}

impl Reply {
    fn class(&self) -> u16 {
        self.code / 100
    }

    fn line(&self) -> String {
        format!("{} {}", self.code, self.text.lines().next().unwrap_or("").trim())
    }
}

fn connect_tcp(host: &str, port: u16) -> Res<TcpStream> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| RErr::Msg(tf("Server «{host}» nicht gefunden: {e}", &[("host", &host), ("e", &e)])))?
        .collect();
    let mut last = RErr::Msg(tf("Keine Adresse für «{host}».", &[("host", &host)]));
    for a in addrs {
        match TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(IO_TIMEOUT));
                let _ = s.set_write_timeout(Some(IO_TIMEOUT));
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = RErr::Msg(tf("Verbindung zu {host}:{port} fehlgeschlagen: {e}", &[("host", &host), ("port", &port), ("e", &e)])),
        }
    }
    Err(last)
}

/// Verbietet Zeilenumbrüche in Namen, damit niemand Befehle einschmuggeln kann.
fn clean(s: &str) -> Res<&str> {
    if s.contains('\r') || s.contains('\n') || s.contains('\0') {
        Err(RErr::Msg(t("Ungültiges Zeichen im Namen.").to_string()))
    } else {
        Ok(s)
    }
}

// ---------------------------------------------------------------- Client

pub struct Ftp {
    ctrl: Stream,
    buf: Vec<u8>,
    peer: IpAddr,
    log: Option<Sender<String>>,
    tls: Option<TlsHandle>,
    prot_p: bool,
    epsv: bool,
    feat_mlsd: bool,
    feat_size: bool,
    feat_rest: bool,
}

impl Ftp {
    pub fn connect(site: &Site, trusted: &[String], log: Option<Sender<String>>) -> Res<Ftp> {
        let host = site.host.trim();
        clean(host)?;
        log_line(&log, tf("Verbinde mit {host}:{port} …", &[("host", &host), ("port", &site.port)]));
        let tcp = connect_tcp(host, site.port)?;
        let peer = tcp.peer_addr().map_err(RErr::from)?.ip();
        let tls = if site.proto == Proto::Ftp { None } else { Some(new_tls(host, trusted)?) };
        let ctrl = if site.proto == Proto::FtpsImplicit { wrap_tls(tls.as_ref().ok_or_else(|| RErr::from("TLS"))?, tcp)? } else { Stream::Plain(tcp) };
        let mut f = Ftp { ctrl, buf: Vec::new(), peer, log, tls, prot_p: false, epsv: true, feat_mlsd: false, feat_size: true, feat_rest: true };

        let mut r = f.reply()?;
        while r.class() == 1 {
            r = f.reply()?;
        }
        f.expect(&r, 2, t("Der Server nimmt keine Verbindung an"))?;

        if site.proto == Proto::Ftps {
            let r = f.cmd("AUTH TLS")?;
            let r = if r.class() == 2 { r } else { f.cmd("AUTH SSL")? };
            f.expect(&r, 2, t("Der Server unterstützt keine Verschlüsselung (AUTH TLS)"))?;
            let tcp = f.ctrl.tcp().try_clone().map_err(RErr::from)?;
            f.ctrl = wrap_tls(f.tls.as_ref().ok_or_else(|| RErr::from("TLS"))?, tcp)?;
        }
        if f.tls.is_some() {
            let r = f.cmd("PBSZ 0")?;
            f.expect(&r, 2, "PBSZ")?;
            let r = f.cmd("PROT P")?;
            f.expect(&r, 2, t("Der Server verlangt keine geschützte Datenverbindung (PROT P)"))?;
            f.prot_p = true;
        }
        f.login(&site.user, &site.password)?;
        f.features();
        let r = f.cmd("TYPE I")?;
        f.expect(&r, 2, "TYPE I")?;
        Ok(f)
    }

    // ------------------------------------------------ Steuerkanal

    fn io_err(&self, e: io::Error) -> RErr {
        if let Some(t) = &self.tls {
            if let Some(u) = untrusted(t) {
                return u;
            }
        }
        RErr::from(e)
    }

    fn read_line(&mut self) -> Res<String> {
        loop {
            if let Some(p) = self.buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=p).collect();
                return Ok(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string());
            }
            let mut chunk = [0u8; 4096];
            let n = match self.ctrl.read(&mut chunk) {
                Ok(n) => n,
                Err(e) => return Err(self.io_err(e)),
            };
            if n == 0 {
                if let Some(t) = &self.tls {
                    if let Some(u) = untrusted(t) {
                        return Err(u);
                    }
                }
                return Err(RErr::Disconnected(t("Der Server hat die Verbindung geschlossen.").to_string()));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    fn reply(&mut self) -> Res<Reply> {
        let first = self.read_line()?;
        log_line(&self.log, format!("< {first}"));
        let b = first.as_bytes();
        if b.len() < 3 || !b[..3].iter().all(|c| c.is_ascii_digit()) {
            return Err(RErr::Msg(tf("Unverständliche Antwort des Servers: {s}", &[("s", &crate::model::short(&first, 80))])));
        }
        let code: u16 = first[..3].parse().unwrap_or(0);
        let mut text = first.get(4..).unwrap_or("").to_string();
        if b.get(3) == Some(&b'-') {
            let prefix = first[..3].to_string();
            loop {
                let l = self.read_line()?;
                log_line(&self.log, format!("< {l}"));
                let end = l.len() >= 4 && l.starts_with(&prefix) && l.as_bytes()[3] == b' ';
                text.push('\n');
                if end {
                    text.push_str(&l[4..]);
                    break;
                }
                // Fortsetzungszeilen dürfen mit Leerzeichen eingerückt sein oder die Zahl wiederholen
                text.push_str(l.strip_prefix(&format!("{prefix}-")).unwrap_or(&l));
            }
        }
        Ok(Reply { code, text })
    }

    fn send(&mut self, line: &str, shown: &str) -> Res<()> {
        log_line(&self.log, format!("> {shown}"));
        let mut data = Vec::with_capacity(line.len() + 2);
        data.extend_from_slice(line.as_bytes());
        data.extend_from_slice(b"\r\n");
        if let Err(e) = self.ctrl.write_all(&data).and_then(|_| self.ctrl.flush()) {
            return Err(self.io_err(e));
        }
        Ok(())
    }

    pub fn cmd(&mut self, line: &str) -> Res<Reply> {
        clean(line)?;
        self.send(line, line)?;
        self.reply()
    }

    fn expect(&self, r: &Reply, class: u16, what: &str) -> Res<()> {
        if r.class() == class {
            return Ok(());
        }
        if r.code == 421 {
            return Err(RErr::Disconnected(r.line()));
        }
        Err(RErr::Msg(format!("{what}: {}", r.line())))
    }

    fn login(&mut self, user: &str, pass: &str) -> Res<()> {
        let (user, pass) = if user.trim().is_empty() { ("anonymous", "ftpwerk@") } else { (user, pass) };
        clean(user)?;
        clean(pass)?;
        let r = self.cmd(&format!("USER {user}"))?;
        let r = match r.code {
            230 | 232 => return Ok(()),
            331 | 332 => {
                let line = format!("PASS {pass}");
                self.send(&line, "PASS ********")?;
                self.reply()?
            }
            _ => r,
        };
        if r.class() == 2 {
            Ok(())
        } else {
            Err(RErr::Msg(tf("Anmeldung fehlgeschlagen: {r}", &[("r", &r.line())])))
        }
    }

    fn features(&mut self) {
        let Ok(r) = self.cmd("FEAT") else { return };
        if r.class() != 2 {
            return;
        }
        let mut utf8 = false;
        for l in r.text.lines() {
            let u = l.trim().to_uppercase();
            if u.starts_with("MLST") {
                self.feat_mlsd = true;
            } else if u.starts_with("UTF8") {
                utf8 = true;
            }
        }
        if utf8 {
            let _ = self.cmd("OPTS UTF8 ON");
        }
    }

    // ------------------------------------------------ Datenkanal

    fn passive_port(&mut self) -> Res<u16> {
        if self.epsv {
            let r = self.cmd("EPSV")?;
            if r.code == 229 {
                if let Some(p) = parse_epsv(&r.text) {
                    return Ok(p);
                }
            }
            if r.code == 421 {
                return Err(RErr::Disconnected(r.line()));
            }
            self.epsv = false;
        }
        let r = self.cmd("PASV")?;
        if r.code != 227 {
            return Err(RErr::Msg(tf("Passiver Modus nicht möglich: {r}", &[("r", &r.line())])));
        }
        parse_pasv(&r.text).ok_or_else(|| RErr::Msg(tf("Unverständliche Antwort des Servers: {s}", &[("s", &crate::model::short(&r.text, 80))])))
    }

    /// Öffnet die Datenverbindung (nur die Adresse der Steuerverbindung zählt – Server hinter NAT
    /// nennen im PASV-Befehl oft eine falsche, private Adresse).
    fn open_data(&mut self) -> Res<TcpStream> {
        let port = self.passive_port()?;
        let addr = SocketAddr::new(self.peer, port);
        let s = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| RErr::Msg(tf("Datenverbindung zu {addr} fehlgeschlagen: {e}", &[("addr", &addr), ("e", &e)])))?;
        let _ = s.set_read_timeout(Some(IO_TIMEOUT));
        let _ = s.set_write_timeout(Some(IO_TIMEOUT));
        let _ = s.set_nodelay(true);
        Ok(s)
    }

    fn wrap_data(&self, tcp: TcpStream) -> Res<Stream> {
        if self.prot_p {
            wrap_tls(self.tls.as_ref().ok_or_else(|| RErr::from("TLS"))?, tcp)
        } else {
            Ok(Stream::Plain(tcp))
        }
    }

    /// Befehl mit Datenverbindung, Antwort als Text (Listen).
    fn fetch_text(&mut self, command: &str) -> Res<String> {
        let tcp = self.open_data()?;
        let r = self.cmd(command)?;
        if r.class() != 1 {
            // leere Ordner melden manche Server als «450 No files found»
            if r.code == 450 && r.text.to_lowercase().contains("no files") {
                return Ok(String::new());
            }
            if r.code == 421 {
                return Err(RErr::Disconnected(r.line()));
            }
            return Err(RErr::Msg(r.line()));
        }
        let mut data = self.wrap_data(tcp)?;
        let mut out = Vec::new();
        let mut chunk = [0u8; CHUNK];
        loop {
            match data.read_data(&mut chunk) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&chunk[..n]),
                Err(e) => return Err(self.io_err(e)),
            }
        }
        drop(data);
        let r = self.reply()?;
        if r.class() != 2 {
            return Err(RErr::Msg(r.line()));
        }
        Ok(String::from_utf8_lossy(&out).to_string())
    }

    fn do_list(&mut self, path: &str) -> Res<Vec<Entry>> {
        let r = self.cmd(&format!("CWD {}", clean(path)?))?;
        if r.class() != 2 {
            return Err(RErr::Msg(tf("Ordner «{p}» lässt sich nicht öffnen: {r}", &[("p", &path), ("r", &r.line())])));
        }
        let now = now_unix();
        if self.feat_mlsd {
            let txt = self.fetch_text("MLSD")?;
            return Ok(listparse::parse_listing(&txt, true, now));
        }
        let txt = match self.fetch_text("LIST -a") {
            Ok(t) => t,
            Err(RErr::Msg(_)) => self.fetch_text("LIST")?,
            Err(e) => return Err(e),
        };
        Ok(listparse::parse_listing(&txt, false, now))
    }

    fn do_size(&mut self, path: &str) -> Res<Option<u64>> {
        if !self.feat_size {
            return Ok(None);
        }
        let r = self.cmd(&format!("SIZE {}", clean(path)?))?;
        match r.code {
            213 => Ok(r.text.split_whitespace().next().and_then(|s| s.parse().ok())),
            500 | 502 => {
                self.feat_size = false;
                Ok(None)
            }
            421 => Err(RErr::Disconnected(r.line())),
            _ => Ok(None),
        }
    }

    fn simple(&mut self, command: String, class: u16, what: &str) -> Res<()> {
        let r = self.cmd(&command)?;
        self.expect(&r, class, what)
    }
}

/// «229 Entering Extended Passive Mode (|||6446|)» → 6446
pub fn parse_epsv(text: &str) -> Option<u16> {
    let open = text.find('(')?;
    let close = text[open..].find(')')? + open;
    let inner = &text[open + 1..close];
    let delim = inner.chars().next()?;
    let parts: Vec<&str> = inner.split(delim).collect();
    // «|||6446|» → ["", "", "", "6446", ""]
    parts.get(3)?.trim().parse().ok()
}

/// «227 Entering Passive Mode (192,168,1,5,19,137)» → 19*256+137
pub fn parse_pasv(text: &str) -> Option<u16> {
    let nums: Vec<u32> = match text.find('(') {
        Some(open) => {
            let close = text[open..].find(')').map(|c| c + open).unwrap_or(text.len());
            text[open + 1..close].split(',').filter_map(|s| s.trim().parse().ok()).collect()
        }
        None => {
            // ohne Klammern: die ersten sechs Zahlen im Text
            let start = text.find(|c: char| c.is_ascii_digit())?;
            text[start..].split(',').filter_map(|s| s.trim().trim_end_matches('.').parse().ok()).collect()
        }
    };
    if nums.len() < 6 || nums[4] > 255 || nums[5] > 255 {
        return None;
    }
    Some((nums[4] * 256 + nums[5]) as u16)
}

/// «257 "/home/x" is current directory» → /home/x (verdoppelte Anführungszeichen zählen als eines)
pub fn parse_pwd(text: &str) -> Option<String> {
    let start = text.find('"')? + 1;
    let mut out = String::new();
    let mut chars = text[start..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                out.push('"');
                chars.next();
            } else {
                return Some(out);
            }
        } else {
            out.push(c);
        }
    }
    None
}

impl Remote for Ftp {
    fn pwd(&mut self) -> Res<String> {
        let r = self.cmd("PWD")?;
        self.expect(&r, 2, "PWD")?;
        Ok(parse_pwd(&r.text).unwrap_or_else(|| "/".to_string()))
    }

    fn list(&mut self, path: &str) -> Res<Vec<Entry>> {
        self.do_list(path)
    }

    fn mkdir(&mut self, path: &str) -> Res<()> {
        self.simple(format!("MKD {}", clean(path)?), 2, t("Ordner anlegen"))
    }

    fn rename(&mut self, from: &str, to: &str) -> Res<()> {
        let r = self.cmd(&format!("RNFR {}", clean(from)?))?;
        self.expect(&r, 3, t("Umbenennen"))?;
        self.simple(format!("RNTO {}", clean(to)?), 2, t("Umbenennen"))
    }

    fn remove_file(&mut self, path: &str) -> Res<()> {
        self.simple(format!("DELE {}", clean(path)?), 2, t("Löschen"))
    }

    fn remove_dir(&mut self, path: &str) -> Res<()> {
        self.simple(format!("RMD {}", clean(path)?), 2, t("Ordner löschen"))
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Res<()> {
        self.simple(format!("SITE CHMOD {:o} {}", mode & 0o7777, clean(path)?), 2, t("Rechte ändern"))
    }

    fn size(&mut self, path: &str) -> Res<Option<u64>> {
        self.do_size(path)
    }

    fn download(&mut self, remote: &str, local: &Path, resume: bool, prog: Prog) -> Res<u64> {
        clean(remote)?;
        let total = self.do_size(remote).unwrap_or(None);
        let mut offset = 0u64;
        if resume {
            if let Ok(m) = std::fs::metadata(local) {
                offset = m.len();
                if let Some(tot) = total {
                    if offset == tot {
                        prog(Tick::Start { offset, total: tot });
                        return Ok(0);
                    }
                    if offset > tot {
                        offset = 0;
                    }
                }
            }
        }
        let tcp = self.open_data()?;
        if offset > 0 {
            let r = self.cmd(&format!("REST {offset}"))?;
            if r.code != 350 {
                offset = 0; // Server kann nicht fortsetzen: von vorn
            }
        }
        let r = self.cmd(&format!("RETR {remote}"))?;
        if r.class() != 1 {
            if r.code == 421 {
                return Err(RErr::Disconnected(r.line()));
            }
            return Err(RErr::Msg(tf("Herunterladen nicht möglich: {r}", &[("r", &r.line())])));
        }
        let mut data = self.wrap_data(tcp)?;
        let mut file = if offset > 0 { OpenOptions::new().append(true).open(local)? } else { File::create(local)? };
        if !prog(Tick::Start { offset, total: total.unwrap_or(0) }) {
            return Err(RErr::Cancelled);
        }
        let mut got = 0u64;
        let mut chunk = vec![0u8; CHUNK];
        loop {
            let n = match data.read_data(&mut chunk) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(self.io_err(e)),
            };
            file.write_all(&chunk[..n])?;
            got += n as u64;
            if !prog(Tick::Bytes(n as u64)) {
                return Err(RErr::Cancelled);
            }
        }
        file.flush()?;
        drop(data);
        let r = self.reply()?;
        if r.class() != 2 {
            return Err(RErr::Msg(tf("Herunterladen fehlgeschlagen: {r}", &[("r", &r.line())])));
        }
        if let Some(tot) = total {
            if offset + got != tot {
                return Err(RErr::Msg(tf("Datei unvollständig: {got} von {tot} Bytes", &[("got", &(offset + got)), ("tot", &tot)])));
            }
        }
        Ok(got)
    }

    fn upload(&mut self, local: &Path, remote: &str, resume: bool, prog: Prog) -> Res<u64> {
        clean(remote)?;
        let mut file = File::open(local)?;
        let total = file.metadata()?.len();
        let mut offset = 0u64;
        let mut append = false;
        if resume {
            if let Ok(Some(rs)) = self.do_size(remote) {
                if rs == total && total > 0 {
                    prog(Tick::Start { offset: rs, total });
                    return Ok(0);
                }
                if rs > 0 && rs < total {
                    offset = rs;
                    append = true;
                }
            }
        }
        let tcp = self.open_data()?;
        let r = self.cmd(&format!("{} {remote}", if append { "APPE" } else { "STOR" }))?;
        if r.class() != 1 {
            if r.code == 421 {
                return Err(RErr::Disconnected(r.line()));
            }
            return Err(RErr::Msg(tf("Hochladen nicht möglich: {r}", &[("r", &r.line())])));
        }
        let mut data = self.wrap_data(tcp)?;
        if offset > 0 {
            file.seek(SeekFrom::Start(offset))?;
        }
        if !prog(Tick::Start { offset, total }) {
            return Err(RErr::Cancelled);
        }
        let mut sent = 0u64;
        let mut chunk = vec![0u8; CHUNK];
        loop {
            let n = file.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            if let Err(e) = data.write_all(&chunk[..n]) {
                return Err(self.io_err(e));
            }
            sent += n as u64;
            if !prog(Tick::Bytes(n as u64)) {
                return Err(RErr::Cancelled);
            }
        }
        data.finish();
        drop(data);
        let r = self.reply()?;
        if r.class() != 2 {
            return Err(RErr::Msg(tf("Hochladen fehlgeschlagen: {r}", &[("r", &r.line())])));
        }
        Ok(sent)
    }

    fn keepalive(&mut self) -> Res<()> {
        let r = self.cmd("NOOP")?;
        self.expect(&r, 2, "NOOP")
    }

    fn close(&mut self) {
        let _ = self.send("QUIT", "QUIT");
        let _ = self.ctrl.tcp().shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasv_und_epsv() {
        assert_eq!(parse_pasv("Entering Passive Mode (192,168,1,5,19,137)."), Some(19 * 256 + 137));
        assert_eq!(parse_pasv("Entering Passive Mode 10,0,0,1,4,1"), Some(4 * 256 + 1));
        assert_eq!(parse_pasv("kaputt"), None);
        assert_eq!(parse_pasv("(1,2,3,4,300,1)"), None);
        assert_eq!(parse_epsv("Entering Extended Passive Mode (|||6446|)"), Some(6446));
        assert_eq!(parse_epsv("Extended Passive Mode OK (!!!2121!)"), Some(2121));
        assert_eq!(parse_epsv("nix"), None);
    }

    #[test]
    fn pwd_antwort() {
        assert_eq!(parse_pwd("\"/home/oli\" is the current directory"), Some("/home/oli".to_string()));
        assert_eq!(parse_pwd("\"/a \"\"b\"\" c\" created"), Some("/a \"b\" c".to_string()));
        assert_eq!(parse_pwd("kein Pfad"), None);
    }

    #[test]
    fn zeilenumbrueche_verboten() {
        assert!(clean("ok.txt").is_ok());
        assert!(clean("a\r\nDELE b").is_err());
        assert!(clean("a\0b").is_err());
    }
}
