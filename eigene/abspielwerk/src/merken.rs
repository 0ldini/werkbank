//! Was Abspielwerk sich merkt und schreibt: Fortsetzen-Positionen pro Datei, «Zuletzt
//! geöffnet», Wiedergabelisten (M3U lesen/schreiben, XSPF schreiben), Ordner und Name für
//! Bildschirmfotos.
//!
//! Ohne egui und ohne libVLC, damit es sich einzeln prüfen lässt:
//! `rustc --edition 2021 --test src/merken.rs -o /tmp/merken && /tmp/merken`

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// So viele Positionen merkt sich «Fortsetzen» höchstens.
pub const MAX_POSITIONEN: usize = 200;
/// So viele Einträge hat «Zuletzt geöffnet».
pub const MAX_ZULETZT: usize = 10;

fn sauber(s: &str) -> bool {
    !s.is_empty() && !s.contains(['\n', '\r', '\t'])
}

// ------------------------------------------------------------------ Fortsetzen

/// Wiedergabe-Positionen pro Datei oder Adresse (neueste zuerst).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Positionen {
    v: Vec<(String, i64)>,
}

impl Positionen {
    /// Position `ms` von `laenge` merken. Kurze Stücke (unter `min_laenge`), der Anfang
    /// (erste 5 s) und das Ende (letzte 10 s oder 95 %) werden nicht gemerkt, sondern vergessen.
    pub fn merken(&mut self, key: &str, ms: i64, laenge: i64, min_laenge: i64) {
        if !sauber(key) || laenge <= 0 {
            return;
        }
        self.v.retain(|(k, _)| k != key);
        let ende = laenge.saturating_sub(10_000).min(laenge / 100 * 95);
        if laenge < min_laenge || ms < 5_000 || ms >= ende {
            return;
        }
        self.v.insert(0, (key.to_string(), ms));
        self.v.truncate(MAX_POSITIONEN);
    }

    pub fn holen(&self, key: &str) -> Option<i64> {
        self.v.iter().find(|(k, _)| k == key).map(|(_, ms)| *ms)
    }

    pub fn vergessen(&mut self, key: &str) {
        self.v.retain(|(k, _)| k != key);
    }

    pub fn len(&self) -> usize {
        self.v.len()
    }

    /// Eine Zeile pro Eintrag: «ms<Tab>Datei».
    pub fn to_text(&self) -> String {
        self.v.iter().map(|(k, ms)| format!("{ms}\t{k}\n")).collect()
    }

    pub fn from_text(s: &str) -> Positionen {
        let mut v: Vec<(String, i64)> = Vec::new();
        for l in s.lines() {
            let Some((ms, k)) = l.split_once('\t') else { continue };
            let Ok(ms) = ms.trim().parse::<i64>() else { continue };
            if ms > 0 && sauber(k) && !v.iter().any(|(x, _)| x == k) {
                v.push((k.to_string(), ms));
            }
            if v.len() >= MAX_POSITIONEN {
                break;
            }
        }
        Positionen { v }
    }
}

// ------------------------------------------------------------------ Zuletzt geöffnet

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zuletzt {
    pub v: Vec<String>,
}

impl Zuletzt {
    pub fn hinzufuegen(&mut self, s: &str) {
        if !sauber(s) {
            return;
        }
        self.v.retain(|x| x != s);
        self.v.insert(0, s.to_string());
        self.v.truncate(MAX_ZULETZT);
    }

    pub fn to_text(&self) -> String {
        self.v.iter().map(|s| format!("{s}\n")).collect()
    }

    pub fn from_text(s: &str) -> Zuletzt {
        let mut z = Zuletzt::default();
        for l in s.lines().rev() {
            z.hinzufuegen(l);
        }
        z
    }
}

// ------------------------------------------------------------------ Adressen

