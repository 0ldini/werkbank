//! Gemeinsame Typen: Verbindungsdaten, Verzeichniseinträge, Fehler, Zeit- und Pfad-Hilfen.
//! Nur Standardbibliothek – wird mit `rustc --test` geprüft.

#![allow(dead_code)]

use std::fmt;

use crate::sprache::{t, tf};

// ---------------------------------------------------------------- Protokolle

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proto {
    /// Einfaches FTP, unverschlüsselt.
    Ftp,
    /// FTP über TLS, ausdrücklich (AUTH TLS auf Port 21) – heute der übliche Weg.
    Ftps,
    /// FTP über TLS, von Anfang an (Port 990).
    FtpsImplicit,
    /// SFTP über SSH (Port 22).
    Sftp,
}

impl Proto {
    pub const ALL: [Proto; 4] = [Proto::Sftp, Proto::Ftps, Proto::FtpsImplicit, Proto::Ftp];

    pub fn default_port(self) -> u16 {
        match self {
            Proto::Ftp | Proto::Ftps => 21,
            Proto::FtpsImplicit => 990,
            Proto::Sftp => 22,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Proto::Ftp => "ftp",
            Proto::Ftps => "ftps",
            Proto::FtpsImplicit => "ftps-implicit",
            Proto::Sftp => "sftp",
        }
    }

    pub fn from_code(s: &str) -> Proto {
        match s.trim().to_lowercase().as_str() {
            "ftp" => Proto::Ftp,
            "ftps" | "ftpes" => Proto::Ftps,
            "ftps-implicit" | "ftpsi" => Proto::FtpsImplicit,
            _ => Proto::Sftp,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Proto::Ftp => "FTP (unverschlüsselt)",
            Proto::Ftps => "FTPS (explizit, TLS)",
            Proto::FtpsImplicit => "FTPS (implizit, Port 990)",
            Proto::Sftp => "SFTP (SSH)",
        }
    }

    pub fn is_ftp_family(self) -> bool {
        !matches!(self, Proto::Sftp)
    }
}

/// Eine gespeicherte oder schnell eingegebene Verbindung.
#[derive(Clone, Debug)]
pub struct Site {
    pub name: String,
    pub proto: Proto,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Passwort (oder Schlüssel-Passphrase). Nur im Speicher; auf die Platte nur verschlüsselt (Windows).
    pub password: String,
    pub save_password: bool,
    /// Pfad zu einer privaten Schlüsseldatei (nur SFTP). Leer = Passwort.
    pub key_file: String,
    /// Startordner auf dem Server (leer = Heimordner).
    pub remote_dir: String,
    /// Startordner lokal (leer = Heimordner).
    pub local_dir: String,
}

impl Default for Site {
    fn default() -> Self {
        Site {
            name: String::new(),
            proto: Proto::Sftp,
            host: String::new(),
            port: 22,
            user: String::new(),
            password: String::new(),
            save_password: false,
            key_file: String::new(),
            remote_dir: String::new(),
            local_dir: String::new(),
        }
    }
}

impl Site {
    pub fn title(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.clone()
        } else if self.user.is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.user, self.host)
        }
    }

    pub fn hostport(&self) -> String {
        format!("{}:{}", self.host.trim().to_lowercase(), self.port)
    }
}

// ---------------------------------------------------------------- Einträge

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    /// Sekunden seit 1970 (UTC); Server-Listen ohne Zeitzone werden als UTC gelesen.
    pub mtime: Option<i64>,
    /// «rwxr-xr-x» oder leer, wenn der Server nichts liefert.
    pub perms: String,
    /// Zugriffsrechte als Zahl (z. B. 0o755), wenn bekannt.
    pub mode: Option<u32>,
}

impl Entry {
    pub fn dir(name: &str) -> Entry {
        Entry { name: name.to_string(), is_dir: true, ..Default::default() }
    }
}

/// Sortierung: Ordner zuerst, dann nach Spalte.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortCol {
    Name,
    Size,
    Time,
}

pub fn sort_entries(v: &mut [Entry], col: SortCol, asc: bool) {
    v.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            let o = match col {
                SortCol::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortCol::Size => a.size.cmp(&b.size).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                SortCol::Time => a.mtime.cmp(&b.mtime).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            };
            if asc { o } else { o.reverse() }
        })
    });
}

