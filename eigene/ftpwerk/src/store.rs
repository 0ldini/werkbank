//! Speichern und Laden: Verbindungen, vertraute Zertifikate/Schlüssel und Einstellungen.
//!
//! Alles liegt als einfache Textdateien im Einstellungsordner der Werkbank (neben `sprache.txt`).
//! Passwörter werden nur unter Windows gespeichert, und zwar mit der Windows-Datenschutz-Schnittstelle
//! (DPAPI) verschlüsselt: nur dasselbe Windows-Benutzerkonto kann sie wieder lesen.
//! Auf anderen Systemen fragt FTPwerk bei jeder Verbindung nach dem Passwort.

#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use crate::model::{Proto, Site};

// ---------------------------------------------------------------- Ordner

pub fn config_dir() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Werkbank").join("config").join("ftpwerk")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Werkbank/ftpwerk")
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        base.join("werkbank").join("ftpwerk")
    };
    Some(dir)
}

fn file(name: &str) -> Option<PathBuf> {
    let d = config_dir()?;
    fs::create_dir_all(&d).ok()?;
    Some(d.join(name))
}

pub fn home_dir() -> PathBuf {
    let v = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") };
    v.map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(|| PathBuf::from("."))
}

// ---------------------------------------------------------------- Textformat

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "")
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------- Base64 (für verschlüsselte Passwörter)

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for ch in data.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if ch.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if ch.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.trim().bytes() {
        if c == b'=' {
            break;
        }
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- Passwort-Schutz (Windows: DPAPI)

#[cfg(windows)]
mod dpapi {
    use std::ffi::c_void;

    #[repr(C)]
    struct DataBlob {
        cb: u32,
        pb: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(pin: *const DataBlob, descr: *const u16, entropy: *const DataBlob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, pout: *mut DataBlob) -> i32;
        fn CryptUnprotectData(pin: *const DataBlob, descr: *mut *mut u16, entropy: *const DataBlob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, pout: *mut DataBlob) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h: *mut c_void) -> *mut c_void;
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    fn entropy() -> DataBlob {
        static E: &[u8] = b"FTPwerk";
        DataBlob { cb: E.len() as u32, pb: E.as_ptr() as *mut u8 }
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let input = DataBlob { cb: data.len() as u32, pb: data.as_ptr() as *mut u8 };
            let ent = entropy();
            let mut out = DataBlob { cb: 0, pb: std::ptr::null_mut() };
            let ok = CryptProtectData(&input, std::ptr::null(), &ent, std::ptr::null_mut(), std::ptr::null_mut(), CRYPTPROTECT_UI_FORBIDDEN, &mut out);
            if ok == 0 || out.pb.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(out.pb, out.cb as usize).to_vec();
            LocalFree(out.pb as *mut c_void);
            Some(v)
        }
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        unsafe {
            let input = DataBlob { cb: data.len() as u32, pb: data.as_ptr() as *mut u8 };
            let ent = entropy();
            let mut out = DataBlob { cb: 0, pb: std::ptr::null_mut() };
            let ok = CryptUnprotectData(&input, std::ptr::null_mut(), &ent, std::ptr::null_mut(), std::ptr::null_mut(), CRYPTPROTECT_UI_FORBIDDEN, &mut out);
            if ok == 0 || out.pb.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(out.pb, out.cb as usize).to_vec();
            LocalFree(out.pb as *mut c_void);
            Some(v)
        }
    }
}

/// Kann FTPwerk Passwörter sicher ablegen?
pub fn can_save_password() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
fn protect(s: &str) -> Option<String> {
    dpapi::protect(s.as_bytes()).map(|b| format!("dpapi:{}", b64_encode(&b)))
}

#[cfg(not(windows))]
fn protect(_s: &str) -> Option<String> {
    None
}

#[cfg(windows)]
fn unprotect(s: &str) -> Option<String> {
    let raw = b64_decode(s.strip_prefix("dpapi:")?)?;
    dpapi::unprotect(&raw).and_then(|b| String::from_utf8(b).ok())
}

#[cfg(not(windows))]
fn unprotect(_s: &str) -> Option<String> {
    None
}

// ---------------------------------------------------------------- Verbindungen