/// Ist `s` eine Adresse (http://, rtsp://, file:// …) statt eines Pfads?
pub fn ist_url(s: &str) -> bool {
    match s.split_once("://") {
        Some((schema, rest)) => {
            !rest.is_empty() && schema.len() >= 2 && schema.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') && schema.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// Pfad → «file:///…»-Adresse (Prozent-kodiert, Windows-Pfade mit Schrägstrichen).
pub fn pfad_zu_url(p: &str) -> String {
    let s = p.replace('\\', "/");
    let mut out = String::with_capacity(s.len() + 8);
    let (vorne, rest) = if let Some(r) = s.strip_prefix("//") {
        // UNC: \\server\freigabe\… → file://server/freigabe/…
        ("file://", r.to_string())
    } else if s.as_bytes().get(1) == Some(&b':') && s.as_bytes().first().is_some_and(|c| c.is_ascii_alphabetic()) {
        ("file:///", s.clone())
    } else if s.starts_with('/') {
        ("file://", s.clone())
    } else {
        ("file:///", s.clone())
    };
    out.push_str(vorne);
    for b in rest.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// «%20» usw. auflösen (für «file://»-Adressen aus Wiedergabelisten).
pub fn url_dekodieren(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b.get(i).copied().unwrap_or(0);
        if c == b'%' {
            if let Some(v) = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// «file:///C:/a%20b» → «C:/a b», «file:///home/x» → «/home/x».
pub fn file_url_zu_pfad(s: &str) -> Option<String> {
    let rest = s.strip_prefix("file://")?;
    let d = url_dekodieren(rest);
    let b = d.as_bytes();
    if b.len() > 2 && b.first() == Some(&b'/') && b.get(2) == Some(&b':') {
        return d.get(1..).map(str::to_string);
    }
    if !d.starts_with('/') && !d.is_empty() {
        // file://server/freigabe → //server/freigabe
        return Some(format!("//{d}"));
    }
    (!d.is_empty()).then_some(d)
}

// ------------------------------------------------------------------ Wiedergabelisten

/// Eintrag einer Wiedergabeliste zum Speichern.
pub struct ListenEintrag<'a> {
    /// Pfad oder Adresse
    pub ort: &'a str,
    pub titel: &'a str,
    pub dauer_ms: Option<i64>,
}

fn eine_zeile(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

/// Erweiterte M3U-Liste (UTF-8).
pub fn m3u_schreiben(v: &[ListenEintrag]) -> String {
    let mut s = String::from("#EXTM3U\n");
    for e in v {
        let sek = e.dauer_ms.filter(|d| *d > 0).map(|d| d / 1000).unwrap_or(-1);
        s.push_str(&format!("#EXTINF:{sek},{}\n{}\n", eine_zeile(e.titel), eine_zeile(e.ort)));
    }
    s
}

pub fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => o.push(c),
        }
    }
    o
}

/// XSPF-Liste (wie VLC sie speichert, ohne VLC-eigene Erweiterungen).
pub fn xspf_schreiben(v: &[ListenEintrag], titel: &str) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<playlist version=\"1\" xmlns=\"http://xspf.org/ns/0/\">\n");
    s.push_str(&format!("  <title>{}</title>\n  <trackList>\n", xml_escape(titel)));
    for e in v {
        let ort = if ist_url(e.ort) { e.ort.to_string() } else { pfad_zu_url(e.ort) };
        s.push_str("    <track>\n");
        s.push_str(&format!("      <location>{}</location>\n", xml_escape(&ort)));
        if !e.titel.is_empty() {
            s.push_str(&format!("      <title>{}</title>\n", xml_escape(e.titel)));
        }
        if let Some(d) = e.dauer_ms.filter(|d| *d > 0) {
            s.push_str(&format!("      <duration>{d}</duration>\n"));
        }
        s.push_str("    </track>\n");
    }
    s.push_str("  </trackList>\n</playlist>\n");
    s
}

/// M3U/M3U8 lesen: Pfade (relativ zu `basis`) und Adressen, in der Reihenfolge der Datei.
pub fn m3u_lesen(text: &str, basis: &Path) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut v = Vec::new();
    for l in text.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with("file://") {
            if let Some(p) = file_url_zu_pfad(l) {
                v.push(p);
            }
        } else if ist_url(l) {
            v.push(l.to_string());
        } else {
            let p = Path::new(l);
            let voll = if p.has_root() || l.as_bytes().get(1) == Some(&b':') { p.to_path_buf() } else { basis.join(p) };
            v.push(voll.to_string_lossy().into_owned());
        }
        if v.len() >= 20_000 {
            break;
        }
    }
    v
}

// ------------------------------------------------------------------ Bildschirmfotos

