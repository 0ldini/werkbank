//! Papierkorb: liest die Metadaten-Datei ($I…) des Windows-Papierkorbs
//! (ursprünglicher Pfad, Grösse, Löschzeitpunkt). Binärformat, nur
//! Standardbibliothek – gut testbar. Das eigentliche Wiederherstellen (die
//! $R…-Datei zurückkopieren) macht die Systemschicht.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recycled {
    pub orig_path: String,
    pub size: u64,
    pub filetime: u64,
}

fn u64le(b: &[u8], off: usize) -> Option<u64> {
    let s = b.get(off..off + 8)?;
    Some(u64::from_le_bytes(s.try_into().ok()?))
}
fn i32le(b: &[u8], off: usize) -> Option<i32> {
    let s = b.get(off..off + 4)?;
    Some(i32::from_le_bytes(s.try_into().ok()?))
}

fn utf16(b: &[u8], start: usize, max_chars: usize) -> String {
    let mut out = String::new();
    let mut i = start;
    for _ in 0..max_chars {
        if i + 1 >= b.len() {
            break;
        }
        let u = u16::from_le_bytes([b[i], b[i + 1]]);
        if u == 0 {
            break;
        }
        out.push(char::from_u32(u as u32).unwrap_or('\u{fffd}'));
        i += 2;
    }
    out
}

/// Parst eine $I-Datei des Papierkorbs (Format 1 und 2).
pub fn parse_i_file(b: &[u8]) -> Option<Recycled> {
    let version = u64le(b, 0)?;
    let size = u64le(b, 8)?;
    let filetime = u64le(b, 16)?;
    let orig_path = if version >= 2 {
        let n = i32le(b, 24)?.max(0) as usize; // Zeichen inkl. Null
        let chars = n.saturating_sub(1);
        utf16(b, 28, chars)
    } else {
        // Format 1: fester 260-Zeichen-Pfad ab Offset 24
        utf16(b, 24, 260)
    };
    Some(Recycled { orig_path, size, filetime })
}

// ---- FILETIME (100ns seit 1601-01-01 UTC) -> Datum ----

/// Zivildatum aus Tagen seit 1970-01-01 (Algorithmus nach H. Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (y + (m <= 2) as i64, m, d)
}

/// FILETIME -> (Jahr, Monat, Tag, Stunde, Minute, Sekunde) in UTC.
pub fn filetime_to_date(ft: u64) -> (i64, u32, u32, u32, u32, u32) {
    let total = (ft / 10_000_000) as i64 - 11_644_473_600;
    let days = total.div_euclid(86_400);
    let rem = total.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    (y, mo, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

/// Kurzes Datum "TT.MM.JJJJ hh:mm".
pub fn fmt_date(ft: u64) -> String {
    if ft == 0 {
        return "—".into();
    }
    let (y, mo, d, hh, mm, _s) = filetime_to_date(ft);
    format!("{:02}.{:02}.{:04} {:02}:{:02}", d, mo, y, hh, mm)
}
