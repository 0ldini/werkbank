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
    pub cross: bool, // Schweizer Kreuz in der Mitte (für Swiss QR Bill)
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fg: [0x12, 0x12, 0x12],
            bg: [0xff, 0xff, 0xff],
            scale: 10,
            quiet: 4,
            ec: Ec::M,
            cross: false,
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
    Swiss(Box<SwissQr>),
}

/// Felder einer Schweizer QR-Rechnung (Swiss QR Bill, Version 2.0).
#[derive(Clone, PartialEq, Default)]
pub struct SwissQr {
    pub iban: String,
    pub amount: String,
    pub currency: String, // "CHF" / "EUR" (leer = CHF)
    pub cr_name: String,
    pub cr_street: String,
    pub cr_bno: String,
    pub cr_zip: String,
    pub cr_city: String,
    pub cr_country: String,
    pub db_name: String,
    pub db_street: String,
    pub db_bno: String,
    pub db_zip: String,
    pub db_city: String,
    pub db_country: String,
    pub ref_type: String, // "QRR" / "SCOR" / "NON"
    pub reference: String,
    pub message: String,
}

fn s_trunc(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn s_clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}
fn s_country(s: &str) -> String {
    let t = s.trim().to_uppercase();
    if t.is_empty() {
        "CH".into()
    } else {
        s_trunc(&t, 2)
    }
}
fn s_amount(s: &str) -> String {
    let t = s.trim().replace(',', ".");
    if t.is_empty() {
        return String::new();
    }
    match t.parse::<f64>() {
        Ok(v) if v >= 0.0 => format!("{:.2}", v),
        _ => String::new(),
    }
}

/// Baut die Nutzlast der Schweizer QR-Rechnung (31 Elemente, CRLF-getrennt).
pub fn swiss_payload(b: &SwissQr) -> String {
    let mut v: Vec<String> = Vec::with_capacity(31);
    v.push("SPC".into());
    v.push("0200".into());
    v.push("1".into());
    v.push(s_clean(&b.iban));
    v.push("S".into());
    v.push(s_trunc(&b.cr_name, 70));
    v.push(s_trunc(&b.cr_street, 70));
    v.push(s_trunc(&b.cr_bno, 16));
    v.push(s_trunc(&b.cr_zip, 16));
    v.push(s_trunc(&b.cr_city, 35));
    v.push(s_country(&b.cr_country));
    for _ in 0..7 {
        v.push(String::new()); // Ultimate Creditor (reserviert, leer)
    }
    v.push(s_amount(&b.amount));
    v.push(if b.currency.trim().is_empty() {
        "CHF".into()
    } else {
        s_trunc(&b.currency.trim().to_uppercase(), 3)
    });
    if b.db_name.trim().is_empty() {
        for _ in 0..7 {
            v.push(String::new());
        }
    } else {
        v.push("S".into());
        v.push(s_trunc(&b.db_name, 70));
        v.push(s_trunc(&b.db_street, 70));
        v.push(s_trunc(&b.db_bno, 16));
        v.push(s_trunc(&b.db_zip, 16));
        v.push(s_trunc(&b.db_city, 35));
        v.push(s_country(&b.db_country));
    }
    let rt = if b.ref_type.trim().is_empty() {
        "NON".to_string()
    } else {
        b.ref_type.trim().to_uppercase()
    };
    v.push(rt.clone());
    v.push(if rt == "NON" {
        String::new()
    } else {
        s_clean(&b.reference)
    });
    v.push(s_trunc(&b.message, 140));
    v.push("EPD".into());
    v.join("\r\n")
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
            Content::Swiss(b) => swiss_payload(b),
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
    if st.cross {
        draw_swiss_cross(&mut img, size, w as u32 * scale);
    }
    Ok((img.into_raw(), size))
}

fn fill_rect(img: &mut RgbaImage, x0: i64, y0: i64, w: i64, h: i64, c: [u8; 3]) {
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let px = Rgba([c[0], c[1], c[2], 255]);
    for y in y0.max(0)..(y0 + h).min(ih) {
        for x in x0.max(0)..(x0 + w).min(iw) {
            img.put_pixel(x as u32, y as u32, px);
        }
    }
}

/// Schweizer Kreuz (weisses Kreuz auf schwarzem Quadrat, mit weissem Rand) in
/// die Mitte zeichnen – wie für die Swiss QR Bill vorgeschrieben.
fn draw_swiss_cross(img: &mut RgbaImage, size: u32, codearea: u32) {
    let cs = ((codearea as i64 * 152 / 1000).max(14)) & !1; // ~7mm von 46mm
    let b = (cs / 12).max(2);
    let cx = size as i64 / 2;
    let cy = size as i64 / 2;
    let ws = cs + 2 * b;
    fill_rect(img, cx - ws / 2, cy - ws / 2, ws, ws, [255, 255, 255]);
    fill_rect(img, cx - cs / 2, cy - cs / 2, cs, cs, [0, 0, 0]);
    let t = ((cs * 20 / 100).max(2)) & !1;
    let l = (cs * 66 / 100) & !1;
    fill_rect(img, cx - l / 2, cy - t / 2, l, t, [255, 255, 255]);
    fill_rect(img, cx - t / 2, cy - l / 2, t, l, [255, 255, 255]);
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
    let mut s = code
        .render::<svg::Color>()
        .min_dimensions(px, px)
        .quiet_zone(st.quiet > 0)
        .dark_color(svg::Color(&hex(st.fg)))
        .light_color(svg::Color(&hex(st.bg)))
        .build();
    if st.cross {
        let overlay = svg_cross(px as f64, (w * st.scale.max(1)) as f64);
        if let Some(pos) = s.rfind("</svg>") {
            s.insert_str(pos, &overlay);
        }
    }
    Ok(s)
}

fn svg_cross(px: f64, codearea: f64) -> String {
    let cs = (codearea * 0.152).max(14.0);
    let b = (cs / 12.0).max(2.0);
    let c = px / 2.0;
    let ws = cs + 2.0 * b;
    let t = (cs * 0.20).max(2.0);
    let l = cs * 0.66;
    format!(
        "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"#ffffff\"/>\
         <rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"#000000\"/>\
         <rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"#ffffff\"/>\
         <rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"#ffffff\"/>",
        c - ws / 2.0, c - ws / 2.0, ws, ws,
        c - cs / 2.0, c - cs / 2.0, cs, cs,
        c - l / 2.0, c - t / 2.0, l, t,
        c - t / 2.0, c - l / 2.0, t, l,
    )
}
