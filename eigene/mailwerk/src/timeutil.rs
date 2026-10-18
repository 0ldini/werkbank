//! Datum und Zeit ohne Abhängigkeiten: RFC-2822-Datum lesen, Unix-Zeit in
//! Kalenderdatum umrechnen, Ortszeit (Europa-Sommerzeit nativ, Browser-Zone im Web).

/// Tage seit 1970-01-01 für ein Kalenderdatum (proleptisch gregorianisch).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Kalenderdatum (Jahr, Monat, Tag) aus Tagen seit 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Wochentag: 0 = Sonntag … 6 = Samstag.
pub fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn zone_offset(z: &str) -> Option<i64> {
    let z = z.trim();
    if let Some(sign) = z.chars().next().filter(|c| *c == '+' || *c == '-') {
        let digits = &z[1..];
        if digits.len() >= 4 && digits.as_bytes()[..4].iter().all(|b| b.is_ascii_digit()) {
            let h: i64 = digits[0..2].parse().ok()?;
            let m: i64 = digits[2..4].parse().ok()?;
            let secs = h * 3600 + m * 60;
            return Some(if sign == '-' { -secs } else { secs });
        }
        return None;
    }
    match z.to_ascii_uppercase().as_str() {
        "GMT" | "UT" | "UTC" | "Z" => Some(0),
        "EST" => Some(-5 * 3600),
        "EDT" => Some(-4 * 3600),
        "CST" => Some(-6 * 3600),
        "CDT" => Some(-5 * 3600),
        "MST" => Some(-7 * 3600),
        "MDT" => Some(-6 * 3600),
        "PST" => Some(-8 * 3600),
        "PDT" => Some(-7 * 3600),
        "CET" => Some(3600),
        "CEST" => Some(2 * 3600),
        _ => None,
    }
}

/// Liest ein Datum nach RFC 2822, z. B. «Wed, 14 Oct 2026 08:42:13 +0200»
/// (Wochentag und Sekunden optional, Kommentare in Klammern werden ignoriert).
/// Ergebnis: Unix-Sekunden (UTC).
pub fn parse_rfc2822(s: &str) -> Option<i64> {
    // Kommentar « (CEST)» entfernen
    let mut clean = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => clean.push(c),
            _ => {}
        }
    }
    let mut parts: Vec<&str> = clean.split_whitespace().collect();
    if parts.first().map(|p| p.ends_with(',')).unwrap_or(false) {
        parts.remove(0);
    } else if parts.first().map(|p| p.len() == 3 && p.chars().all(|c| c.is_ascii_alphabetic())).unwrap_or(false)
        && parts.len() > 4
    {
        parts.remove(0); // «Tue 14 Oct …» ohne Komma
    }
    if parts.len() < 4 {
        return None;
    }
    let day: i64 = parts[0].parse().ok()?;
    let mon_name = parts[1].to_ascii_lowercase();
    let month = MONTHS.iter().position(|m| mon_name.starts_with(m))? as i64 + 1;
    let mut year: i64 = parts[2].parse().ok()?;
    if year < 50 {
        year += 2000;
    } else if year < 100 {
        year += 1900;
    }
    let tp: Vec<&str> = parts[3].split(':').collect();
    if tp.len() < 2 {
        return None;
    }
    let hh: i64 = tp[0].parse().ok()?;
    let mm: i64 = tp[1].parse().ok()?;
    let ss: i64 = tp.get(2).and_then(|x| x.parse().ok()).unwrap_or(0);
    let off = parts.get(4).and_then(|z| zone_offset(z)).unwrap_or(0);
    if !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86400 + hh * 3600 + mm * 60 + ss - off)
}

/// Liest «2026-10-14T08:42:13Z» bzw. mit Zone «+02:00» (JMAP, ISO 8601).
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    let h: i64 = s.get(11..13)?.parse().ok()?;
    let mi: i64 = s.get(14..16)?.parse().ok()?;
    let se: i64 = s.get(17..19)?.parse().ok()?;
    let mut rest = &s[19..];
    if rest.starts_with('.') {
        rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    }
    let off = if rest.is_empty() || rest == "Z" || rest == "z" {
        0
    } else {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 4 {
            return None;
        }
        let oh: i64 = digits[0..2].parse().ok()?;
        let om: i64 = digits[2..4].parse().ok()?;
        sign * (oh * 3600 + om * 60)
    };
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se - off)
}

