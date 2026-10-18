//! Reine Logik rund um Links und Dateinamen (ohne egui, nur std):
//! Links in beliebigem Text finden, Dateiname aus Content-Disposition oder Adresse,
//! sichere Dateinamen, freie Namen ohne Überschreiben, Weiterleitungsziele.
//!
//! Prüfung: `rustc --edition 2021 --test src/links.rs -o /tmp/links && /tmp/links`

#![allow(dead_code)]

/// Ein gefundener Link, optional mit einer SHA-256-Prüfsumme, die in derselben Zeile
/// hinter dem Link stand (z. B. «https://…/datei.zip  3a7bd3e2…»).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub url: String,
    pub sha256: Option<String>,
}

/// Zeichen, die einen Link beenden.
fn ends_url(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '{' | '}' | '|' | '\\' | '^' | '[' | ']')
}

/// Schneidet Satzzeichen am Ende ab und eine schliessende Klammer, die keine öffnende hat.
fn trim_url_end(mut s: &str) -> &str {
    loop {
        let Some(last) = s.chars().last() else { return s };
        let cut = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '*' => true,
            ')' => s.matches('(').count() < s.matches(')').count(),
            _ => false,
        };
        if !cut {
            return s;
        }
        s = &s[..s.len() - last.len_utf8()];
    }
}

/// Prüft, ob nach «scheme://» ein Rechnername folgt.
fn has_host(url: &str) -> bool {
    url.split_once("://")
        .map(|(_, rest)| rest.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '['))
        .unwrap_or(false)
}

/// 64 Hex-Zeichen → Prüfsumme in Kleinbuchstaben; alles andere → None.
pub fn parse_sha256(s: &str) -> Option<String> {
    let s = s.trim();
    (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

/// Findet alle http(s)-Links in einem Text (doppelte nur einmal, Reihenfolge bleibt).
pub fn extract_links(text: &str) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for line in text.lines() {
        // ASCII-Kleinschreibung ändert keine Bytelängen, Positionen passen also auf `line`.
        let lower = line.to_ascii_lowercase();
        let mut starts: Vec<usize> = lower
            .match_indices("http://")
            .chain(lower.match_indices("https://"))
            .map(|(p, _)| p)
            .collect();
        starts.sort_unstable();
        let mut line_links: Vec<(usize, usize)> = Vec::new();
        for p in starts {
            if line_links.last().is_some_and(|&(_, end)| p < end) {
                continue; // liegt innerhalb des vorigen Links
            }
            let Some(rest) = line.get(p..) else { continue };
            let len = rest.find(ends_url).unwrap_or(rest.len());
            let url = trim_url_end(&rest[..len]);
            if has_host(url) {
                line_links.push((p, p + url.len()));
            }
        }
        for (i, &(a, b)) in line_links.iter().enumerate() {
            let Some(url) = line.get(a..b) else { continue };
            let next = line_links.get(i + 1).map(|l| l.0).unwrap_or(line.len());
            let tail = line.get(b..next).unwrap_or("");
            let sha = tail
                .split(|c: char| !c.is_ascii_hexdigit())
                .find_map(parse_sha256);
            if let Some(f) = out.iter_mut().find(|f| f.url == url) {
                if f.sha256.is_none() {
                    f.sha256 = sha;
                }
            } else {
                out.push(Found { url: url.to_string(), sha256: sha });
            }
        }
    }
    out
}

/// «%C3%A4» → «ä». Ungültige Folgen bleiben stehen.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'%' {
            let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
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

/// Teil nach «scheme://» bis zum Pfad, ohne Benutzerangaben.
fn authority(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = &rest[..end];
    auth.rsplit_once('@').map(|(_, h)| h).unwrap_or(auth)
}

/// Rechnername eines Links ohne Port, z. B. «example.com».
pub fn host_of(url: &str) -> String {
    let a = authority(url);
    let host = if a.starts_with('[') {
        a.split_once(']').map(|(h, _)| &h[1..]).unwrap_or(a)
    } else {
        a.split(':').next().unwrap_or(a)
    };
    host.to_ascii_lowercase()
}

/// Pfad eines Links ohne Abfrage und Anker.
fn path_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let rest = &rest[rest.find(['/', '?', '#']).unwrap_or(rest.len())..];
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    &rest[..end]
}

/// Dateiname aus der Adresse (letzter Pfadteil), bereinigt.
pub fn file_name_from_url(url: &str) -> Option<String> {
    let last = path_of(url).rsplit('/').next().unwrap_or("");
    sanitize_file_name(&percent_decode(last))
}

/// Zerlegt einen Kopfzeilen-Wert an «;», ohne Anführungszeichen zu zerschneiden.
fn split_params(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            '\\' if quoted => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ';' if !quoted => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur);
    parts
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if let Some(inner) = v.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else {
                out.push(c);
            }
        }
        out
    } else {
        v.to_string()
    }
}

