//! Daten für die Widgets «Termine» (Kalender als ICS) und «Nachrichten» (RSS).
//! Bewusst ohne XML-/ICS-Bibliothek: nur das, was gängige Kalender und Nachrichtenseiten liefern.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Beginn in lokaler Zeit.
    pub start: NaiveDateTime,
    pub all_day: bool,
    pub title: String,
}

/// Zeilen eines ICS-Texts mit aufgelösten Fortsetzungszeilen (RFC 5545: Zeile beginnt mit Leerzeichen).
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        out.push(line.to_string());
    }
    out
}

fn unescape_ics(s: &str) -> String {
    s.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// DTSTART-Wert → (lokale Zeit, ganztägig).
fn parse_dt(params: &str, value: &str) -> Option<(NaiveDateTime, bool)> {
    let v = value.trim();
    if params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME") || v.len() == 8 {
        let d = NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        return Some((d.and_time(NaiveTime::MIN), true));
    }
    if let Some(utc) = v.strip_suffix('Z') {
        let n = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        let local = Utc.from_utc_datetime(&n).with_timezone(&Local);
        return Some((local.naive_local(), false));
    }
    // Mit TZID oder ohne Zone: als Ortszeit des Rechners lesen (für die Schweiz passt das).
    let n = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    Some((n, false))
}

struct Rule {
    freq: String,
    interval: i64,
    count: Option<u32>,
    until: Option<NaiveDateTime>,
    byday: Vec<Weekday>,
}

fn parse_rule(v: &str) -> Option<Rule> {
    let mut r = Rule { freq: String::new(), interval: 1, count: None, until: None, byday: Vec::new() };
    for part in v.split(';') {
        let (k, val) = part.split_once('=')?;
        match k {
            "FREQ" => r.freq = val.to_string(),
            "INTERVAL" => r.interval = val.parse().unwrap_or(1).max(1),
            "COUNT" => r.count = val.parse().ok(),
            "UNTIL" => r.until = parse_dt("", val).map(|(d, _)| d),
            "BYDAY" => {
                for d in val.split(',') {
                    let code = d.trim_start_matches(|c: char| c == '-' || c == '+' || c.is_ascii_digit());
                    let wd = match code {
                        "MO" => Weekday::Mon,
                        "TU" => Weekday::Tue,
                        "WE" => Weekday::Wed,
                        "TH" => Weekday::Thu,
                        "FR" => Weekday::Fri,
                        "SA" => Weekday::Sat,
                        "SU" => Weekday::Sun,
                        _ => continue,
                    };
                    r.byday.push(wd);
                }
            }
            _ => {}
        }
    }
    Some(r)
}

/// Termine von `from` bis `from + days` (sortiert). Wiederholungen: täglich, wöchentlich (auch
/// mit mehreren Wochentagen), monatlich und jährlich am selben Tag; Ausnahmen (EXDATE) werden beachtet.
pub fn parse_ics(text: &str, from: NaiveDateTime, days: i64) -> Vec<Event> {
    let to = from + Duration::days(days);
    let mut out = Vec::new();
    let mut in_event = false;
    let (mut start, mut title, mut rule, mut exdates, mut cancelled, mut recurrence_id) = (None, String::new(), None, Vec::new(), false, false);
    for line in unfold(text) {
        if line == "BEGIN:VEVENT" {
            in_event = true;
            start = None;
            title.clear();
            rule = None;
            exdates.clear();
            cancelled = false;
            recurrence_id = false;
            continue;
        }
        if line == "END:VEVENT" {
            in_event = false;
            let Some((s, all_day)) = start else { continue };
            if cancelled || recurrence_id {
                // Einzeln geänderte Wiederholungen werden als eigener Termin geführt (s. u.).
                if !cancelled && s >= from && s < to {
                    out.push(Event { start: s, all_day, title: title.clone() });
                }
                continue;
            }
            match &rule {
                None => {
                    if (s >= from || (all_day && s.date() == from.date())) && s < to {
                        out.push(Event { start: s, all_day, title: title.clone() });
                    }
                }
                Some(r) => expand(r, s, all_day, &title, &exdates, from, to, &mut out),
            }
            continue;
        }
        if !in_event {
            continue;
        }
        let Some((head, value)) = line.split_once(':') else { continue };
        let (name, params) = head.split_once(';').unwrap_or((head, ""));
        match name {
            "DTSTART" => start = parse_dt(params, value),
            "SUMMARY" => title = unescape_ics(value),
            "RRULE" => rule = parse_rule(value),
            "EXDATE" => exdates.extend(value.split(',').filter_map(|v| parse_dt(params, v).map(|(d, _)| d))),
            "STATUS" => cancelled = value.trim() == "CANCELLED",
            "RECURRENCE-ID" => recurrence_id = true,
            _ => {}
        }
    }
    out.sort_by_key(|e| e.start);
    out.dedup();
    out
}

