//! Unix-Zeit -> Datumstext, ohne externe Bibliothek. Tokens im Muster:
//! JJJJ (Jahr 4-stellig), JJ (2-stellig), MM (Monat), TT (Tag),
//! hh (Stunde), mm (Minute), ss (Sekunde). Berechnet in UTC.

fn civil_from_days(z0: i64) -> (i64, u32, u32) {
    let z = z0 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn fmt_date(secs: i64, pattern: &str) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mi = (rem % 3600) / 60;
    let ss = rem % 60;
    pattern
        .replace("JJJJ", &format!("{:04}", y))
        .replace("JJ", &format!("{:02}", y.rem_euclid(100)))
        .replace("MM", &format!("{:02}", m))
        .replace("TT", &format!("{:02}", d))
        .replace("hh", &format!("{:02}", hh))
        .replace("mm", &format!("{:02}", mi))
        .replace("ss", &format!("{:02}", ss))
}