/// Dateiname aus «Content-Disposition» (RFC 6266): `filename*` hat Vorrang vor `filename`.
pub fn content_disposition_filename(header: &str) -> Option<String> {
    let mut plain = None;
    let mut ext = None;
    for p in split_params(header).iter().skip(1) {
        let Some((k, v)) = p.split_once('=') else { continue };
        match k.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                // charset'sprache'kodierter-wert
                let v = unquote(v);
                let mut it = v.splitn(3, '\'');
                let (charset, _lang, val) = (it.next(), it.next(), it.next());
                if let (Some(cs), Some(val)) = (charset, val) {
                    let cs = cs.to_ascii_lowercase();
                    if cs == "utf-8" || cs == "iso-8859-1" || cs.is_empty() {
                        let decoded = if cs == "iso-8859-1" {
                            // Jedes Byte ist ein Zeichen.
                            let raw: Vec<u8> = percent_decode_bytes(val);
                            raw.iter().map(|&b| b as char).collect()
                        } else {
                            percent_decode(val)
                        };
                        ext = sanitize_file_name(&decoded);
                    }
                }
            }
            "filename" => plain = sanitize_file_name(&unquote(v)),
            _ => {}
        }
    }
    ext.or(plain)
}

fn percent_decode_bytes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let Some(v) = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Macht einen Namen als Dateiname auf Windows und Linux brauchbar: keine Pfadteile,
/// keine verbotenen Zeichen, keine reservierten Namen, höchstens 200 Bytes.
pub fn sanitize_file_name(name: &str) -> Option<String> {
    // Nur der letzte Pfadteil zählt (Schutz vor «../../datei»).
    let name = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).trim_start().to_string();
    if s.is_empty() || s.chars().all(|c| c == '.') {
        return None;
    }
    let stem = s.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem[3..].chars().all(|c| c.is_ascii_digit()));
    if reserved {
        s.insert(0, '_');
    }
    if s.len() > 200 {
        // Endung möglichst behalten, nur an Zeichengrenzen schneiden.
        let (base, ext) = match s.rfind('.') {
            Some(p) if s.len() - p <= 16 && p > 0 => (s[..p].to_string(), s[p..].to_string()),
            _ => (s.clone(), String::new()),
        };
        let mut cut = 200usize.saturating_sub(ext.len());
        while cut > 0 && !base.is_char_boundary(cut) {
            cut -= 1;
        }
        s = format!("{}{}", &base[..cut.min(base.len())], ext);
    }
    Some(s)
}

/// «name.ext» → «name (2).ext», «name (3).ext» … bis `taken` nein sagt.
pub fn unique_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (base, ext) = match name.rfind('.') {
        Some(p) if p > 0 => (&name[..p], &name[p..]),
        _ => (name, ""),
    };
    for n in 2..100_000u32 {
        let cand = format!("{base} ({n}){ext}");
        if !taken(&cand) {
            return cand;
        }
    }
    format!("{base} ({}){ext}", std::process::id())
}

