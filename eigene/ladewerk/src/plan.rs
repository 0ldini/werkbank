//! Reine Rechen- und Planungslogik (ohne egui, nur std): Grössen, Geschwindigkeit,
//! Restzeit, Geschwindigkeitsbegrenzung, HTTP-Antworten einordnen, Wartezeit beim
//! Wiederholen, Archive erkennen (auch mehrteilige) und das Entpacken planen.
//!
//! Prüfung: `rustc --edition 2021 --test src/plan.rs -o /tmp/plan && /tmp/plan`

#![allow(dead_code)]

/// «1.5 MB», «820 KB», «12 B» (Basis 1024, wie im Dateimanager üblich).
pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v < 10.0 {
        format!("{v:.2} {}", UNITS[u])
    } else if v < 100.0 {
        format!("{v:.1} {}", UNITS[u])
    } else {
        format!("{v:.0} {}", UNITS[u])
    }
}

/// Geschwindigkeit in Bytes pro Sekunde → «2.40 MB/s».
pub fn format_speed(bps: f64) -> String {
    if !bps.is_finite() || bps <= 0.0 {
        return "0 B/s".to_string();
    }
    format!("{}/s", format_bytes(bps.min(1e18) as u64))
}

/// Sekunden → «0:42», «12:05», «1:02:03».
pub fn format_eta(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Restzeit in Sekunden, wenn sie sich sinnvoll schätzen lässt.
pub fn eta(remaining: u64, bps: f64) -> Option<u64> {
    if !bps.is_finite() || bps < 1.0 {
        return None;
    }
    let s = remaining as f64 / bps;
    (s.is_finite() && s < 360_000_000.0).then_some(s.ceil() as u64)
}

/// Gleitender Mittelwert für eine ruhige Geschwindigkeitsanzeige.
pub fn smooth(prev: f64, sample: f64, dt: f64) -> f64 {
    if !sample.is_finite() || sample < 0.0 {
        return prev;
    }
    if !prev.is_finite() || prev <= 0.0 {
        return sample;
    }
    // Zeitkonstante etwa 2 Sekunden.
    let a = (dt / 2.0).clamp(0.0, 1.0);
    prev + (sample - prev) * a
}

/// Anteil 0..1 (0 bei unbekannter Grösse).
pub fn fraction(done: u64, total: Option<u64>) -> f32 {
    match total {
        Some(t) if t > 0 => (done as f64 / t as f64).clamp(0.0, 1.0) as f32,
        _ => 0.0,
    }
}

/// Token-Eimer für die gemeinsame Geschwindigkeitsbegrenzung aller Downloads.
/// Zeit in Sekunden ab beliebigem Nullpunkt, Rate in Bytes pro Sekunde.
#[derive(Clone, Debug, Default)]
pub struct Bucket {
    tokens: f64,
    last: f64,
}

impl Bucket {
    /// Bucht `n` Bytes und liefert, wie lange der Aufrufer danach warten muss (Sekunden).
    /// Rate 0 = unbegrenzt.
    pub fn take(&mut self, now: f64, rate: f64, n: u64) -> f64 {
        if !(rate.is_finite() && rate > 0.0) || !now.is_finite() {
            self.tokens = 0.0;
            self.last = now;
            return 0.0;
        }
        // Höchstens eine halbe Sekunde Vorrat, damit nach Pausen kein grosser Schub kommt.
        let burst = (rate * 0.5).max(16_384.0);
        let dt = (now - self.last).max(0.0);
        self.last = now;
        self.tokens = (self.tokens + dt * rate).min(burst);
        self.tokens -= n as f64;
        if self.tokens >= 0.0 {
            0.0
        } else {
            (-self.tokens / rate).min(60.0)
        }
    }
}

/// «bytes 100-199/1000» → (Anfang, Ende, Gesamtgrösse falls bekannt).
/// «bytes */1000» (bei 416) → (0, 0, Some(1000)) mit Anfang = Ende = 0.
pub fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    if range.trim() == "*" {
        return Some((0, 0, total));
    }
    let (a, b) = range.trim().split_once('-')?;
    let (a, b): (u64, u64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a <= b).then_some((a, b, total))
}

