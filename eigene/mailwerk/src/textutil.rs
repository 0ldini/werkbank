//! Text-Bausteine für Mails: Quoted-Printable, Zeichensätze, Kopfzeilen nach
//! RFC 2047, HTML → Text. Nur Standardbibliothek.

use crate::b64;

// ---------------------------------------------------------------- Quoted-Printable

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Dekodiert Quoted-Printable. `header = true`: `_` bedeutet Leerzeichen (RFC 2047, Q).
pub fn decode_qp(input: &[u8], header: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let c = input[i];
        if c == b'=' {
            // weicher Zeilenumbruch
            if input.get(i + 1) == Some(&b'\r') && input.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            if input.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if let (Some(h), Some(l)) = (
                input.get(i + 1).and_then(|&x| hexval(x)),
                input.get(i + 2).and_then(|&x| hexval(x)),
            ) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
            out.push(c);
            i += 1;
        } else if header && c == b'_' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Kodiert als Quoted-Printable (Zeilen höchstens 76 Zeichen, Zeilenende CRLF).
pub fn encode_qp(input: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len() + input.len() / 10);
    let mut line_len = 0usize;
    let mut i = 0;
    while i < input.len() {
        let c = input[i];
        // Zeilenende (CRLF oder LF) bleibt ein harter Umbruch.
        let is_eol = c == b'\n' || (c == b'\r' && input.get(i + 1) == Some(&b'\n'));
        if is_eol {
            out.push_str("\r\n");
            line_len = 0;
            i += if c == b'\r' { 2 } else { 1 };
            continue;
        }
        let next_is_eol =
            i + 1 >= input.len() || input[i + 1] == b'\n' || (input[i + 1] == b'\r' && input.get(i + 2) == Some(&b'\n'));
        let literal = (c >= 33 && c <= 126 && c != b'=') || ((c == b' ' || c == b'\t') && !next_is_eol);
        let width = if literal { 1 } else { 3 };
        // Platz für ein abschliessendes "=" lassen.
        if line_len + width > 75 {
            out.push_str("=\r\n");
            line_len = 0;
        }
        if literal {
            out.push(c as char);
        } else {
            out.push('=');
            out.push(HEX[(c >> 4) as usize] as char);
            out.push(HEX[(c & 15) as usize] as char);
        }
        line_len += width;
        i += 1;
    }
    out
}

// ---------------------------------------------------------------- Zeichensätze

const CP1252_80: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”',
    '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