pub fn sites_to_text(sites: &[Site]) -> String {
    let mut out = String::from("# FTPwerk – gespeicherte Verbindungen\n");
    for s in sites {
        out.push_str("\n[verbindung]\n");
        out.push_str(&format!("name={}\nproto={}\nhost={}\nport={}\nuser={}\n", esc(&s.name), s.proto.code(), esc(&s.host), s.port, esc(&s.user)));
        if !s.key_file.is_empty() {
            out.push_str(&format!("key={}\n", esc(&s.key_file)));
        }
        if !s.remote_dir.is_empty() {
            out.push_str(&format!("remote={}\n", esc(&s.remote_dir)));
        }
        if !s.local_dir.is_empty() {
            out.push_str(&format!("local={}\n", esc(&s.local_dir)));
        }
        if s.save_password && !s.password.is_empty() {
            if let Some(p) = protect(&s.password) {
                out.push_str(&format!("pass={p}\n"));
            }
        }
    }
    out
}

pub fn sites_from_text(text: &str) -> Vec<Site> {
    let mut out: Vec<Site> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim() == "[verbindung]" {
            out.push(Site::default());
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "name" => cur.name = unesc(v),
            "proto" => {
                cur.proto = Proto::from_code(v);
                cur.port = cur.proto.default_port();
            }
            "host" => cur.host = unesc(v),
            "port" => cur.port = v.trim().parse().unwrap_or_else(|_| cur.proto.default_port()),
            "user" => cur.user = unesc(v),
            "key" => cur.key_file = unesc(v),
            "remote" => cur.remote_dir = unesc(v),
            "local" => cur.local_dir = unesc(v),
            "pass" => {
                if let Some(p) = unprotect(v.trim()) {
                    cur.password = p;
                    cur.save_password = true;
                }
            }
            _ => {}
        }
    }
    out.retain(|s| !s.host.trim().is_empty());
    out
}

pub fn load_sites() -> Vec<Site> {
    file("verbindungen.txt").and_then(|p| fs::read_to_string(p).ok()).map(|t| sites_from_text(&t)).unwrap_or_default()
}

pub fn save_sites(sites: &[Site]) -> Result<(), String> {
    let p = file("verbindungen.txt").ok_or_else(|| "Kein Einstellungsordner".to_string())?;
    fs::write(p, sites_to_text(sites)).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- Vertraute Zertifikate und Schlüssel

/// Fingerabdrücke, denen der Benutzer vertraut: je Zeile «art TAB server:port TAB fingerabdruck».
#[derive(Clone, Default, Debug)]
pub struct Trust {
    lines: Vec<(String, String, String)>,
}

impl Trust {
    pub fn parse(text: &str) -> Trust {
        let lines = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let mut p = l.splitn(3, '\t');
                Some((p.next()?.to_string(), p.next()?.to_string(), p.next()?.trim().to_string()))
            })
            .collect();
        Trust { lines }
    }

    pub fn load() -> Trust {
        file("vertraut.txt").and_then(|p| fs::read_to_string(p).ok()).map(|t| Trust::parse(&t)).unwrap_or_default()
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# FTPwerk – vertraute Server (art, server:port, fingerabdruck)\n");
        for (k, h, f) in &self.lines {
            s.push_str(&format!("{k}\t{h}\t{f}\n"));
        }
        s
    }

    pub fn save(&self) {
        if let Some(p) = file("vertraut.txt") {
            let _ = fs::write(p, self.to_text());
        }
    }

    /// Alle vertrauten Fingerabdrücke für diesen Server. `kind`: «tls» oder «ssh».
    pub fn for_host(&self, kind: &str, hostport: &str) -> Vec<String> {
        self.lines.iter().filter(|(k, h, _)| k == kind && h == hostport).map(|(_, _, f)| f.clone()).collect()
    }

    /// Merkt sich einen Fingerabdruck. Ist der Server schon bekannt, wird der alte ersetzt
    /// (so bleibt nach einem bewusst bestätigten Wechsel nur der neue gültig).
    pub fn trust(&mut self, kind: &str, hostport: &str, fp: &str) {
        self.lines.retain(|(k, h, _)| !(k == kind && h == hostport));
        self.lines.push((kind.to_string(), hostport.to_string(), fp.to_string()));
    }

    pub fn forget(&mut self, kind: &str, hostport: &str) {
        self.lines.retain(|(k, h, _)| !(k == kind && h == hostport));
    }

    pub fn all(&self) -> &[(String, String, String)] {
        &self.lines
    }
}

// ---------------------------------------------------------------- Einstellungen

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overwrite {
    /// Angefangene Dateien fortsetzen, sonst überschreiben.
    Resume,
    Overwrite,
    Skip,
}