/// Wie eine HTTP-Antwort zu behandeln ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpClass {
    /// 200: ganze Datei.
    Full,
    /// 206: Teil ab der gewünschten Stelle.
    Partial,
    /// 416: Bereich passt nicht (oft: Datei schon vollständig).
    RangeNotSatisfiable,
    /// 301/302/303/307/308.
    Redirect,
    /// Später nochmals versuchen (408, 425, 429, 5xx).
    Retry,
    /// Endgültiger Fehler (404, 403 …).
    Fatal,
}

pub fn classify(code: u16) -> HttpClass {
    match code {
        200 | 203 => HttpClass::Full,
        206 => HttpClass::Partial,
        416 => HttpClass::RangeNotSatisfiable,
        301 | 302 | 303 | 307 | 308 => HttpClass::Redirect,
        408 | 425 | 429 | 500..=599 => HttpClass::Retry,
        _ => HttpClass::Fatal,
    }
}

/// Wartezeit vor Versuch Nr. `attempt` (1, 2, 3 …): Grundzeit, verdoppelt, höchstens 10 Minuten.
pub fn retry_delay(base_secs: u32, attempt: u32) -> u64 {
    let base = u64::from(base_secs.max(1));
    let shift = attempt.saturating_sub(1).min(16);
    base.saturating_mul(1u64 << shift).min(600)
}

/// Archivarten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArcKind {
    Zip,
    /// ZIP in mehreren Teilen (.z01 … oder .zip.001): braucht 7-Zip.
    SplitZip,
    SevenZ,
    Rar,
}

impl ArcKind {
    /// Lässt sich ohne Zusatzprogramm entpacken?
    pub fn builtin(self) -> bool {
        self == ArcKind::Zip
    }
    fn family(self) -> u8 {
        match self {
            ArcKind::Zip | ArcKind::SplitZip => 0,
            ArcKind::SevenZ => 1,
            ArcKind::Rar => 2,
        }
    }
}

/// Ein Dateiname als Teil eines Archivs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArcPart {
    /// Name ohne Teil- und Archivendung, z. B. «Film» für «Film.part01.rar».
    pub stem: String,
    pub kind: ArcKind,
    /// Mit diesem Teil wird das Entpacken gestartet.
    pub first: bool,
}

