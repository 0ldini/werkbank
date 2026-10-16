//! Passwort-Stärke – vollständig offline, nur Standardbibliothek. Prüft Länge,
//! Zeichenklassen, Entropie, Muster (Wiederholungen, Reihen, Tastatur-Reihen)
//! und vergleicht mit einer Liste bekannter Passwörter. Gut testbar.

pub struct PwResult {
    pub len: usize,
    pub has_lower: bool,
    pub has_upper: bool,
    pub has_digit: bool,
    pub has_symbol: bool,
    pub classes: u8,
    pub entropy_bits: f64,
    pub common: bool,
    pub repeat: bool,
    pub sequence: bool,
    pub keyboard: bool,
    pub level: u8, // 0..=4
    pub crack: String,
}

pub fn level_label(l: u8) -> &'static str {
    match l {
        0 => "Sehr schwach",
        1 => "Schwach",
        2 => "Mittel",
        3 => "Stark",
        _ => "Sehr stark",
    }
}

/// Analysiert `pw`. `common` ist eine Liste bekannter (schwacher) Passwörter.
pub fn analyze(pw: &str, common: &[&str]) -> PwResult {
    let chars: Vec<char> = pw.chars().collect();
    let len = chars.len();
    let mut has_lower = false;
    let mut has_upper = false;
    let mut has_digit = false;
    let mut has_symbol = false;
    for &c in &chars {
        if c.is_ascii_lowercase() {
            has_lower = true;
        } else if c.is_ascii_uppercase() {
            has_upper = true;
        } else if c.is_ascii_digit() {
            has_digit = true;
        } else {
            has_symbol = true;
        }
    }
    let classes = has_lower as u8 + has_upper as u8 + has_digit as u8 + has_symbol as u8;

    // Zeichenraum
    let mut pool = 0u32;
    if has_lower {
        pool += 26;
    }
    if has_upper {
        pool += 26;
    }
    if has_digit {
        pool += 10;
    }
    if has_symbol {
        pool += 33;
    }
    let pool = pool.max(1);

    let base = (len as f64) * (pool as f64).log2();

    let lower = pw.to_lowercase();
    let is_common = !pw.is_empty() && common.iter().any(|c| c.eq_ignore_ascii_case(pw));
    let repeat = has_repeat(&chars);
    let sequence = has_sequence(&lower);
    let keyboard = has_keyboard(&lower);

    let mut eff = base;
    if repeat || sequence || keyboard {
        eff *= 0.55;
    }
    if is_common {
        eff = eff.min(10.0);
    }
    if len == 0 {
        eff = 0.0;
    }

    let level = if is_common || eff < 28.0 {
        0
    } else if eff < 40.0 {
        1
    } else if eff < 60.0 {
        2
    } else if eff < 96.0 {
        3
    } else {
        4
    };

    PwResult {
        len,
        has_lower,
        has_upper,
        has_digit,
        has_symbol,
        classes,
        entropy_bits: eff,
        common: is_common,
        repeat,
        sequence,
        keyboard,
        level,
        crack: crack_time(eff),
    }
}

fn has_repeat(chars: &[char]) -> bool {
    let mut run = 1;
    for i in 1..chars.len() {
        if chars[i] == chars[i - 1] {
            run += 1;
            if run >= 3 {
                return true;
            }
        } else {
            run = 1;
        }
    }
    false
}

/// Auf- oder absteigende Reihe von mindestens 4 (abcd, 4321 …).
fn has_sequence(s: &str) -> bool {
    let b: Vec<u8> = s.bytes().collect();
    if b.len() < 4 {
        return false;
    }
    let seqish = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    let mut up = 1;
    let mut down = 1;
    for i in 1..b.len() {
        if seqish(b[i]) && seqish(b[i - 1]) && b[i] == b[i - 1] + 1 {
            up += 1;
            down = 1;
        } else if seqish(b[i]) && seqish(b[i - 1]) && b[i] + 1 == b[i - 1] {
            down += 1;
            up = 1;
        } else {
            up = 1;
            down = 1;
        }
        if up >= 4 || down >= 4 {
            return true;
        }
    }
    false
}

/// Tastatur-Reihe von mindestens 4 Zeichen (qwertz, asdf …).
fn has_keyboard(s: &str) -> bool {
    const ROWS: &[&str] = &[
        "qwertzuiop", "qwertyuiop", "asdfghjkl", "yxcvbnm", "zxcvbnm", "1234567890",
    ];
    for row in ROWS {
        let fwd: Vec<char> = row.chars().collect();
        let rev: Vec<char> = row.chars().rev().collect();
        if contains_run(s, &fwd, 4) || contains_run(s, &rev, 4) {
            return true;
        }
    }
    false
}

fn contains_run(s: &str, row: &[char], n: usize) -> bool {
    if row.len() < n {
        return false;
    }
    for w in row.windows(n) {
        let sub: String = w.iter().collect();
        if s.contains(&sub) {
            return true;
        }
    }
    false
}

/// Geschätzte Knackzeit bei schnellem Offline-Angriff (~10 Mrd. Versuche/s).
fn crack_time(bits: f64) -> String {
    if bits <= 0.0 {
        return "sofort".into();
    }
    // Durchschnittlich die halbe Menge an Versuchen.
    let log_guesses = bits - 1.0;
    let rate_log2 = 33.2; // log2(1e10)
    let secs_log2 = log_guesses - rate_log2;
    if secs_log2 < 0.0 {
        return "sofort".into();
    }
    let secs = 2f64.powf(secs_log2.min(300.0));
    humanize(secs)
}

fn humanize(secs: f64) -> String {
    let min = 60.0;
    let hour = 3600.0;
    let day = 86_400.0;
    let month = day * 30.0;
    let year = day * 365.0;
    let century = year * 100.0;
    if secs < min {
        "ein paar Sekunden".into()
    } else if secs < hour {
        format!("ca. {} Minuten", (secs / min).round() as u64)
    } else if secs < day {
        format!("ca. {} Stunden", (secs / hour).round() as u64)
    } else if secs < month {
        format!("ca. {} Tage", (secs / day).round() as u64)
    } else if secs < year {
        format!("ca. {} Monate", (secs / month).round() as u64)
    } else if secs < century {
        format!("ca. {} Jahre", (secs / year).round() as u64)
    } else if secs < century * 1000.0 {
        format!("ca. {} Jahrhunderte", (secs / century).round().max(1.0) as u64)
    } else {
        "praktisch unknackbar".into()
    }
}
