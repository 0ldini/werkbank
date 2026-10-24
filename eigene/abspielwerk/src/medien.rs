//! Dateiarten, Ordner lesen, natürliche Sortierung, Zeitanzeige.
//!
//! Ohne egui und ohne libVLC, damit es sich einzeln prüfen lässt:
//! `rustc --edition 2021 --test src/medien.rs -o /tmp/medien && /tmp/medien`

#![allow(dead_code)]

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Art {
    Video,
    Musik,
    Bild,
}

pub const VIDEO: &[&str] = &[
    "mp4", "m4v", "mkv", "mov", "avi", "webm", "wmv", "flv", "mpg", "mpeg", "m2v", "ts", "mts", "m2ts", "3gp", "ogv", "vob", "divx",
];
pub const MUSIK: &[&str] = &["mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav", "wma", "aif", "aiff", "ape", "mka", "wv", "alac"];
pub const BILD: &[&str] = &["jpg", "jpeg", "jpe", "png", "webp", "gif", "bmp", "tif", "tiff"];

/// So viele Einträge nimmt die Wiedergabeliste höchstens auf.
pub const MAX_EINTRAEGE: usize = 20_000;

/// Art einer Datei nach ihrer Endung (gross/klein egal).
pub fn art_von(p: &Path) -> Option<Art> {
    let e = p.extension()?.to_str()?.to_ascii_lowercase();
    if VIDEO.contains(&e.as_str()) {
        Some(Art::Video)
    } else if MUSIK.contains(&e.as_str()) {
        Some(Art::Musik)
    } else if BILD.contains(&e.as_str()) {
        Some(Art::Bild)
    } else {
        None
    }
}

fn versteckt(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))
}

/// Alle abspielbaren Dateien eines Ordners (nicht in Unterordnern), natürlich sortiert.
pub fn ordner_lesen(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !versteckt(p) && art_von(p).is_some())
        .take(MAX_EINTRAEGE)
        .collect();
    sortieren(&mut v);
    Ok(v)
}

/// Dateien und Ordner (z. B. hineingezogen) zu einer Liste abspielbarer Dateien auflösen.
/// Ordner zählen mit ihrem Inhalt (ohne Unterordner); die Reihenfolge der Eingabe bleibt.
pub fn aufloesen(pfade: &[PathBuf]) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for p in pfade {
        if v.len() >= MAX_EINTRAEGE {
            break;
        }
        if p.is_dir() {
            if let Ok(mut inhalt) = ordner_lesen(p) {
                v.append(&mut inhalt);
            }
        } else if art_von(p).is_some() {
            v.push(p.clone());
        }
    }
    v.truncate(MAX_EINTRAEGE);
    v
}

pub fn sortieren(v: &mut [PathBuf]) {
    v.sort_by(|a, b| {
        let na = a.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let nb = b.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        natuerlich(&na, &nb)
    });
}

/// Natürliche Reihenfolge: «Bild 2» vor «Bild 10», gross/klein egal.
pub fn natuerlich(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut da = String::new();
                while let Some(c) = a.peek().copied().filter(char::is_ascii_digit) {
                    da.push(c);
                    a.next();
                }
                let mut db = String::new();
                while let Some(c) = b.peek().copied().filter(char::is_ascii_digit) {
                    db.push(c);
                    b.next();
                }
                let ta = da.trim_start_matches('0');
                let tb = db.trim_start_matches('0');
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| db.len().cmp(&da.len()));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Millisekunden als «0:42» oder «1:23:10».
pub fn zeit_text(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// «file:///C:/Users/a%20b/x.jpg» → Pfad (libVLC liefert das Albumbild so).
pub fn url_zu_pfad(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    let s = String::from_utf8(out).ok()?;
    // Windows: «/C:/…» → «C:/…»
    let s = if cfg!(windows) && s.len() > 2 && s.starts_with('/') && s.as_bytes().get(2) == Some(&b':') { s[1..].to_string() } else { s };
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

/// Kleiner Zufallsgenerator (xorshift) für «Zufall»; braucht keine weitere Bibliothek.
pub struct Zufall(u64);

impl Zufall {
    pub fn neu() -> Zufall {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E37_79B9_7F4A_7C15);
        Zufall(t | 1)
    }
    pub fn zahl(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Zahl in 0..n (n > 0).
    pub fn bis(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.zahl() % n as u64) as usize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arten() {
        assert_eq!(art_von(Path::new("a/B.MKV")), Some(Art::Video));
        assert_eq!(art_von(Path::new("lied.flac")), Some(Art::Musik));
        assert_eq!(art_von(Path::new("x.JPeG")), Some(Art::Bild));
        assert_eq!(art_von(Path::new("x.txt")), None);
        assert_eq!(art_von(Path::new("ohne")), None);
    }

    #[test]
    fn sortierung() {
        let mut v = vec!["IMG_10.jpg", "img_2.jpg", "IMG_1.jpg", "a.mp3", "IMG_002.jpg"];
        v.sort_by(|a, b| natuerlich(a, b));
        assert_eq!(v, ["a.mp3", "IMG_1.jpg", "IMG_002.jpg", "img_2.jpg", "IMG_10.jpg"]);
        assert_eq!(natuerlich("Ä1", "ä1"), Ordering::Equal);
        assert_eq!(natuerlich("99999999999999999999999", "1"), Ordering::Greater);
    }

    #[test]
    fn zeiten() {
        assert_eq!(zeit_text(42_000), "0:42");
        assert_eq!(zeit_text(5_010_999), "1:23:30");
        assert_eq!(zeit_text(-5), "0:00");
        assert_eq!(zeit_text(i64::MAX), format!("{}:{:02}:{:02}", i64::MAX / 1000 / 3600, (i64::MAX / 1000 / 60) % 60, (i64::MAX / 1000) % 60));
    }

    #[test]
    fn urls() {
        let p = url_zu_pfad("file:///home/oli/a%20b/c%C3%A4.jpg").unwrap();
        assert_eq!(p, PathBuf::from("/home/oli/a b/cä.jpg"));
        assert_eq!(url_zu_pfad("http://x"), None);
        assert_eq!(url_zu_pfad("file://"), None);
        assert!(url_zu_pfad("file:///x%2").is_some());
        assert!(url_zu_pfad("file:///x%zz%").is_some());
    }

    #[test]
    fn zufall() {
        let mut z = Zufall::neu();
        for _ in 0..1000 {
            assert!(z.bis(7) < 7);
        }
        assert_eq!(z.bis(0), 0);
    }
}