fn digits(s: &str) -> Option<u32> {
    (!s.is_empty() && s.len() <= 4 && s.chars().all(|c| c.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// Erkennt Archive und Archivteile am Namen.
pub fn archive_part(name: &str) -> Option<ArcPart> {
    let lower = name.to_ascii_lowercase();
    let mk = |stem_len: usize, kind, first| ArcPart { stem: name.get(..stem_len).unwrap_or(name).to_string(), kind, first };
    let (base, ext) = lower.rsplit_once('.')?;
    // «x.7z.001», «x.zip.001»
    if let Some(n) = digits(ext).filter(|_| ext.len() == 3) {
        if let Some(b) = base.strip_suffix(".7z") {
            return Some(mk(b.len(), ArcKind::SevenZ, n == 1));
        }
        if let Some(b) = base.strip_suffix(".zip") {
            return Some(mk(b.len(), ArcKind::SplitZip, n == 1));
        }
        return None;
    }
    match ext {
        "zip" => Some(mk(base.len(), ArcKind::Zip, true)),
        "7z" => Some(mk(base.len(), ArcKind::SevenZ, true)),
        "rar" => {
            // «x.part01.rar»
            if let Some((b, part)) = base.rsplit_once('.') {
                if let Some(n) = part.strip_prefix("part").and_then(digits) {
                    return Some(mk(b.len(), ArcKind::Rar, n == 1));
                }
            }
            Some(mk(base.len(), ArcKind::Rar, true))
        }
        _ => {
            // «x.r00» (alte RAR-Teile) und «x.z01» (geteiltes ZIP)
            let num = ext.get(1..).and_then(digits).filter(|_| (3..=4).contains(&ext.len()));
            match (ext.as_bytes().first(), num) {
                (Some(b'r'), Some(_)) => Some(mk(base.len(), ArcKind::Rar, false)),
                (Some(b'z'), Some(_)) => Some(mk(base.len(), ArcKind::SplitZip, false)),
                _ => None,
            }
        }
    }
}

/// Was nach dem Download eines Pakets zu entpacken ist: (erster Teil, Art, Stamm).
/// Ein «.zip» mit «.z01»-Geschwistern wird als geteiltes ZIP behandelt.
pub fn archives_to_extract(names: &[String]) -> Vec<(String, ArcKind, String)> {
    let parts: Vec<(&String, ArcPart)> = names.iter().filter_map(|n| archive_part(n).map(|p| (n, p))).collect();
    let mut out = Vec::new();
    for (name, p) in &parts {
        if !p.first {
            continue;
        }
        let mut kind = p.kind;
        if kind == ArcKind::Zip && parts.iter().any(|(_, q)| q.kind == ArcKind::SplitZip && q.stem.eq_ignore_ascii_case(&p.stem)) {
            kind = ArcKind::SplitZip;
        }
        if !out.iter().any(|(n, _, _): &(String, ArcKind, String)| n == *name) {
            out.push(((*name).clone(), kind, p.stem.clone()));
        }
    }
    out
}

/// Alle Dateien, die zum Archiv mit diesem Stamm gehören (zum Löschen nach dem Entpacken).
pub fn parts_of(stem: &str, kind: ArcKind, names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| archive_part(n).is_some_and(|p| p.stem.eq_ignore_ascii_case(stem) && p.kind.family() == kind.family()))
        .cloned()
        .collect()
}

/// Paketname aus dem ersten Dateinamen: ohne Archiv-/Teilendung bzw. ohne Endung.
pub fn package_name_from(file_name: &str) -> String {
    if let Some(p) = archive_part(file_name) {
        return p.stem;
    }
    match file_name.rfind('.') {
        Some(p) if p > 0 => file_name[..p].to_string(),
        _ => file_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groessen() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(50 * 1024 * 1024), "50.0 MB");
        assert_eq!(format_bytes(700 * 1024 * 1024 * 1024), "700 GB");
        assert_eq!(format_bytes(u64::MAX), "16777216 TB");
        assert_eq!(format_speed(f64::NAN), "0 B/s");
        assert_eq!(format_speed(2048.0), "2.00 KB/s");
    }

    #[test]
    fn restzeit() {
        assert_eq!(format_eta(42), "0:42");
        assert_eq!(format_eta(725), "12:05");
        assert_eq!(format_eta(3723), "1:02:03");
        assert_eq!(eta(1000, 100.0), Some(10));
        assert_eq!(eta(1000, 0.0), None);
        assert_eq!(eta(u64::MAX, 1.0), None);
        assert_eq!(fraction(5, Some(10)), 0.5);
        assert_eq!(fraction(5, None), 0.0);
        assert_eq!(fraction(5, Some(0)), 0.0);
    }

    #[test]
    fn glaetten() {
        assert_eq!(smooth(0.0, 100.0, 0.2), 100.0);
        let v = smooth(100.0, 200.0, 0.2);
        assert!(v > 100.0 && v < 200.0);
        assert_eq!(smooth(50.0, f64::NAN, 0.2), 50.0);
    }

    #[test]
    fn begrenzung() {
        let mut b = Bucket::default();
        assert_eq!(b.take(0.0, 0.0, 1_000_000), 0.0); // unbegrenzt
        // 100 KB/s: ein Block von 64 KiB am Anfang muss warten (Vorrat ist leer).
        let mut b = Bucket::default();
        let w = b.take(0.0, 100_000.0, 65_536);
        assert!((w - 0.65536).abs() < 1e-9);
        // Nach der Wartezeit ist die Schuld beglichen.
        assert_eq!(b.take(w, 100_000.0, 0), 0.0);
        // Langer Leerlauf füllt nur bis zum Vorrat (0.5 s).
        let mut b = Bucket::default();
        b.take(0.0, 100_000.0, 0);
        assert_eq!(b.take(100.0, 100_000.0, 50_000), 0.0);
        assert!(b.take(100.0, 100_000.0, 50_000) > 0.0);
    }

    #[test]
    fn bereiche() {
        assert_eq!(parse_content_range("bytes 100-199/1000"), Some((100, 199, Some(1000))));
        assert_eq!(parse_content_range("bytes 0-9/*"), Some((0, 9, None)));
        assert_eq!(parse_content_range("bytes */1000"), Some((0, 0, Some(1000))));
        assert_eq!(parse_content_range("bytes 9-1/10"), None);
        assert_eq!(parse_content_range("items 1-2/3"), None);
        assert_eq!(parse_content_range("bytes x-y/z"), None);
    }

    #[test]
    fn antworten() {
        assert_eq!(classify(200), HttpClass::Full);
        assert_eq!(classify(206), HttpClass::Partial);
        assert_eq!(classify(302), HttpClass::Redirect);
        assert_eq!(classify(503), HttpClass::Retry);
        assert_eq!(classify(429), HttpClass::Retry);
        assert_eq!(classify(404), HttpClass::Fatal);
        assert_eq!(classify(304), HttpClass::Fatal);
    }

    #[test]
    fn wartezeit() {
        assert_eq!(retry_delay(5, 1), 5);
        assert_eq!(retry_delay(5, 3), 20);
        assert_eq!(retry_delay(5, 100), 600);
        assert_eq!(retry_delay(0, 1), 1);
    }

    #[test]
    fn archive() {
        let p = |n: &str| archive_part(n).map(|p| (p.stem, p.kind, p.first));
        assert_eq!(p("Film.part01.rar"), Some(("Film".into(), ArcKind::Rar, true)));
        assert_eq!(p("Film.part2.rar"), Some(("Film".into(), ArcKind::Rar, false)));
        assert_eq!(p("Alt.rar"), Some(("Alt".into(), ArcKind::Rar, true)));
        assert_eq!(p("Alt.r00"), Some(("Alt".into(), ArcKind::Rar, false)));
        assert_eq!(p("Daten.7z"), Some(("Daten".into(), ArcKind::SevenZ, true)));
        assert_eq!(p("Daten.7Z.001"), Some(("Daten".into(), ArcKind::SevenZ, true)));
        assert_eq!(p("Daten.7z.002"), Some(("Daten".into(), ArcKind::SevenZ, false)));
        assert_eq!(p("a.zip"), Some(("a".into(), ArcKind::Zip, true)));
        assert_eq!(p("a.z01"), Some(("a".into(), ArcKind::SplitZip, false)));
        assert_eq!(p("a.zip.001"), Some(("a".into(), ArcKind::SplitZip, true)));
        assert_eq!(p("bild.png"), None);
        assert_eq!(p("x.001"), None);
        assert_eq!(p("rar"), None);
        assert_eq!(p("ä.part1.rar"), Some(("ä".into(), ArcKind::Rar, true)));
    }

    #[test]
    fn entpackplan() {
        let names: Vec<String> = ["F.part1.rar", "F.part2.rar", "a.zip", "a.z01", "b.zip", "c.7z.001", "c.7z.002", "readme.txt"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let plan = archives_to_extract(&names);
        assert_eq!(
            plan,
            vec![
                ("F.part1.rar".to_string(), ArcKind::Rar, "F".to_string()),
                ("a.zip".to_string(), ArcKind::SplitZip, "a".to_string()),
                ("b.zip".to_string(), ArcKind::Zip, "b".to_string()),
                ("c.7z.001".to_string(), ArcKind::SevenZ, "c".to_string()),
            ]
        );
        assert_eq!(parts_of("F", ArcKind::Rar, &names), vec!["F.part1.rar", "F.part2.rar"]);
        assert_eq!(parts_of("a", ArcKind::SplitZip, &names), vec!["a.zip", "a.z01"]);
    }

    #[test]
    fn paketname() {
        assert_eq!(package_name_from("Urlaub.part01.rar"), "Urlaub");
        assert_eq!(package_name_from("buch.pdf"), "buch");
        assert_eq!(package_name_from(".bashrc"), ".bashrc");
    }
}
