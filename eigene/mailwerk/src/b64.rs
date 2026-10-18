//! Base64 (RFC 4648) ohne Abhängigkeiten. Nur Standardbibliothek.

const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Kodiert ohne Zeilenumbrüche.
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHA[((n >> 18) & 63) as usize] as char);
        out.push(ALPHA[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHA[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHA[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Kodiert und bricht nach `line` Zeichen mit CRLF um (für Mail-Anhänge: 76).
/// Das Ergebnis endet mit CRLF, sofern Daten vorhanden sind.
pub fn encode_wrapped(data: &[u8], line: usize) -> String {
    let flat = encode(data);
    let mut out = String::with_capacity(flat.len() + flat.len() / line.max(1) * 2 + 2);
    let bytes = flat.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let end = (i + line).min(bytes.len());
        out.push_str(&flat[i..end]);
        out.push_str("\r\n");
        i = end;
    }
    out
}

fn val(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a') as u32 + 26),
        b'0'..=b'9' => Some((c - b'0') as u32 + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Dekodiert nachsichtig: Leerraum und ungültige Zeichen werden übersprungen,
/// fehlendes Auffüllen (`=`) ist erlaubt, ein `=` beendet die Daten.
pub fn decode(s: &str) -> Vec<u8> {
    decode_bytes(s.as_bytes())
}

pub fn decode_bytes(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in s {
        if c == b'=' {
            break;
        }
        if let Some(v) = val(c) {
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((acc >> bits) & 0xff) as u8);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vektoren() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn rueckweg_und_nachsicht() {
        assert_eq!(decode("Zm9vYmFy"), b"foobar");
        assert_eq!(decode("Zm9v\r\nYmE="), b"fooba");
        assert_eq!(decode("Zm9vYg"), b"foob");
        assert_eq!(decode("Zg"), b"f");
        let bin: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&encode(&bin)), bin);
    }

    #[test]
    fn umbruch_bei_76() {
        let data = vec![0xabu8; 100];
        let w = encode_wrapped(&data, 76);
        let lines: Vec<&str> = w.split("\r\n").collect();
        assert_eq!(lines[0].len(), 76);
        assert!(lines[1].len() <= 76 && !lines[1].is_empty());
        assert_eq!(lines[2], "");
        assert_eq!(decode(&w), data);
    }
}