// ---------------------------------------------------------------- Fehler

/// Fehler einer Server-Operation. Zwei Fälle brauchen eine Rückfrage an den Benutzer.
#[derive(Clone, Debug, PartialEq)]
pub enum RErr {
    Msg(String),
    /// Der Benutzer hat abgebrochen.
    Cancelled,
    /// FTPS: Zertifikat nicht von einer bekannten Stelle (z. B. selbst signiert) oder geändert.
    UntrustedCert { fp: String, detail: String, changed: bool },
    /// SFTP: Server-Schlüssel noch unbekannt oder geändert.
    HostKey { algo: String, fp: String, changed: bool },
    /// Verbindung verloren – es lohnt sich, neu zu verbinden.
    Disconnected(String),
}

pub type Res<T> = Result<T, RErr>;

impl From<String> for RErr {
    fn from(s: String) -> RErr {
        RErr::Msg(s)
    }
}

impl From<&str> for RErr {
    fn from(s: &str) -> RErr {
        RErr::Msg(s.to_string())
    }
}

impl From<std::io::Error> for RErr {
    fn from(e: std::io::Error) -> RErr {
        use std::io::ErrorKind::*;
        match e.kind() {
            ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof | TimedOut | WouldBlock | NotConnected => RErr::Disconnected(e.to_string()),
            _ => RErr::Msg(e.to_string()),
        }
    }
}

impl fmt::Display for RErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RErr::Msg(s) | RErr::Disconnected(s) => f.write_str(s),
            RErr::Cancelled => f.write_str(t("Abgebrochen")),
            RErr::UntrustedCert { .. } => f.write_str(t("Dem Zertifikat des Servers wird nicht vertraut.")),
            RErr::HostKey { changed, .. } => {
                if *changed {
                    f.write_str(t("Der Schlüssel des Servers hat sich geändert!"))
                } else {
                    f.write_str(t("Der Server ist noch nicht bekannt."))
                }
            }
        }
    }
}

impl RErr {
    pub fn is_disconnect(&self) -> bool {
        matches!(self, RErr::Disconnected(_))
    }
}

// ---------------------------------------------------------------- Pfade (Server: immer «/»)

/// Fügt einen Namen an einen Serverpfad.
pub fn rjoin(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// Eine Ebene höher; «/» bleibt «/».
pub fn rparent(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => p[..i].to_string(),
        None => {
            if path.starts_with('/') {
                "/".to_string()
            } else {
                ".".to_string()
            }
        }
    }
}

/// Letzter Teil eines Serverpfads.
pub fn rname(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(i) => p[i + 1..].to_string(),
        None => p.to_string(),
    }
}

/// Windows-Pfade wie «C:\Users\x» werden hier nicht angefasst; das ist nur für Serverpfade.
pub fn rnormalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let abs = path.starts_with('/');
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    let j = parts.join("/");
    if abs {
        format!("/{j}")
    } else if j.is_empty() {
        ".".to_string()
    } else {
        j
    }
}

// ---------------------------------------------------------------- Anzeige

pub fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{:.0} {}", v, U[i])
    } else if v >= 10.0 {
        format!("{:.1} {}", v, U[i])
    } else {
        format!("{:.2} {}", v, U[i])
    }
}

pub fn human_speed(bytes_per_s: f64) -> String {
    format!("{}/s", human_size(bytes_per_s.max(0.0) as u64))
}