/// Ordner «Bilder» des Benutzers. `home`: HOME (unter Windows USERPROFILE); `user_dirs`:
/// Inhalt von ~/.config/user-dirs.dirs (Linux, z. B. XDG_PICTURES_DIR="$HOME/Bilder");
/// `ist_ordner` prüft, ob es einen Ordner gibt. Ohne «Bilder» der Benutzerordner.
pub fn bilder_ordner(home: Option<&Path>, user_dirs: Option<&str>, ist_ordner: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let home = home?;
    if let Some(ud) = user_dirs {
        for l in ud.lines() {
            let Some(wert) = l.trim().strip_prefix("XDG_PICTURES_DIR=") else { continue };
            let wert = wert.trim().trim_matches('"');
            let p = if let Some(r) = wert.strip_prefix("$HOME/") {
                home.join(r)
            } else if wert == "$HOME" {
                home.to_path_buf()
            } else {
                PathBuf::from(wert)
            };
            if p.has_root() && ist_ordner(&p) {
                return Some(p);
            }
        }
    }
    for name in ["Pictures", "Bilder"] {
        let p = home.join(name);
        if ist_ordner(&p) {
            return Some(p);
        }
    }
    Some(home.to_path_buf())
}

/// «abspielwerk-2026-11-13-14h05m09s.png»
pub fn schnappschuss_name(j: i32, mo: u32, t: u32, h: u32, mi: u32, s: u32) -> String {
    format!("abspielwerk-{j:04}-{mo:02}-{t:02}-{h:02}h{mi:02}m{s:02}s.png")
}

