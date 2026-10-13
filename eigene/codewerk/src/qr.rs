//! Kern: Inhalte (Text, WLAN, E-Mail, Kontakt) in eine QR-Nutzlast wandeln
//! und als Bild (RGBA/PNG) oder SVG erzeugen. Plattformunabhängig und ohne
//! GUI – wird von der Oberfläche und von der Kommandozeile genutzt.

use image::{ImageFormat, Rgba, RgbaImage};
use qrcode::{EcLevel, QrCode};
use std::io::Cursor;

/// Fehlerkorrektur-Stufe.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ec {
    L,
    M,
    Q,
    H,
}

impl Ec {
    pub fn level(self) -> EcLevel {
        match self {
            Ec::L => EcLevel::L,
            Ec::M => EcLevel::M,
            Ec::Q => EcLevel::Q,
            Ec::H => EcLevel::H,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Ec::L => "L – niedrig (7 %)",
            Ec::M => "M – mittel (15 %)",
            Ec::Q => "Q – gehoben (25 %)",
            Ec::H => "H – hoch (30 %)",
        }
    }
}

/// Darstellung (Farben, Grösse, Rand).
#[derive(Clone)]
pub struct Style {
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub scale: u32, // Pixel je Modul
    pub quiet: u32, // Rand in Modulen
    pub ec: Ec,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fg: [0x12, 0x12, 0x12],
            bg: [0xff, 0xff, 0xff],
            scale: 10,
            quiet: 4,
            ec: Ec::M,
        }
    }
}

/// Inhaltstyp des QR-Codes.
#[derive(Clone, PartialEq)]
pub enum Content {
    Text(String),
    Wifi {
        ssid: String,
        pass: String,
        enc: WifiEnc,
        hidden: bool,
    },
    Email {
        to: String,
        subject: String,
        body: String,
    },
    Contact {
        name: String,
        phone: String,
        email: String,
        org: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WifiEnc {
    Wpa,
    Wep,
    None,
}

impl WifiEnc {
    fn code(self) -> &'static str {
        match self {
            WifiEnc::Wpa => "WPA",
            WifiEnc::Wep => "WEP",
            WifiEnc::None => "nopass",
        }
    }
}

/// escape für WLAN-Felder (Sonderzeichen \ ; , : " maskieren).
fn wifi_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// einfache URL-Prozent-Kodierung für mailto-Parameter.
fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// vCard-Feld entschärfen (Zeilenumbrüche/Kommas/Semikolons).
fn vcard_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

impl Content {
    /// Baut die eigentliche Zeichenkette, die im QR-Code steht.
    pub fn payload(&self) -> String {
        match self {
            Content::Text(t) => t.clone(),
            Content::Wifi {
                ssid,
                pass,
                enc,
                hidden,
            } => {
                let p = if *enc == WifiEnc::None {
                    String::new()
                } else {
                    format!("P:{};", wifi_escape(pass))
                };
                format!(
                    "WIFI:T:{};S:{};{}{};",
                    enc.code(),
                    wifi_escape(ssid),
                    p,
                    if *hidden { "H:true;" } else { "" }
                )
            }
            Content::Email { to, subject, body } => {
                let mut q = Vec::new();
                if !subject.is_empty() {
                    q.push(format!("subject={}", url_encode(subject)));
                }
                if !body.is_empty() {
                    q.push(format!("body={}", url_encode(body)));
                }
                if q.is_empty() {
                    format!("mailto:{}", to)
                } else {
                    format!("mailto:{}?{}", to, q.join("&"))
                }
            }
            Content::Contact {
                name,
                phone,
                email,
                org,
            } => {
                let mut s = String::from("BEGIN:VCARD\nVERSION:3.0\n");
                if !name.is_empty() {
                    s.push_str(&format!("FN:{}\n", vcard_escape(name)));
                }
                if !org.is_empty() {
                    s.push_str(&format!("ORG:{}\n", vcard_escape(org)));
                }
                if !phone.is_empty() {
                    s.push_str(&format!("TEL:{}\n", vcard_escape(phone)));
                }
                if !email.is_empty() {
                    s.push_str(&format!("EMAIL:{}\n", vcard_escape(email)));
                }
                s.push_str("END:VCARD");
                s
            }
        }
    }
}

fn build_code(data: &str, ec: Ec) -> Result<QrCode, String> {
    if data.is_empty() {
        return Err("Kein Inhalt".into());
    }
    QrCode::with_error_correction_level(data.as_bytes(), ec.level())
        .map_err(|e| format!("QR-Code zu lang oder ungültig: {e}"))
}

/// Erzeugt ein RGBA-Bild (Breite = Höhe). Rückgabe: (pixels, size).
pub fn to_rgba(data: &str, st: &Style) -> Result<(Vec<u8>, u32), String> {
    let code = build_code(data, st.ec)?;
    let w = code.width();
    let colors = code.to_colors();
    let scale = st.scale.max(1);
    let quiet = st.quiet;
    let size = (w as u32 + 2 * quiet) * scale;
    let fg = Rgba([st.fg[0], st.fg[1], st.fg[2], 255]);
    let bg = Rgba([st.bg[0], st.bg[1], st.bg[2], 255]);
    let mut img = RgbaImage::from_pixel(size, size, bg);
    for y in 0..w {
        for x in 0..w {
            if colors[y * w + x] == qrcode::Color::Dark {
                let px0 = (x as u32 + quiet) * scale;
                let py0 = (y as u32 + quiet) * scale;
                for dy in 0..scale {
                    for dx in 0..scale {
                        img.put_pixel(px0 + dx, py0 + dy, fg);
                    }
                }
            }
        }
    }
    Ok((img.into_raw(), size))
}

/// Erzeugt die PNG-Bytes.
pub fn to_png(data: &str, st: &Style) -> Result<Vec<u8>, String> {
    let (raw, size) = to_rgba(data, st)?;
    let img = RgbaImage::from_raw(size, size, raw).ok_or("Bildpuffer ungültig")?;
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Erzeugt ein SVG (verlustfrei skalierbar, ideal zum Drucken).
pub fn to_svg(data: &str, st: &Style) -> Result<String, String> {
    use qrcode::render::svg;
    let code = build_code(data, st.ec)?;
    let w = code.width() as u32;
    let px = (w + 2 * st.quiet) * st.scale.max(1);
    let s = code
        .render::<svg::Color>()
        .min_dimensions(px, px)
        .quiet_zone(st.quiet > 0)
        .dark_color(svg::Color(&hex(st.fg)))
        .light_color(svg::Color(&hex(st.bg)))
        .build();
    Ok(s)
}