#[allow(clippy::too_many_arguments)]
fn expand(r: &Rule, s: NaiveDateTime, all_day: bool, title: &str, ex: &[NaiveDateTime], from: NaiveDateTime, to: NaiveDateTime, out: &mut Vec<Event>) {
    let mut n = 0u32;
    let mut emit = |t: NaiveDateTime, out: &mut Vec<Event>| -> bool {
        if r.until.is_some_and(|u| t > u) {
            return false;
        }
        n += 1;
        if r.count.is_some_and(|c| n > c) {
            return false;
        }
        if t >= from && t < to && !ex.contains(&t) {
            out.push(Event { start: t, all_day, title: title.to_string() });
        }
        t < to
    };
    // Obergrenze gegen endlose Schleifen.
    for i in 0..5000i64 {
        let step = i * r.interval;
        match r.freq.as_str() {
            "DAILY" => {
                if !emit(s + Duration::days(step), out) {
                    return;
                }
            }
            "WEEKLY" => {
                let week = s + Duration::weeks(step);
                if r.byday.is_empty() {
                    if !emit(week, out) {
                        return;
                    }
                } else {
                    let monday = week - Duration::days(week.weekday().num_days_from_monday() as i64);
                    let mut days: Vec<i64> = r.byday.iter().map(|d| d.num_days_from_monday() as i64).collect();
                    days.sort();
                    for d in days {
                        let t = monday + Duration::days(d);
                        if t < s {
                            continue;
                        }
                        if !emit(t, out) {
                            return;
                        }
                    }
                }
            }
            "MONTHLY" => {
                let m = s.month0() as i64 + step;
                let (y, mo) = (s.year() as i64 + m.div_euclid(12), m.rem_euclid(12) as u32 + 1);
                if let Some(d) = NaiveDate::from_ymd_opt(y as i32, mo, s.day()) {
                    if !emit(d.and_time(s.time()), out) {
                        return;
                    }
                } else if NaiveDate::from_ymd_opt(y as i32, mo, 1).is_some_and(|d| d.and_time(s.time()) > to) {
                    return;
                }
            }
            "YEARLY" => {
                if let Some(d) = NaiveDate::from_ymd_opt(s.year() + step as i32, s.month(), s.day()) {
                    if !emit(d.and_time(s.time()), out) {
                        return;
                    }
                }
            }
            _ => {
                emit(s, out);
                return;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub title: String,
    pub link: String,
    pub date: Option<DateTime<Local>>,
}

fn tag<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let start = block.find(&open)?;
    let after = &block[start + open.len()..];
    // Tag-Name genau (nicht <titleX>), Attribute erlaubt.
    if !after.starts_with('>') && !after.starts_with(' ') && !after.starts_with('/') {
        return None;
    }
    if after.starts_with("/>") || after.find('>').is_some_and(|i| after[..i].ends_with('/')) {
        // <link href="…"/> (Atom)
        let end = after.find('>')?;
        let attrs = &after[..end];
        let href = attrs.split("href=\"").nth(1)?.split('"').next()?;
        return Some(href);
    }
    let body_start = after.find('>')? + 1;
    let close = format!("</{name}>");
    let end = after[body_start..].find(&close)?;
    Some(&after[body_start..body_start + end])
}

fn clean(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).unwrap_or(s);
    let mut out = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#8217;", "’")
        .replace("&nbsp;", " ");
    // Zahl-Entitäten &#x…; und &#…;
    while let Some(i) = out.find("&#") {
        let Some(len) = out[i..].find(';') else { break };
        let code = &out[i + 2..i + len];
        let ch = if let Some(hex) = code.strip_prefix('x').or_else(|| code.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()
        } else {
            code.parse().ok()
        }
        .and_then(char::from_u32);
        match ch {
            Some(c) => out.replace_range(i..i + len + 1, &c.to_string()),
            None => break,
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Einträge aus RSS 2.0 oder Atom.
pub fn parse_rss(text: &str) -> Vec<Item> {
    let (open, close) = if text.contains("<item") { ("<item", "</item>") } else { ("<entry", "</entry>") };
    let mut out = Vec::new();
    for part in text.split(open).skip(1) {
        let Some(end) = part.find(close) else { continue };
        let block = &part[..end];
        let block = &block[block.find('>').map(|i| i + 1).unwrap_or(0)..];
        let Some(title) = tag(block, "title").map(clean).filter(|t| !t.is_empty()) else { continue };
        let link = tag(block, "link").map(clean).unwrap_or_default();
        let date = tag(block, "pubDate")
            .and_then(|d| DateTime::parse_from_rfc2822(clean(d).as_str()).ok())
            .or_else(|| tag(block, "updated").or_else(|| tag(block, "published")).and_then(|d| DateTime::parse_from_rfc3339(clean(d).as_str()).ok()))
            .map(|d| d.with_timezone(&Local));
        out.push(Item { title, link, date });
        if out.len() >= 30 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn kalender() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART;TZID=Europe/Zurich:20261019T093000\r\nSUMMARY:Einzel\r\n stunde\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261018\r\nSUMMARY:Ganzer Tag\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART:20261001T180000\r\nRRULE:FREQ=WEEKLY;BYDAY=TU,TH\r\nEXDATE:20261020T180000\r\nSUMMARY:Kurs\\, Welpen\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART:20260101T080000\r\nRRULE:FREQ=DAILY;COUNT=3\r\nSUMMARY:Alt\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let ev = parse_ics(ics, dt("2026-10-18 10:00"), 7);
        let names: Vec<String> = ev.iter().map(|e| format!("{} {}", e.start.format("%d %H:%M"), e.title)).collect();
        assert_eq!(names, vec!["18 00:00 Ganzer Tag", "19 09:30 Einzelstunde", "22 18:00 Kurs, Welpen"]);
        assert!(ev[0].all_day);
    }

    #[test]
    fn nachrichten() {
        let rss = r#"<rss><channel><title>SRF</title><item><title><![CDATA[Bund &amp; Kantone]]></title><link>https://srf.ch/a</link><pubDate>Sun, 18 Oct 2026 12:00:00 +0200</pubDate></item><item><title>Zweite &#x00FC;bersicht</title><link>https://srf.ch/b</link></item></channel></rss>"#;
        let items = parse_rss(rss);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Bund & Kantone");
        assert_eq!(items[0].link, "https://srf.ch/a");
        assert!(items[0].date.is_some());
        assert_eq!(items[1].title, "Zweite übersicht");
        let atom = r#"<feed><entry><title>A</title><link href="https://x/1"/><updated>2026-10-18T10:00:00Z</updated></entry></feed>"#;
        assert_eq!(parse_rss(atom)[0].link, "https://x/1");
    }
}