/// `name` in `dir`, oder mit «-2», «-3» … davor, falls es die Datei schon gibt.
pub fn freier_pfad(dir: &Path, name: &str, gibt_es: &dyn Fn(&Path) -> bool) -> PathBuf {
    let p = dir.join(name);
    if !gibt_es(&p) {
        return p;
    }
    let (stamm, endung) = match name.rsplit_once('.') {
        Some((a, b)) => (a.to_string(), format!(".{b}")),
        None => (name.to_string(), String::new()),
    };
    for n in 2..1000 {
        let p = dir.join(format!("{stamm}-{n}{endung}"));
        if !gibt_es(&p) {
            return p;
        }
    }
    dir.join(format!("{stamm}-x{endung}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positionen() {
        let mut p = Positionen::default();
        p.merken("/a.mkv", 754_000, 3_600_000, 60_000);
        assert_eq!(p.holen("/a.mkv"), Some(754_000));
        // Anfang und Ende werden vergessen
        p.merken("/a.mkv", 2_000, 3_600_000, 60_000);
        assert_eq!(p.holen("/a.mkv"), None);
        p.merken("/a.mkv", 100_000, 3_600_000, 60_000);
        p.merken("/a.mkv", 3_595_000, 3_600_000, 60_000);
        assert_eq!(p.holen("/a.mkv"), None);
        p.merken("/a.mkv", 3_500_000, 3_600_000, 60_000); // > 95 %
        assert_eq!(p.holen("/a.mkv"), None);
        // kurze Stücke nicht
        p.merken("/kurz.mp4", 20_000, 50_000, 60_000);
        assert_eq!(p.holen("/kurz.mp4"), None);
        // ungültige Namen nicht
        p.merken("a\tb", 100_000, 3_600_000, 60_000);
        p.merken("", 100_000, 3_600_000, 60_000);
        assert_eq!(p.len(), 0);
        // Speichern und Laden
        p.merken("/x y.mkv", 61_000, 600_000, 60_000);
        p.merken("http://h/s", 70_000, 600_000, 60_000);
        let q = Positionen::from_text(&p.to_text());
        assert_eq!(p, q);
        assert_eq!(q.holen("/x y.mkv"), Some(61_000));
        // Begrenzung
        for i in 0..(MAX_POSITIONEN + 20) {
            p.merken(&format!("/f{i}"), 100_000, 600_000, 60_000);
        }
        assert_eq!(p.len(), MAX_POSITIONEN);
        assert_eq!(p.holen("/f0"), None);
        assert!(p.holen(&format!("/f{}", MAX_POSITIONEN + 19)).is_some());
        // Kaputter Text
        let q = Positionen::from_text("abc\n12\n-5\t/x\n7\t\n9\t/ok\n9\t/ok\n");
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn zuletzt() {
        let mut z = Zuletzt::default();
        for i in 0..15 {
            z.hinzufuegen(&format!("/f{i}"));
        }
        assert_eq!(z.v.len(), MAX_ZULETZT);
        assert_eq!(z.v[0], "/f14");
        z.hinzufuegen("/f10");
        assert_eq!(z.v[0], "/f10");
        assert_eq!(z.v.len(), MAX_ZULETZT);
        z.hinzufuegen("a\nb");
        assert_eq!(z.v[0], "/f10");
        assert_eq!(Zuletzt::from_text(&z.to_text()), z);
    }

    #[test]
    fn adressen() {
        assert!(ist_url("http://example.org/a.mp4"));
        assert!(ist_url("rtsp://h:554/x"));
        assert!(ist_url("file:///x"));
        assert!(!ist_url("C:\\Filme\\a.mkv"));
        assert!(!ist_url("/home/a://b"));
        assert!(!ist_url("http://"));
        assert_eq!(pfad_zu_url("/home/oli/a b/ä.srt"), "file:///home/oli/a%20b/%C3%A4.srt");
        assert_eq!(pfad_zu_url("C:\\Filme\\x#1.srt"), "file:///C:/Filme/x%231.srt");
        assert_eq!(pfad_zu_url("\\\\server\\f\\a.srt"), "file://server/f/a.srt");
        assert_eq!(file_url_zu_pfad("file:///home/oli/a%20b/%C3%A4.srt").as_deref(), Some("/home/oli/a b/ä.srt"));
        assert_eq!(file_url_zu_pfad("file:///C:/Filme/x%231.srt").as_deref(), Some("C:/Filme/x#1.srt"));
        assert_eq!(file_url_zu_pfad("file://server/f").as_deref(), Some("//server/f"));
        assert_eq!(file_url_zu_pfad("http://x"), None);
        assert_eq!(url_dekodieren("%zz%4"), "%zz%4");
        assert_eq!(url_dekodieren("%E2%82"), "\u{FFFD}");
    }

    #[test]
    fn listen() {
        let v = [
            ListenEintrag { ort: "/m/a.mkv", titel: "Film <1>", dauer_ms: Some(125_400) },
            ListenEintrag { ort: "http://h/s?a=1&b=2", titel: "", dauer_ms: None },
        ];
        let m = m3u_schreiben(&v);
        assert_eq!(m, "#EXTM3U\n#EXTINF:125,Film <1>\n/m/a.mkv\n#EXTINF:-1,\nhttp://h/s?a=1&b=2\n");
        let x = xspf_schreiben(&v, "Liste & Co");
        assert!(x.contains("<title>Liste &amp; Co</title>"));
        assert!(x.contains("<location>file:///m/a.mkv</location>"));
        assert!(x.contains("<title>Film &lt;1&gt;</title>"));
        assert!(x.contains("<duration>125400</duration>"));
        assert!(x.contains("<location>http://h/s?a=1&amp;b=2</location>"));
        // Lesen
        let gelesen = m3u_lesen(&m, Path::new("/m"));
        assert_eq!(gelesen, ["/m/a.mkv", "http://h/s?a=1&b=2"]);
        let r = m3u_lesen("\u{feff}#EXTM3U\r\nunter/b.mp3\r\n\r\nfile:///x%20y.mp3\r\n", Path::new("/basis"));
        assert_eq!(r[0], Path::new("/basis").join("unter/b.mp3").to_string_lossy());
        assert_eq!(r[1], "/x y.mp3");
        assert_eq!(xml_escape("a\u{1}b'"), "ab&apos;");
    }

    #[test]
    fn bildschirmfotos() {
        let home = Path::new("/home/oli");
        let alle = |_: &Path| true;
        let keine = |_: &Path| false;
        assert_eq!(bilder_ordner(Some(home), Some("# x\nXDG_PICTURES_DIR=\"$HOME/Bilder\"\n"), &alle), Some(home.join("Bilder")));
        assert_eq!(bilder_ordner(Some(home), None, &alle), Some(home.join("Pictures")));
        assert_eq!(bilder_ordner(Some(home), Some("XDG_PICTURES_DIR=\"$HOME/Bilder\""), &keine), Some(PathBuf::from("/home/oli")));
        assert_eq!(bilder_ordner(None, None, &alle), None);
        assert_eq!(schnappschuss_name(2026, 11, 3, 4, 5, 9), "abspielwerk-2026-11-03-04h05m09s.png");
        let da = |p: &Path| p.ends_with("a.png") || p.ends_with("a-2.png");
        assert_eq!(freier_pfad(Path::new("/b"), "a.png", &da), Path::new("/b").join("a-3.png"));
        assert_eq!(freier_pfad(Path::new("/b"), "c.png", &da), Path::new("/b").join("c.png"));
    }
}