pub fn human_eta(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "–".to_string();
    }
    let s = secs.round() as u64;
    if s >= 3600 {
        format!("{} h {:02} min", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

/// Tage seit 1970 → (Jahr, Monat 1–12, Tag 1–31). Verfahren nach Howard Hinnant.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (Jahr, Monat, Tag) → Tage seit 1970.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Sekunden seit 1970 aus Datum und Uhrzeit (UTC).
pub fn unix_from_parts(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    days_from_civil(y, mo, d) * 86_400 + h as i64 * 3600 + mi as i64 * 60 + s as i64
}

/// «2026-10-18 13:47»
pub fn fmt_time(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// «rwxr-xr-x» → 0o755. Sonderbits (s, t) werden mitgezählt.
pub fn mode_from_perms(p: &str) -> Option<u32> {
    let b: Vec<char> = p.chars().collect();
    if b.len() < 9 {
        return None;
    }
    let b = &b[b.len() - 9..];
    let mut m = 0u32;
    for (i, c) in b.iter().enumerate() {
        let bit = 1u32 << (8 - i);
        match (i % 3, *c) {
            (_, 'r') | (_, 'w') | (_, 'x') => m |= bit,
            (2, 's') | (2, 't') => m |= bit,
            (_, 'S') | (_, 'T') | (_, '-') => {}
            (_, 's') | (_, 't') => {}
            _ => return None,
        }
    }
    if matches!(b[2], 's' | 'S') {
        m |= 0o4000;
    }
    if matches!(b[5], 's' | 'S') {
        m |= 0o2000;
    }
    if matches!(b[8], 't' | 'T') {
        m |= 0o1000;
    }
    Some(m)
}

/// 0o755 → «rwxr-xr-x»
pub fn perms_from_mode(m: u32) -> String {
    let mut s = String::new();
    for i in 0..9 {
        let bit = 1u32 << (8 - i);
        let c = ['r', 'w', 'x'][i % 3];
        s.push(if m & bit != 0 { c } else { '-' });
    }
    s
}

/// Text für Fehlermeldungen: lange Server-Antworten kürzen.
pub fn short(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(n).collect();
        o.push('…');
        o
    }
}

/// Fehlermeldung mit Zusatz, z. B. «Ordner anlegen: 550 Permission denied».
pub fn with_ctx(what: &str, e: impl fmt::Display) -> RErr {
    RErr::Msg(tf("{what}: {e}", &[("what", &what), ("e", &e)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serverpfade() {
        assert_eq!(rjoin("/a/b", "c"), "/a/b/c");
        assert_eq!(rjoin("/", "c"), "/c");
        assert_eq!(rjoin("", "c"), "c");
        assert_eq!(rparent("/a/b/c"), "/a/b");
        assert_eq!(rparent("/a"), "/");
        assert_eq!(rparent("/"), "/");
        assert_eq!(rparent("a/b"), "a");
        assert_eq!(rname("/a/b/c/"), "c");
        assert_eq!(rnormalize("/a/./b/../c"), "/a/c");
        assert_eq!(rnormalize("/../x"), "/x");
    }

    #[test]
    fn zeit() {
        // 2026-10-18 13:47:00 UTC
        let s = unix_from_parts(2026, 10, 18, 13, 47, 0);
        assert_eq!(fmt_time(s), "2026-10-18 13:47");
        assert_eq!(fmt_time(0), "1970-01-01 00:00");
        assert_eq!(fmt_time(unix_from_parts(2024, 2, 29, 23, 59, 0)), "2024-02-29 23:59");
        let (y, m, d) = civil_from_days(days_from_civil(2000, 3, 1));
        assert_eq!((y, m, d), (2000, 3, 1));
    }

    #[test]
    fn groessen() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.50 KB");
        assert_eq!(human_size(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(human_eta(75.0), "1 min 15 s");
        assert_eq!(human_eta(3725.0), "1 h 02 min");
    }

    #[test]
    fn rechte() {
        assert_eq!(mode_from_perms("rwxr-xr-x"), Some(0o755));
        assert_eq!(mode_from_perms("rw-r--r--"), Some(0o644));
        assert_eq!(mode_from_perms("rwsr-xr-x"), Some(0o4755));
        assert_eq!(mode_from_perms("drwxr-xr-x"), Some(0o755));
        assert_eq!(perms_from_mode(0o640), "rw-r-----");
        assert_eq!(mode_from_perms("xyz"), None);
    }

    #[test]
    fn sortierung() {
        let mut v = vec![
            Entry { name: "b.txt".into(), size: 5, ..Default::default() },
            Entry::dir("Zeta"),
            Entry { name: "A.txt".into(), size: 9, ..Default::default() },
            Entry::dir("alpha"),
        ];
        sort_entries(&mut v, SortCol::Name, true);
        let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "A.txt", "b.txt"]);
        sort_entries(&mut v, SortCol::Size, false);
        assert_eq!(v[2].name, "A.txt");
    }
}
