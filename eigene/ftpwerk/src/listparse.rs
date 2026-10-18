//! Liest die Verzeichnislisten von FTP-Servern: MLSD (genormt), Unix-«ls -l» und DOS/IIS.
//! Nur Standardbibliothek – wird mit `rustc --test` gegen echte Beispielzeilen geprüft.

use crate::model::{mode_from_perms, unix_from_parts, Entry};

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn month_num(s: &str) -> Option<u32> {
    let l = s.to_lowercase();
    MONTHS.iter().position(|m| *m == l).map(|i| i as u32 + 1)
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Zerlegt in Wörter und merkt sich, wo jedes Wort beginnt (damit der Dateiname mit seinen
/// Leerzeichen unverändert bleibt).
fn tokens(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                out.push((s, &line[s..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

/// Eine Zeile einer MLSD-Antwort: «type=file;size=12;modify=20261018134700; name».
pub fn parse_mlsd(line: &str) -> Option<Entry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (facts, name) = line.split_once(' ')?;
    if name.is_empty() {
        return None;
    }
    let mut e = Entry { name: name.to_string(), ..Default::default() };
    let mut kind = String::new();
    for f in facts.split(';') {
        let Some((k, v)) = f.split_once('=') else { continue };
        match k.to_lowercase().as_str() {
            "type" => kind = v.to_lowercase(),
            "size" => e.size = v.parse().unwrap_or(0),
            "sizd" => {
                if e.size == 0 {
                    e.size = v.parse().unwrap_or(0)
                }
            }
            "modify" => e.mtime = parse_mlsd_time(v),
            "unix.mode" => {
                if let Ok(m) = u32::from_str_radix(v.trim_start_matches('0'), 8) {
                    e.mode = Some(m & 0o7777);
                } else if v.chars().all(|c| c == '0') {
                    e.mode = Some(0);
                }
            }
            _ => {}
        }
    }
    match kind.as_str() {
        "dir" => e.is_dir = true,
        "file" => {}
        "cdir" | "pdir" => return None,
        k if k.contains("symlink") || k.contains("slink") => e.is_link = true,
        _ => {}
    }
    if e.name == "." || e.name == ".." {
        return None;
    }
    if let Some(m) = e.mode {
        e.perms = crate::model::perms_from_mode(m);
    }
    Some(e)
}

/// «20261018134700» oder «20261018134700.123» → Unix-Zeit.
pub fn parse_mlsd_time(v: &str) -> Option<i64> {
    let v = v.split('.').next()?;
    if v.len() < 8 || !all_digits(v) {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<u32> { v.get(a..b).and_then(|s| s.parse().ok()) };
    let y: i64 = v.get(0..4)?.parse().ok()?;
    let mo = num(4, 6)?;
    let d = num(6, 8)?;
    let h = num(8, 10).unwrap_or(0);
    let mi = num(10, 12).unwrap_or(0);
    let s = num(12, 14).unwrap_or(0);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(unix_from_parts(y, mo, d, h, mi, s))
}

/// Eine Zeile einer LIST-Antwort (Unix oder DOS). `now` = aktuelle Unix-Zeit (für Zeilen ohne Jahr).
pub fn parse_list(line: &str, now: i64) -> Option<Entry> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.trim().is_empty() {
        return None;
    }
    let lower = line.trim_start().to_lowercase();
    if lower.starts_with("total ") || lower.starts_with("gesamt ") {
        return None;
    }
    let first = line.trim_start().chars().next()?;
    if first.is_ascii_digit() {
        return parse_dos(line);
    }
    parse_unix(line, now)
}

fn parse_unix(line: &str, now: i64) -> Option<Entry> {
    let tk = tokens(line);
    if tk.len() < 6 {
        return None;
    }
    let perms = tk[0].1;
    let kind = perms.chars().next()?;
    if !matches!(kind, 'd' | 'l' | '-' | 'b' | 'c' | 'p' | 's' | 'D') {
        return None;
    }
    // Monatsname suchen: davor steht die Grösse (nur Ziffern), danach der Tag.
    let mut idx = None;
    for i in 3..tk.len().saturating_sub(2) {
        if month_num(tk[i].1).is_some() && all_digits(tk[i - 1].1) && all_digits(tk[i + 1].1) {
            idx = Some(i);
            break;
        }
    }
    let i = idx?;
    let size: u64 = tk[i - 1].1.parse().ok()?;
    let month = month_num(tk[i].1)?;
    let day: u32 = tk[i + 1].1.parse().ok()?;
    let ty = tk.get(i + 2)?;
    let name_start = ty.0 + ty.1.len();
    let rest = line[name_start..].trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return None;
    }
    let (h, mi, year) = if let Some((hh, mm)) = ty.1.split_once(':') {
        let (hh, mm): (u32, u32) = (hh.parse().ok()?, mm.parse().ok()?);
        // ohne Jahr: das laufende Jahr, ausser das Datum läge mehr als einen Tag in der Zukunft
        let (cy, _, _) = crate::model::civil_from_days(now.div_euclid(86_400));
        let t = unix_from_parts(cy, month, day.max(1), hh, mm, 0);
        let y = if t > now + 86_400 { cy - 1 } else { cy };
        (hh, mm, y)
    } else {
        (0, 0, ty.1.parse::<i64>().ok()?)
    };
    let mtime = if (1..=31).contains(&day) { Some(unix_from_parts(year, month, day, h, mi, 0)) } else { None };

    let mut e = Entry { size, mtime, ..Default::default() };
    let mut name = rest.to_string();
    match kind {
        'd' | 'D' => e.is_dir = true,
        'l' => {
            e.is_link = true;
            if let Some(p) = name.find(" -> ") {
                name.truncate(p);
            }
        }
        _ => {}
    }
    if name == "." || name == ".." {
        return None;
    }
    e.name = name;
    e.perms = perms.chars().skip(1).take(9).collect();
    e.mode = mode_from_perms(&e.perms);
    Some(e)
}

fn parse_dos(line: &str) -> Option<Entry> {
    let tk = tokens(line);
    if tk.len() < 4 {
        return None;
    }
    // Datum: MM-DD-YY oder MM-DD-YYYY
    let dp: Vec<&str> = tk[0].1.split(['-', '/']).collect();
    if dp.len() != 3 {
        return None;
    }
    let mo: u32 = dp[0].parse().ok()?;
    let d: u32 = dp[1].parse().ok()?;
    let mut y: i64 = dp[2].parse().ok()?;
    if dp[2].len() <= 2 {
        y += if y < 70 { 2000 } else { 1900 };
    }
    // Zeit: HH:MM, mit AM/PM angehängt oder als eigenes Wort
    let ts = tk[1].1.to_uppercase();
    let pm = ts.ends_with("PM");
    let am = ts.ends_with("AM");
    let core = ts.trim_end_matches("PM").trim_end_matches("AM");
    let (hh, mm) = core.split_once(':')?;
    let mut h: u32 = hh.parse().ok()?;
    let mi: u32 = mm.parse().ok()?;
    if pm && h < 12 {
        h += 12;
    }
    if am && h == 12 {
        h = 0;
    }
    let mut next = 2;
    let mut ampm_word = tk.get(2).map(|t| t.1.to_uppercase());
    if let Some(w) = ampm_word.take() {
        if w == "AM" || w == "PM" {
            if w == "PM" && h < 12 {
                h += 12;
            }
            if w == "AM" && h == 12 {
                h = 0;
            }
            next = 3;
        }
    }
    let kind = tk.get(next)?;
    let name_tok = tk.get(next + 1)?;
    let name = line[name_tok.0..].trim_end().to_string();
    let mut e = Entry { name, mtime: Some(unix_from_parts(y, mo, d.max(1), h, mi, 0)), ..Default::default() };
    if kind.1.eq_ignore_ascii_case("<DIR>") {
        e.is_dir = true;
    } else {
        e.size = kind.1.replace(',', "").parse().ok()?;
    }
    if e.name == "." || e.name == ".." {
        return None;
    }
    Some(e)
}

/// Ganze Antwort (mehrere Zeilen) lesen. `mlsd` wählt das Format.
pub fn parse_listing(text: &str, mlsd: bool, now: i64) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| if mlsd { parse_mlsd(l) } else { parse_list(l, now) })
        .filter(|e| !e.name.is_empty() && e.name != "." && e.name != "..")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-18 12:00 UTC
    fn now() -> i64 {
        unix_from_parts(2026, 10, 18, 12, 0, 0)
    }

    #[test]
    fn unix_normal() {
        let e = parse_list("drwxr-xr-x    2 ftp      ftp          4096 Oct 17 13:47 Fotos", now()).unwrap();
        assert!(e.is_dir);
        assert_eq!(e.name, "Fotos");
        assert_eq!(e.size, 4096);
        assert_eq!(e.perms, "rwxr-xr-x");
        assert_eq!(e.mode, Some(0o755));
        assert_eq!(e.mtime, Some(unix_from_parts(2026, 10, 17, 13, 47, 0)));
    }

    #[test]
    fn unix_jahr_statt_zeit() {
        let e = parse_list("-rw-r--r--    1 user group  1234567 Jan  5  2023 Mein Bericht (final).pdf", now()).unwrap();
        assert!(!e.is_dir);
        assert_eq!(e.name, "Mein Bericht (final).pdf");
        assert_eq!(e.size, 1_234_567);
        assert_eq!(e.mtime, Some(unix_from_parts(2023, 1, 5, 0, 0, 0)));
    }

    #[test]
    fn unix_zukunft_ist_vorjahr() {
        // «Dec 24 10:00» im Oktober 2026 kann nur Dezember 2025 sein.
        let e = parse_list("-rw-r--r-- 1 u g 10 Dec 24 10:00 a.txt", now()).unwrap();
        assert_eq!(e.mtime, Some(unix_from_parts(2025, 12, 24, 10, 0, 0)));
    }

    #[test]
    fn unix_symlink() {
        let e = parse_list("lrwxrwxrwx 1 root root 11 Mar 3 09:15 www -> /var/www/html", now()).unwrap();
        assert!(e.is_link);
        assert_eq!(e.name, "www");
    }

    #[test]
    fn unix_ohne_gruppe() {
        let e = parse_list("-rw-r--r-- 1 owner 4711 Feb 2 2020 notiz.txt", now()).unwrap();
        assert_eq!(e.size, 4711);
        assert_eq!(e.name, "notiz.txt");
    }

    #[test]
    fn unix_name_mit_mehreren_leerzeichen() {
        let e = parse_list("-rw-r--r-- 1 u g 5 Oct 1 2025 a  b   c.txt", now()).unwrap();
        assert_eq!(e.name, "a  b   c.txt");
    }

    #[test]
    fn unix_ueberspringt_total_und_punkte() {
        assert!(parse_list("total 48", now()).is_none());
        assert!(parse_list("drwxr-xr-x 2 u g 4096 Oct 1 12:00 .", now()).is_none());
        assert!(parse_list("drwxr-xr-x 2 u g 4096 Oct 1 12:00 ..", now()).is_none());
        assert!(parse_list("", now()).is_none());
    }

    #[test]
    fn dos_iis() {
        let e = parse_list("10-18-26  01:47PM       <DIR>          Bilder", now()).unwrap();
        assert!(e.is_dir);
        assert_eq!(e.name, "Bilder");
        assert_eq!(e.mtime, Some(unix_from_parts(2026, 10, 18, 13, 47, 0)));
        let f = parse_list("02-29-2024  12:05AM              1,234 Mit Leerzeichen.txt", now()).unwrap();
        assert_eq!(f.size, 1234);
        assert_eq!(f.name, "Mit Leerzeichen.txt");
        assert_eq!(f.mtime, Some(unix_from_parts(2024, 2, 29, 0, 5, 0)));
    }

    #[test]
    fn mlsd_zeilen() {
        let e = parse_mlsd("type=file;size=1024;modify=20261018134700;perm=adfrw; bericht 2026.pdf").unwrap();
        assert_eq!(e.name, "bericht 2026.pdf");
        assert_eq!(e.size, 1024);
        assert_eq!(e.mtime, Some(unix_from_parts(2026, 10, 18, 13, 47, 0)));
        let d = parse_mlsd("type=dir;modify=20260101000000;UNIX.mode=0755; Ordner").unwrap();
        assert!(d.is_dir);
        assert_eq!(d.mode, Some(0o755));
        assert_eq!(d.perms, "rwxr-xr-x");
        assert!(parse_mlsd("type=cdir;modify=20260101000000; .").is_none());
        assert!(parse_mlsd("type=pdir;modify=20260101000000; ..").is_none());
    }

    #[test]
    fn mlsd_zeit_mit_bruchteil() {
        assert_eq!(parse_mlsd_time("20261018134700.250"), Some(unix_from_parts(2026, 10, 18, 13, 47, 0)));
        assert_eq!(parse_mlsd_time("2026"), None);
        assert_eq!(parse_mlsd_time("20261318134700"), None);
    }

    #[test]
    fn ganze_antwort() {
        let txt = "total 3\r\ndrwxr-xr-x 2 u g 4096 Oct 1 12:00 .\r\ndrwxr-xr-x 2 u g 4096 Oct 1 12:00 a\r\n-rw-r--r-- 1 u g 7 Oct 1 12:00 b.txt\r\n";
        let v = parse_listing(txt, false, now());
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "a");
        assert_eq!(v[1].size, 7);
    }
}