impl Overwrite {
    pub fn code(self) -> &'static str {
        match self {
            Overwrite::Resume => "resume",
            Overwrite::Overwrite => "overwrite",
            Overwrite::Skip => "skip",
        }
    }
    pub fn from_code(s: &str) -> Overwrite {
        match s.trim() {
            "overwrite" => Overwrite::Overwrite,
            "skip" => Overwrite::Skip,
            _ => Overwrite::Resume,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub parallel: usize,
    pub overwrite: Overwrite,
    pub show_hidden: bool,
    pub last_local: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { parallel: 2, overwrite: Overwrite::Resume, show_hidden: true, last_local: String::new() }
    }
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k.trim() {
                "parallel" => s.parallel = v.trim().parse::<usize>().unwrap_or(2).clamp(1, 6),
                "overwrite" => s.overwrite = Overwrite::from_code(v),
                "hidden" => s.show_hidden = v.trim() != "0",
                "local" => s.last_local = unesc(v),
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        format!("parallel={}\noverwrite={}\nhidden={}\nlocal={}\n", self.parallel, self.overwrite.code(), if self.show_hidden { 1 } else { 0 }, esc(&self.last_local))
    }

    pub fn load() -> Settings {
        file("einstellungen.txt").and_then(|p| fs::read_to_string(p).ok()).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = file("einstellungen.txt") {
            let _ = fs::write(p, self.to_text());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_rundlauf() {
        for s in ["", "a", "ab", "abc", "abcd", "Grüezi mit Ümlaut ✓"] {
            let e = b64_encode(s.as_bytes());
            assert_eq!(b64_decode(&e).unwrap(), s.as_bytes(), "{s}");
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
        assert_eq!(b64_encode(b"Ma"), "TWE=");
        assert_eq!(b64_encode(b"M"), "TQ==");
        assert!(b64_decode("TW?u").is_none());
    }

    #[test]
    fn verbindungen_rundlauf() {
        let a = Site {
            name: "Mein Server".into(),
            proto: Proto::Ftps,
            host: "ftp.example.org".into(),
            port: 2121,
            user: "oli".into(),
            key_file: String::new(),
            remote_dir: "/var/www".into(),
            local_dir: "C:\\Web\\site".into(),
            ..Default::default()
        };
        let b = Site { name: "Zweiter".into(), proto: Proto::Sftp, host: "10.0.0.5".into(), port: 22, user: "root".into(), key_file: "/home/oli/.ssh/id_ed25519".into(), ..Default::default() };
        let txt = sites_to_text(&[a.clone(), b.clone()]);
        let v = sites_from_text(&txt);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "Mein Server");
        assert_eq!(v[0].proto, Proto::Ftps);
        assert_eq!(v[0].port, 2121);
        assert_eq!(v[0].local_dir, "C:\\Web\\site");
        assert_eq!(v[1].key_file, "/home/oli/.ssh/id_ed25519");
        assert_eq!(v[1].proto, Proto::Sftp);
    }

    #[test]
    fn passwort_wird_nie_im_klartext_geschrieben() {
        let a = Site { host: "h".into(), password: "streng geheim".into(), save_password: true, ..Default::default() };
        let txt = sites_to_text(&[a]);
        assert!(!txt.contains("streng geheim"));
    }

    #[test]
    fn vertrauen() {
        let mut t = Trust::default();
        t.trust("ssh", "example.org:22", "SHA256:AAA");
        t.trust("tls", "example.org:21", "AB:CD");
        assert_eq!(t.for_host("ssh", "example.org:22"), vec!["SHA256:AAA".to_string()]);
        t.trust("ssh", "example.org:22", "SHA256:BBB");
        assert_eq!(t.for_host("ssh", "example.org:22"), vec!["SHA256:BBB".to_string()]);
        let t2 = Trust::parse(&t.to_text());
        assert_eq!(t2.for_host("tls", "example.org:21"), vec!["AB:CD".to_string()]);
        assert!(t2.for_host("tls", "anderer:21").is_empty());
    }

    #[test]
    fn einstellungen() {
        let s = Settings { parallel: 4, overwrite: Overwrite::Skip, show_hidden: false, last_local: "D:\\x\ny".into() };
        let r = Settings::parse(&s.to_text());
        assert_eq!(r.parallel, 4);
        assert_eq!(r.overwrite, Overwrite::Skip);
        assert!(!r.show_hidden);
        assert_eq!(r.last_local, "D:\\x\ny");
        assert_eq!(Settings::parse("parallel=99").parallel, 6);
    }
}