/// Liest das IMAP-Datum «14-Oct-2026 08:42:13 +0200».
pub fn parse_imap_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let mut it = s.split_whitespace();
    let dmy = it.next()?;
    let time = it.next().unwrap_or("00:00:00");
    let zone = it.next().unwrap_or("+0000");
    let mut p = dmy.split('-');
    let d = p.next()?;
    let m = p.next()?;
    let y = p.next()?;
    parse_rfc2822(&format!("{d} {m} {y} {time} {zone}"))
}

/// Formatiert Unix-Zeit nach RFC 2822 in UTC («Wed, 14 Oct 2026 06:42:13 +0000»).
pub fn format_rfc2822(ts: i64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = ts.div_euclid(86400);
    let rem = ts.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        DAYS[weekday(days) as usize],
        d,
        MON[(m - 1) as usize],
        y,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// ---------------------------------------------------------------- Ortszeit

/// Mitteleuropäische Zeit mit EU-Sommerzeit (letzter Sonntag im März 01:00 UTC
/// bis letzter Sonntag im Oktober 01:00 UTC): +1 h bzw. +2 h.
pub fn eu_central_offset(ts: i64) -> i64 {
    let year = civil_from_days(ts.div_euclid(86400)).0;
    let last_sunday = |month: i64| -> i64 {
        // letzter Tag des Monats, dann zurück bis Sonntag
        let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
        let last = days_from_civil(ny, nm, 1) - 1;
        last - weekday(last)
    };
    let start = last_sunday(3) * 86400 + 3600;
    let end = last_sunday(10) * 86400 + 3600;
    if ts >= start && ts < end {
        7200
    } else {
        3600
    }
}

/// Versatz der Ortszeit zu UTC in Sekunden für den Zeitpunkt `ts`.
#[cfg(not(target_arch = "wasm32"))]
pub fn local_offset(ts: i64) -> i64 {
    eu_central_offset(ts)
}

#[cfg(target_arch = "wasm32")]
pub fn local_offset(ts: i64) -> i64 {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0));
    // getTimezoneOffset: Minuten, positiv westlich von UTC
    (-d.get_timezone_offset() * 60.0) as i64
}

#[cfg(not(target_arch = "wasm32"))]
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
pub fn now_ts() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// Lokaler Kalendertag (Tage seit 1970) zu einem Zeitpunkt.
pub fn local_day(ts: i64) -> i64 {
    (ts + local_offset(ts)).div_euclid(86400)
}

/// Gruppenname für die Liste: «Heute», «Gestern», «Diese Woche» oder «Älter».
pub fn group_label(ts: i64, now: i64) -> &'static str {
    let diff = local_day(now) - local_day(ts);
    match diff {
        i64::MIN..=0 => "Heute",
        1 => "Gestern",
        2..=6 => "Diese Woche",
        _ => "Älter",
    }
}

/// Kurze Zeit für die Liste: «08:42» heute, sonst «14.10.».
pub fn short_label(ts: i64, now: i64) -> String {
    let loc = ts + local_offset(ts);
    let diff = local_day(now) - local_day(ts);
    if diff <= 0 {
        let rem = loc.rem_euclid(86400);
        format!("{:02}:{:02}", rem / 3600, (rem % 3600) / 60)
    } else if diff < 7 {
        const WT: [&str; 7] = ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"];
        WT[weekday(loc.div_euclid(86400)) as usize].to_string()
    } else {
        let (y, m, d) = civil_from_days(loc.div_euclid(86400));
        let (ny, _, _) = civil_from_days(local_day(now));
        if y == ny {
            format!("{d}.{m}.")
        } else {
            format!("{d}.{m}.{}", y % 100)
        }
    }
}