/// Wandelt Bytes in Text. Unterstützt UTF-8, US-ASCII, ISO-8859-1/-15 und
/// Windows-1252; alles andere wird als UTF-8 mit Ersatzzeichen gelesen.
pub fn to_string_charset(bytes: &[u8], charset: &str) -> String {
    let cs = charset.trim().trim_matches('"').to_ascii_lowercase();
    match cs.as_str() {
        "iso-8859-1" | "iso8859-1" | "latin1" | "latin-1" | "l1" => bytes.iter().map(|&b| b as char).collect(),
        "iso-8859-15" | "latin9" | "latin-9" => bytes
            .iter()
            .map(|&b| match b {
                0xA4 => '€',
                0xA6 => 'Š',
                0xA8 => 'š',
                0xB4 => 'Ž',
                0xB8 => 'ž',
                0xBC => 'Œ',
                0xBD => 'œ',
                0xBE => 'Ÿ',
                _ => b as char,
            })
            .collect(),
        "windows-1252" | "cp1252" | "windows1252" | "x-ansi" => bytes
            .iter()
            .map(|&b| if (0x80..0xA0).contains(&b) { CP1252_80[(b - 0x80) as usize] } else { b as char })
            .collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

// ---------------------------------------------------------------- RFC 2047

/// Löst `=?charset?B|Q?…?=` in Kopfzeilen auf. Leerraum zwischen zwei
/// kodierten Wörtern entfällt.
pub fn decode_encoded_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut last_was_encoded = false;
    while let Some(start) = rest.find("=?") {
        let (before, after) = rest.split_at(start);
        match parse_encoded_word(after) {
            Some((decoded, used)) => {
                if !(last_was_encoded && before.chars().all(|c| c == ' ' || c == '\t' || c == '\r' || c == '\n')) {
                    out.push_str(before);
                }
                out.push_str(&decoded);
                rest = &after[used..];
                last_was_encoded = true;
            }
            None => {
                out.push_str(before);
                out.push_str("=?");
                rest = &after[2..];
                last_was_encoded = false;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `s` beginnt mit `=?`. Liefert (Text, verbrauchte Bytes).
fn parse_encoded_word(s: &str) -> Option<(String, usize)> {
    let body = &s[2..];
    let q1 = body.find('?')?;
    let charset = &body[..q1];
    let after_cs = &body[q1 + 1..];
    let q2 = after_cs.find('?')?;
    let enc = &after_cs[..q2];
    let after_enc = &after_cs[q2 + 1..];
    let end = after_enc.find("?=")?;
    let text = &after_enc[..end];
    if charset.is_empty() || charset.contains(' ') || text.contains(' ') {
        return None;
    }
    let charset = charset.split('*').next().unwrap_or(charset); // RFC 2231: Sprache abtrennen
    let bytes = match enc {
        "B" | "b" => b64::decode(text),
        "Q" | "q" => decode_qp(text.as_bytes(), true),
        _ => return None,
    };
    let used = 2 + q1 + 1 + q2 + 1 + end + 2;
    Some((to_string_charset(&bytes, charset), used))
}

/// Kodiert Text für eine Kopfzeile. Reines ASCII bleibt unverändert; sonst
/// entstehen UTF-8-Base64-Wörter von höchstens 75 Zeichen, getrennt durch
/// Zeilenumbruch mit Leerzeichen.
pub fn encode_header_text(s: &str) -> String {
    let plain = s.chars().all(|c| (' '..='~').contains(&c));
    if plain {
        return s.to_string();
    }
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        // 45 Bytes ergeben 60 Base64-Zeichen; mit "=?UTF-8?B?" und "?=" bleibt es unter 75.
        if cur.len() + ch.len_utf8() > 45 {
            words.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
        .iter()
        .map(|w| format!("=?UTF-8?B?{}?=", b64::encode(w.as_bytes())))
        .collect::<Vec<_>>()
        .join("\r\n ")
}

// ---------------------------------------------------------------- Prozent-Kodierung (RFC 2231)

pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let (Some(h), Some(l)) = (b.get(i + 1).and_then(|&x| hexval(x)), b.get(i + 2).and_then(|&x| hexval(x))) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------- HTML → Text

fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "szlig" => 'ß',
        "euro" => '€',
        "copy" => '©',
        "reg" => '®',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "eacute" => 'é',
        "egrave" => 'è',
        "agrave" => 'à',
        "ccedil" => 'ç',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        _ => return None,
    })
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let tail = &rest[p + 1..];
        let semi = tail.find(';').filter(|&i| i <= 10);
        let mut handled = false;
        if let Some(i) = semi {
            let name = &tail[..i];
            let ch = if let Some(num) = name.strip_prefix('#') {
                let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    num.parse::<u32>().ok()
                };
                code.and_then(char::from_u32)
            } else {
                named_entity(name)
            };
            if let Some(c) = ch {
                out.push(c);
                rest = &tail[i + 1..];
                handled = true;
            }
        }
        if !handled {
            out.push('&');
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// Wandelt HTML in lesbaren Text: Skripte und Stile entfallen, Absätze und
/// Zeilenumbrüche bleiben, Listenpunkte werden zu «• ». Bilder werden nie geladen.
pub fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Kommentar
            if lower[i..].starts_with("<!--") {
                match lower[i + 4..].find("-->") {
                    Some(e) => i += 4 + e + 3,
                    None => break,
                }
                continue;
            }
            let end = match html[i..].find('>') {
                Some(e) => i + e,
                None => break,
            };
            let tag_full = &lower[i + 1..end];
            let name: String = tag_full
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            let closing = tag_full.starts_with('/');
            if (name == "script" || name == "style" || name == "head") && !closing {
                let close = format!("</{}", name);
                match lower[end..].find(&close) {
                    Some(c) => {
                        let after = end + c;
                        i = lower[after..].find('>').map(|g| after + g + 1).unwrap_or(bytes.len());
                    }
                    None => i = bytes.len(),
                }
                continue;
            }
            match name.as_str() {
                "br" => out.push('\n'),
                "p" | "div" | "tr" | "table" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "ul" | "ol" => {
                    if !out.ends_with("\n\n") {
                        if !out.ends_with('\n') {
                            out.push('\n');
                        }
                        if name == "p" || name.starts_with('h') || name == "blockquote" {
                            out.push('\n');
                        }
                    }
                }
                "li" if !closing => {
                    if !out.ends_with('\n') && !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str("• ");
                }
                "li" => {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                "td" | "th" if closing => out.push_str("  "),
                _ => {}
            }
            i = end + 1;
        } else {
            let next = html[i..].find('<').map(|p| i + p).unwrap_or(bytes.len());
            out.push_str(&html[i..next]);
            i = next;
        }
    }
    let text = decode_entities(&out);
    collapse_whitespace(&text)
}

/// Leerzeichen-Folgen auf eines, höchstens eine Leerzeile in Folge, Ränder weg.
fn collapse_whitespace(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in s.replace('\r', "").split('\n') {
        let mut line = String::new();
        let mut prev_space = false;
        for ch in raw.chars() {
            let sp = ch == ' ' || ch == '\t' || ch == '\u{a0}';
            if sp {
                if !prev_space && !line.is_empty() {
                    line.push(' ');
                }
                prev_space = true;
            } else {
                line.push(ch);
                prev_space = false;
            }
        }
        lines.push(line.trim_end().to_string());
    }
    let mut out = String::new();
    let mut blank = 0;
    for l in lines {
        if l.is_empty() {
            blank += 1;
            if blank <= 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(&l);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

// ---------------------------------------------------------------- Kleinkram

/// Anfangsbuchstaben für den runden Absender-Kreis (1–2 Zeichen, Grossbuchstaben).
pub fn initials(name_or_email: &str) -> String {
    let base = name_or_email.split('@').next().unwrap_or(name_or_email);
    let mut letters: Vec<char> = Vec::new();
    for word in base.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if let Some(c) = word.chars().next() {
            letters.push(c.to_uppercase().next().unwrap_or(c));
        }
        if letters.len() == 2 {
            break;
        }
    }
    if letters.is_empty() {
        "?".to_string()
    } else {
        letters.into_iter().collect()
    }
}

/// Dateiname ohne Pfadanteile und Steuerzeichen (für das Speichern von Anhängen).
pub fn safe_filename(name: &str) -> String {
    let last = name.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(name);
    let cleaned: String = last
        .chars()
        .map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    let t = cleaned.trim().trim_matches('.').to_string();
    if t.is_empty() {
        "anhang".to_string()
    } else {
        t
    }
}

pub fn human_size(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{} KB", (n + 512) / 1024)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qp_roundtrip() {
        assert_eq!(decode_qp(b"Gr=C3=BCsse =\r\nvon hier", false), "Grüsse von hier".as_bytes());
        assert_eq!(decode_qp(b"a_b=20c", true), b"a b c");
        let text = "Müller schreibt: Grüezi!\nZweite Zeile mit Umlaut ä und Gleichzeichen = ok.\n";
        let enc = encode_qp(text.as_bytes());
        for l in enc.split("\r\n") {
            assert!(l.len() <= 76, "zu lang: {l}");
        }
        assert_eq!(decode_qp(enc.as_bytes(), false), text.replace('\n', "\r\n").as_bytes());
    }

    #[test]
    fn qp_lange_zeile_und_leerzeichen_am_ende() {
        let long = "x".repeat(200);
        let enc = encode_qp(long.as_bytes());
        for l in enc.split("\r\n") {
            assert!(l.len() <= 76);
        }
        assert_eq!(decode_qp(enc.as_bytes(), false), long.as_bytes());
        let enc2 = encode_qp(b"Ende \nNeu");
        assert!(enc2.starts_with("Ende=20\r\n"), "{enc2}");
    }

    #[test]
    fn zeichensaetze() {
        assert_eq!(to_string_charset(&[0x47, 0x72, 0xfc, 0xdf, 0x65], "ISO-8859-1"), "Grüße");
        assert_eq!(to_string_charset(&[0x80, 0x93, 0x94], "windows-1252"), "€“”");
        assert_eq!(to_string_charset(&[0xa4], "iso-8859-15"), "€");
        assert_eq!(to_string_charset("Grüezi".as_bytes(), "utf-8"), "Grüezi");
    }

    #[test]
    fn encoded_words() {
        assert_eq!(decode_encoded_words("=?UTF-8?B?R3LDvMOfZQ==?="), "Grüße");
        assert_eq!(decode_encoded_words("=?iso-8859-1?Q?Gr=FC=DFe_aus_Z=FCrich?="), "Grüße aus Zürich");
        // Leerraum zwischen zwei kodierten Wörtern entfällt
        assert_eq!(decode_encoded_words("=?UTF-8?Q?Gr=C3=BC?= =?UTF-8?Q?=C3=9Fe?="), "Grüße");
        // gemischt
        assert_eq!(decode_encoded_words("Re: =?UTF-8?Q?K=C3=BCndigung?= bestätigt"), "Re: Kündigung bestätigt");
        // kaputt bleibt stehen
        assert_eq!(decode_encoded_words("a =? b"), "a =? b");
    }

    #[test]
    fn header_kodieren() {
        assert_eq!(encode_header_text("Hallo Welt"), "Hallo Welt");
        let e = encode_header_text("Grüße aus Zürich");
        assert!(e.starts_with("=?UTF-8?B?"));
        assert_eq!(decode_encoded_words(&e), "Grüße aus Zürich");
        let long = "Ä".repeat(100);
        let e = encode_header_text(&long);
        for l in e.split("\r\n") {
            assert!(l.trim().len() <= 75, "Zeile {}", l.len());
        }
        assert_eq!(decode_encoded_words(&e.replace("\r\n ", " ")), long);
    }

    #[test]
    fn prozent() {
        assert_eq!(percent_encode("Röntgen bild.pdf"), "R%C3%B6ntgen%20bild.pdf");
        assert_eq!(percent_decode("R%C3%B6ntgen%20bild.pdf"), "Röntgen bild.pdf".as_bytes());
    }

    #[test]
    fn html_wird_text() {
        let h = "<html><head><style>p{color:red}</style></head><body><p>Hallo <b>Welt</b> &amp; Freunde</p><p>Zweiter&nbsp;Absatz<br>mit Umbruch</p><ul><li>eins</li><li>zwei &auml;</li></ul><script>alert(1)</script><!-- x --></body></html>";
        let t = html_to_text(h);
        assert_eq!(t, "Hallo Welt & Freunde\n\nZweiter Absatz\nmit Umbruch\n\n• eins\n• zwei ä");
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a &#65; &#x42; &euro; &unbekannt; &"), "a A B € &unbekannt; &");
    }

    #[test]
    fn kleinkram() {
        assert_eq!(initials("Beatrice Keller"), "BK");
        assert_eq!(initials("b.keller@example.org"), "BK");
        assert_eq!(initials("müller"), "M");
        assert_eq!(initials(""), "?");
        assert_eq!(safe_filename("../../etc/pass:wd.txt"), "pass_wd.txt");
        assert_eq!(safe_filename("C:\\x\\y\\bild.png"), "bild.png");
        assert_eq!(human_size(1536), "2 KB");
    }
}