/// Ziel einer Weiterleitung (Location) relativ zur bisherigen Adresse auflösen.
/// Nur http und https sind erlaubt.
pub fn resolve_location(base: &str, loc: &str) -> Option<String> {
    let loc = loc.trim();
    if loc.is_empty() {
        return None;
    }
    let lower = loc.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(loc.to_string());
    }
    if lower.contains("://") || lower.starts_with("javascript:") || lower.starts_with("data:") {
        return None;
    }
    let (scheme, rest) = base.split_once("://")?;
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", &rest[..auth_end]);
    if let Some(r) = loc.strip_prefix("//") {
        return Some(format!("{scheme}://{r}"));
    }
    if loc.starts_with('/') {
        return Some(format!("{origin}{loc}"));
    }
    let path = path_of(base);
    if loc.starts_with('?') {
        return Some(format!("{origin}{path}{loc}"));
    }
    let dir = match path.rfind('/') {
        Some(p) => &path[..=p],
        None => "/",
    };
    Some(format!("{origin}{dir}{loc}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_finden() {
        let text = "Hier: https://example.com/a.zip, und (siehe http://x.org/b_(1).rar).\n\
                    <a href=\"https://example.com/c.7z\">c</a> HTTPS://Example.com/d?x=1&y=2.\n\
                    doppelt https://example.com/a.zip\nkein link: http:// oder https://";
        let urls: Vec<String> = extract_links(text).into_iter().map(|f| f.url).collect();
        assert_eq!(
            urls,
            vec![
                "https://example.com/a.zip",
                "http://x.org/b_(1).rar",
                "https://example.com/c.7z",
                "HTTPS://Example.com/d?x=1&y=2",
            ]
        );
    }

    #[test]
    fn pruefsumme_in_der_zeile() {
        let h = "3A7BD3E2360A3D29EEA436FCFB7E44C735D117C42D1C1835420B6B9942DD4F1B";
        let text = format!("https://e.com/x.iso  sha256: {h}\nhttps://e.com/y.iso https://e.com/z.iso");
        let f = extract_links(&text);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].sha256.as_deref(), Some(h.to_ascii_lowercase().as_str()));
        assert_eq!(f[1].sha256, None);
        assert_eq!(parse_sha256("abc"), None);
    }

    #[test]
    fn bbcode_und_umlaute() {
        let f = extract_links("[url=https://e.com/ä.zip]Datei[/url] ünd https://e.com/ö");
        assert_eq!(f[0].url, "https://e.com/ä.zip");
        assert_eq!(f[1].url, "https://e.com/ö");
    }

    #[test]
    fn name_aus_adresse() {
        assert_eq!(file_name_from_url("https://e.com/pfad/Mein%20Film.mkv?token=1#x").as_deref(), Some("Mein Film.mkv"));
        assert_eq!(file_name_from_url("https://e.com/"), None);
        assert_eq!(file_name_from_url("https://e.com"), None);
        assert_eq!(file_name_from_url("https://e.com/a/..%2F..%2Fetc%2Fpasswd").as_deref(), Some("passwd"));
        assert_eq!(file_name_from_url("https://e.com/%E4"), Some("\u{FFFD}".to_string()));
    }

    #[test]
    fn content_disposition() {
        assert_eq!(content_disposition_filename("attachment; filename=\"a b.zip\"").as_deref(), Some("a b.zip"));
        assert_eq!(content_disposition_filename("attachment; filename=plain.txt").as_deref(), Some("plain.txt"));
        assert_eq!(
            content_disposition_filename("attachment; filename=\"fallback.txt\"; filename*=UTF-8''%C3%A4rger%20%E2%82%AC.txt").as_deref(),
            Some("ärger €.txt")
        );
        assert_eq!(content_disposition_filename("attachment; filename*=iso-8859-1'de'%E4.txt").as_deref(), Some("ä.txt"));
        assert_eq!(content_disposition_filename("attachment; filename=\"x;y\\\".txt\"").as_deref(), Some("x;y_.txt"));
        assert_eq!(content_disposition_filename("inline"), None);
        assert_eq!(content_disposition_filename("attachment; filename=\"../../evil.sh\"").as_deref(), Some("evil.sh"));
    }

    #[test]
    fn sichere_namen() {
        assert_eq!(sanitize_file_name("a:b*c?.txt").as_deref(), Some("a_b_c_.txt"));
        assert_eq!(sanitize_file_name("CON.txt").as_deref(), Some("_CON.txt"));
        assert_eq!(sanitize_file_name("com1").as_deref(), Some("_com1"));
        assert_eq!(sanitize_file_name("...").as_deref(), None);
        assert_eq!(sanitize_file_name("  name. ").as_deref(), Some("name"));
        let long = format!("{}.zip", "ä".repeat(150));
        let s = sanitize_file_name(&long).unwrap();
        assert!(s.len() <= 200 && s.ends_with(".zip"));
    }

    #[test]
    fn freie_namen() {
        let taken = ["a.zip", "a (2).zip", "ohne"];
        assert_eq!(unique_name("a.zip", |n| taken.contains(&n)), "a (3).zip");
        assert_eq!(unique_name("ohne", |n| taken.contains(&n)), "ohne (2)");
        assert_eq!(unique_name("neu.txt", |n| taken.contains(&n)), "neu.txt");
        assert_eq!(unique_name(".hidden", |n| n == ".hidden"), ".hidden (2)");
    }

    #[test]
    fn weiterleitungen() {
        let b = "https://e.com/a/b/c.zip?x=1";
        assert_eq!(resolve_location(b, "http://o.org/z").as_deref(), Some("http://o.org/z"));
        assert_eq!(resolve_location(b, "//cdn.e.com/f").as_deref(), Some("https://cdn.e.com/f"));
        assert_eq!(resolve_location(b, "/root.zip").as_deref(), Some("https://e.com/root.zip"));
        assert_eq!(resolve_location(b, "d.zip").as_deref(), Some("https://e.com/a/b/d.zip"));
        assert_eq!(resolve_location(b, "?y=2").as_deref(), Some("https://e.com/a/b/c.zip?y=2"));
        assert_eq!(resolve_location("https://e.com", "x").as_deref(), Some("https://e.com/x"));
        assert_eq!(resolve_location(b, "ftp://e.com/x"), None);
        assert_eq!(resolve_location(b, ""), None);
    }

    #[test]
    fn rechner() {
        assert_eq!(host_of("https://user:pw@Files.Example.com:8443/x"), "files.example.com");
        assert_eq!(host_of("http://[::1]:80/"), "::1");
        assert_eq!(host_of("https://e.com?x"), "e.com");
    }

    #[test]
    fn prozent() {
        assert_eq!(percent_decode("a%20b%zz%"), "a b%zz%");
    }
}