/// Langes Datum für die Leseansicht: «Heute, 08:42», «Gestern, 17:20» oder «14.10.2026, 08:42».
pub fn long_label(ts: i64, now: i64) -> String {
    let loc = ts + local_offset(ts);
    let rem = loc.rem_euclid(86400);
    let hm = format!("{:02}:{:02}", rem / 3600, (rem % 3600) / 60);
    match local_day(now) - local_day(ts) {
        i64::MIN..=0 => format!("Heute, {hm}"),
        1 => format!("Gestern, {hm}"),
        _ => {
            let (y, m, d) = civil_from_days(loc.div_euclid(86400));
            format!("{d}.{m}.{y}, {hm}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tage() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(civil_from_days(11017), (2000, 3, 1));
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 18)), (2026, 10, 18));
        assert_eq!(weekday(days_from_civil(2026, 10, 18)), 0); // Sonntag
        assert_eq!(weekday(days_from_civil(1970, 1, 1)), 4); // Donnerstag
    }

    #[test]
    fn rfc2822() {
        let t = parse_rfc2822("Wed, 14 Oct 2026 08:42:13 +0200").unwrap();
        assert_eq!(format_rfc2822(t), "Wed, 14 Oct 2026 06:42:13 +0000");
        assert_eq!(parse_rfc2822("14 Oct 2026 06:42 GMT"), Some(t - 13));
        assert_eq!(parse_rfc2822("Wed, 14 Oct 2026 08:42:13 +0200 (CEST)"), Some(t));
        assert_eq!(parse_rfc2822("Mon, 1 Jan 24 00:00:00 -0500"), Some(days_from_civil(2024, 1, 1) * 86400 + 5 * 3600));
        assert_eq!(parse_rfc2822("Unsinn"), None);
    }

    #[test]
    fn iso_und_imap() {
        let a = parse_iso8601("2026-10-14T06:42:13Z").unwrap();
        assert_eq!(parse_iso8601("2026-10-14T08:42:13+02:00"), Some(a));
        assert_eq!(parse_iso8601("2026-10-14T06:42:13.123Z"), Some(a));
        assert_eq!(parse_imap_date("14-Oct-2026 08:42:13 +0200"), Some(a));
    }

    #[test]
    fn sommerzeit_ostschweiz() {
        let winter = days_from_civil(2026, 1, 15) * 86400 + 12 * 3600;
        let sommer = days_from_civil(2026, 7, 15) * 86400 + 12 * 3600;
        assert_eq!(eu_central_offset(winter), 3600);
        assert_eq!(eu_central_offset(sommer), 7200);
        // Umstellung 2026: 29. März und 25. Oktober, je 01:00 UTC
        let vor = days_from_civil(2026, 3, 29) * 86400 + 3599;
        assert_eq!(eu_central_offset(vor), 3600);
        assert_eq!(eu_central_offset(vor + 1), 7200);
        let okt_vor = days_from_civil(2026, 10, 25) * 86400 + 3599;
        assert_eq!(eu_central_offset(okt_vor), 7200);
        assert_eq!(eu_central_offset(okt_vor + 1), 3600);
    }

    #[test]
    fn gruppen_und_texte() {
        // jetzt = 18.10.2026 09:40 Ortszeit (+2) = 07:40 UTC
        let now = days_from_civil(2026, 10, 18) * 86400 + 7 * 3600 + 40 * 60;
        let heute = days_from_civil(2026, 10, 18) * 86400 + 6 * 3600 + 42 * 60; // 08:42 Ortszeit
        let gestern = days_from_civil(2026, 10, 17) * 86400 + 15 * 3600 + 20 * 60; // 17:20 Ortszeit
        let alt = days_from_civil(2026, 9, 1) * 86400 + 10 * 3600;
        assert_eq!(group_label(heute, now), "Heute");
        assert_eq!(group_label(gestern, now), "Gestern");
        assert_eq!(group_label(alt, now), "Älter");
        assert_eq!(short_label(heute, now), "08:42");
        assert_eq!(short_label(alt, now), "1.9.");
        assert_eq!(long_label(gestern, now), "Gestern, 17:20");
        assert_eq!(long_label(alt, now), "1.9.2026, 12:00");
        // Mitternachtsgrenze: 00:30 Ortszeit ist heute, obwohl UTC noch «gestern» ist
        let nacht = days_from_civil(2026, 10, 17) * 86400 + 22 * 3600 + 30 * 60;
        assert_eq!(group_label(nacht, now), "Heute");
    }
}
