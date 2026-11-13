//! Befehle (aus Menüs, Knöpfen und Tasten), die Tastenbelegung wie in VLC, Wechsel durch
//! Seitenverhältnis/Zuschnitt/Deinterlace, Geschwindigkeitsstufen, Bildgeometrie und das
//! Verschieben in der Wiedergabeliste.
//!
//! Ohne egui und ohne libVLC, damit es sich einzeln prüfen lässt:
//! `rustc --edition 2021 --test src/befehl.rs -o /tmp/befehl && /tmp/befehl`

#![allow(dead_code)]

/// Was ein Klick oder eine Taste auslöst; ausgeführt nach dem Zeichnen.
#[derive(Clone, Debug, PartialEq)]
pub enum Aktion {
    // Medien
    Oeffnen,
    MehrereOeffnen,
    OrdnerOeffnen,
    NetzwerkDialog,
    ZuletztOeffnen(usize),
    ZuletztLeeren,
    ListeSpeichern,
    Beenden,
    // Wiedergabeliste
    Zeigen(usize),
    Entfernen(usize),
    Leeren,
    /// Eintrag `von` vor die Stelle `vor` (in der Liste vor dem Verschieben) setzen.
    Verschieben(usize, usize),
    // Wiedergabe
    PlayPause,
    Stopp,
    Zurueck,
    Weiter,
    Spulen(i64),
    Springen(i64),
    ZeitDialog,
    Schneller,
    Langsamer,
    TempoNormal,
    Tempo(f32),
    BildFuerBild,
    ZeitZeigen,
    Titel(i32),
    Kapitel(i32),
    WdhWechseln,
    Wdh(u8),
    ZufallUmschalten,
    // Audio
    Lauter,
    Leiser,
    Stumm,
    TonspurWechseln,
    Tonspur(i32),
    Audiogeraet(String),
    StereoModus(i32),
    // Video
    Vollbild,
    VollbildAus,
    Seitenverhaeltnis(usize),
    SeitenWechseln,
    Zuschnitt(usize),
    ZuschnittWechseln,
    Deinterlace(usize),
    DeinterlaceWechseln,
    Schnappschuss,
    // Untertitel
    Untertitel(i32),
    UntertitelWechseln,
    UntertitelDatei,
    /// Verzögerung ändern (in ms)
    UntertitelVerzoegerung(i64),
    UntertitelVerzoegerungNull,
    // Werkzeuge
    Effekte,
    Medieninfo,
    Einstellungen,
    // Ansicht
    ListeUmschalten,
    Minimal,
    ImmerOben,
    // Hilfe
    Tastenkuerzel,
    Ueber,
    // Bilder
    Diashow,
    Drehen(u8),
    Einpassen,
    Papierkorb,
    /// Entf: in der Liste den gewählten Eintrag entfernen, sonst Bild in den Papierkorb.
    EntfTaste,
}

/// Taste (unabhängig von egui).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taste {
    Leer,
    Links,
    Rechts,
    Hoch,
    Runter,
    Esc,
    Entf,
    Plus,
    Minus,
    Gleich,
    F1,
    /// Buchstabe A–Z (gross)
    Buchstabe(char),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    /// Strg (unter macOS die Befehlstaste)
    pub strg: bool,
    pub alt: bool,
}

/// Sprungweiten wie in VLC: sehr kurz 3 s, kurz 10 s, mittel 1 min, lang 5 min.
pub const SPRUNG_SEHR_KURZ: i64 = 3_000;
pub const SPRUNG_KURZ: i64 = 10_000;
pub const SPRUNG_MITTEL: i64 = 60_000;
pub const SPRUNG_LANG: i64 = 300_000;
/// Untertitel-Verzögerung pro Tastendruck (G/H)
pub const SPU_SCHRITT_MS: i64 = 50;

/// Tastenbelegung wie VLC (siehe Hilfe › Tastenkürzel). `bild`: ein Bild wird gezeigt
/// (← / → blättern statt spulen).
pub fn aus_taste(k: Taste, m: Mods, bild: bool) -> Option<Aktion> {
    use Aktion as A;
    // + / - / = kommen je nach Tastatur mit oder ohne Umschalt (Schweizer Tastatur: «+» ist Umschalt+1).
    if !m.strg && !m.alt {
        match k {
            Taste::Plus => return Some(A::Schneller),
            Taste::Minus => return Some(A::Langsamer),
            Taste::Gleich => return Some(A::TempoNormal),
            _ => {}
        }
    }
    let sprung = |ms: i64, vor: bool| Some(A::Spulen(if vor { ms } else { -ms }));
    match (m.strg, m.alt, m.shift) {
        // Strg+Alt
        (true, true, false) => match k {
            Taste::Links => sprung(SPRUNG_LANG, false),
            Taste::Rechts => sprung(SPRUNG_LANG, true),
            _ => None,
        },
        // Strg+Umschalt
        (true, false, true) => match k {
            Taste::Buchstabe('O') => Some(A::MehrereOeffnen),
            _ => None,
        },
        // Strg
        (true, false, false) => match k {
            Taste::Links => sprung(SPRUNG_MITTEL, false),
            Taste::Rechts => sprung(SPRUNG_MITTEL, true),
            Taste::Hoch => Some(A::Lauter),
            Taste::Runter => Some(A::Leiser),
            Taste::Buchstabe(c) => match c {
                'O' => Some(A::Oeffnen),
                'F' => Some(A::OrdnerOeffnen),
                'N' => Some(A::NetzwerkDialog),
                'Y' => Some(A::ListeSpeichern),
                'L' => Some(A::ListeUmschalten),
                'T' => Some(A::ZeitDialog),
                'H' => Some(A::Minimal),
                'E' => Some(A::Effekte),
                'I' => Some(A::Medieninfo),
                'P' => Some(A::Einstellungen),
                'Q' => Some(A::Beenden),
                _ => None,
            },
            _ => None,
        },
        // Alt: kurzer Sprung (wie ohne Zusatztaste)
        (false, true, false) => match k {
            Taste::Links if !bild => sprung(SPRUNG_KURZ, false),
            Taste::Rechts if !bild => sprung(SPRUNG_KURZ, true),
            _ => None,
        },
        // Umschalt
        (false, false, true) => match k {
            Taste::Links if !bild => sprung(SPRUNG_SEHR_KURZ, false),
            Taste::Rechts if !bild => sprung(SPRUNG_SEHR_KURZ, true),
            Taste::Buchstabe('S') => Some(A::Schnappschuss),
            _ => None,
        },
        // ohne Zusatztaste
        (false, false, false) => match k {
            Taste::Leer => Some(A::PlayPause),
            Taste::Links => if bild { Some(A::Zurueck) } else { sprung(SPRUNG_KURZ, false) },
            Taste::Rechts => if bild { Some(A::Weiter) } else { sprung(SPRUNG_KURZ, true) },
            Taste::Hoch => Some(A::Lauter),
            Taste::Runter => Some(A::Leiser),
            Taste::Esc => Some(A::VollbildAus),
            Taste::Entf => Some(A::EntfTaste),
            Taste::F1 => Some(A::Tastenkuerzel),
            Taste::Buchstabe(c) => match c {
                'F' => Some(A::Vollbild),
                'S' => Some(A::Stopp),
                'N' => Some(A::Weiter),
                'P' => Some(A::Zurueck),
                'M' => Some(A::Stumm),
                'E' => Some(A::BildFuerBild),
                'T' => Some(A::ZeitZeigen),
                'V' => Some(A::UntertitelWechseln),
                'B' => Some(A::TonspurWechseln),
                'A' => Some(A::SeitenWechseln),
                'C' => Some(A::ZuschnittWechseln),
                'D' => Some(A::DeinterlaceWechseln),
                'G' => Some(A::UntertitelVerzoegerung(-SPU_SCHRITT_MS)),
                'H' => Some(A::UntertitelVerzoegerung(SPU_SCHRITT_MS)),
                'L' => Some(A::WdhWechseln),
                'R' => Some(A::ZufallUmschalten),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

// ------------------------------------------------------------------ Wechsellisten

/// Seitenverhältnisse wie im VLC-Menü («» = Standard, aus dem Video).
pub const SEITEN: [&str; 9] = ["", "16:9", "4:3", "1:1", "16:10", "2.21:1", "2.35:1", "2.39:1", "5:4"];
/// Zuschnitte wie im VLC-Menü («» = Standard, nichts abschneiden).
pub const ZUSCHNITTE: [&str; 11] = ["", "16:10", "16:9", "4:3", "1.85:1", "2.21:1", "2.35:1", "2.39:1", "5:3", "5:4", "1:1"];
/// Deinterlace-Verfahren von libVLC («» = aus).
pub const DEINTERLACE: [&str; 11] = ["", "blend", "bob", "discard", "linear", "mean", "x", "yadif", "yadif2x", "phosphor", "ivtc"];

/// Nächster Eintrag einer Liste mit `n` Einträgen (von hinten wieder vorne).
pub fn weiter_index(i: usize, n: usize) -> usize {
    if n == 0 {
        0
    } else {
        (i.saturating_add(1)) % n
    }
}

/// «16:9», «2.35:1» oder «1,85:1» → Breite/Höhe.
pub fn verhaeltnis(s: &str) -> Option<f64> {
    let (a, b) = s.trim().split_once(':')?;
    let a: f64 = a.trim().replace(',', ".").parse().ok()?;
    let b: f64 = b.trim().replace(',', ".").parse().ok()?;
    let r = a / b;
    (a > 0.0 && b > 0.0 && r.is_finite() && r > 0.01 && r < 100.0).then_some(r)
}

/// Anzeige eines Videobilds `w`×`h` mit Pixel-Seitenverhältnis `sar`: Seitenverhältnis der
/// Anzeige und der sichtbare Ausschnitt (u0, v0, u1, v1) nach Seitenverhältnis-Vorgabe und
/// Zuschnitt (Mitte bleibt).
pub fn geometrie(w: usize, h: usize, sar: f64, seiten: Option<f64>, zuschnitt: Option<f64>) -> (f64, [f32; 4]) {
    let sar = if sar.is_finite() && sar > 0.01 && sar < 100.0 { sar } else { 1.0 };
    let quelle = if w > 0 && h > 0 { w as f64 * sar / h as f64 } else { 16.0 / 9.0 };
    let basis = seiten.filter(|r| r.is_finite() && *r > 0.0).unwrap_or(quelle);
    match zuschnitt.filter(|r| r.is_finite() && *r > 0.0) {
        Some(c) if (c - basis).abs() > 1e-6 => {
            if basis > c {
                let f = (c / basis) as f32;
                let r = (1.0 - f) / 2.0;
                (c, [r, 0.0, 1.0 - r, 1.0])
            } else {
                let f = (basis / c) as f32;
                let r = (1.0 - f) / 2.0;
                (c, [0.0, r, 1.0, 1.0 - r])
            }
        }
        _ => (basis, [0.0, 0.0, 1.0, 1.0]),
    }
}

// ------------------------------------------------------------------ Geschwindigkeit

pub const TEMPI: [f32; 10] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0];

pub fn schneller(r: f32) -> f32 {
    TEMPI.iter().copied().find(|t| *t > r + 1e-3).unwrap_or(TEMPI[TEMPI.len() - 1])
}

pub fn langsamer(r: f32) -> f32 {
    TEMPI.iter().rev().copied().find(|t| *t < r - 1e-3).unwrap_or(TEMPI[0])
}

/// «1.5×»
pub fn tempo_text(r: f32) -> String {
    let s = format!("{r:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}×")
}

// ------------------------------------------------------------------ Liste umordnen

/// Ziel-Index, wenn Eintrag `von` vor die Stelle `vor` gesetzt wird (Stellen der Liste vor dem
/// Verschieben; `vor == len` = ans Ende). `None`, wenn sich nichts ändert oder ungültig.
pub fn verschieben_ziel(len: usize, von: usize, vor: usize) -> Option<usize> {
    if von >= len || vor > len {
        return None;
    }
    let nach = if vor > von { vor - 1 } else { vor };
    (nach != von).then_some(nach)
}

/// Neuer Index eines Eintrags `i`, nachdem `von` nach `nach` verschoben wurde.
pub fn index_nach(i: usize, von: usize, nach: usize) -> usize {
    if i == von {
        nach
    } else if von < i && i <= nach {
        i - 1
    } else if nach <= i && i < von {
        i + 1
    } else {
        i
    }
}

/// Verschiebt in `v` und liefert den Ziel-Index.
pub fn verschieben<T>(v: &mut Vec<T>, von: usize, vor: usize) -> Option<usize> {
    let nach = verschieben_ziel(v.len(), von, vor)?;
    let e = v.remove(von);
    v.insert(nach.min(v.len()), e);
    Some(nach)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEINE: Mods = Mods { shift: false, strg: false, alt: false };

    fn m(shift: bool, strg: bool, alt: bool) -> Mods {
        Mods { shift, strg, alt }
    }

    #[test]
    fn tasten_wie_vlc() {
        assert_eq!(aus_taste(Taste::Leer, KEINE, false), Some(Aktion::PlayPause));
        assert_eq!(aus_taste(Taste::Rechts, KEINE, false), Some(Aktion::Spulen(10_000)));
        assert_eq!(aus_taste(Taste::Links, KEINE, false), Some(Aktion::Spulen(-10_000)));
        assert_eq!(aus_taste(Taste::Links, m(true, false, false), false), Some(Aktion::Spulen(-3_000)));
        assert_eq!(aus_taste(Taste::Rechts, m(false, false, true), false), Some(Aktion::Spulen(10_000)));
        assert_eq!(aus_taste(Taste::Rechts, m(false, true, false), false), Some(Aktion::Spulen(60_000)));
        assert_eq!(aus_taste(Taste::Links, m(false, true, true), false), Some(Aktion::Spulen(-300_000)));
        assert_eq!(aus_taste(Taste::Hoch, m(false, true, false), false), Some(Aktion::Lauter));
        assert_eq!(aus_taste(Taste::Runter, KEINE, false), Some(Aktion::Leiser));
        assert_eq!(aus_taste(Taste::Buchstabe('S'), KEINE, false), Some(Aktion::Stopp));
        assert_eq!(aus_taste(Taste::Buchstabe('S'), m(true, false, false), false), Some(Aktion::Schnappschuss));
        assert_eq!(aus_taste(Taste::Buchstabe('O'), m(false, true, false), false), Some(Aktion::Oeffnen));
        assert_eq!(aus_taste(Taste::Buchstabe('O'), m(true, true, false), false), Some(Aktion::MehrereOeffnen));
        assert_eq!(aus_taste(Taste::Buchstabe('Q'), m(false, true, false), false), Some(Aktion::Beenden));
        assert_eq!(aus_taste(Taste::Buchstabe('H'), m(false, true, false), false), Some(Aktion::Minimal));
        assert_eq!(aus_taste(Taste::Buchstabe('H'), KEINE, false), Some(Aktion::UntertitelVerzoegerung(50)));
        assert_eq!(aus_taste(Taste::Buchstabe('G'), KEINE, false), Some(Aktion::UntertitelVerzoegerung(-50)));
        assert_eq!(aus_taste(Taste::Buchstabe('X'), KEINE, false), None);
        assert_eq!(aus_taste(Taste::Esc, KEINE, false), Some(Aktion::VollbildAus));
    }

    #[test]
    fn plus_minus_auch_mit_umschalt() {
        assert_eq!(aus_taste(Taste::Plus, m(true, false, false), false), Some(Aktion::Schneller));
        assert_eq!(aus_taste(Taste::Plus, KEINE, false), Some(Aktion::Schneller));
        assert_eq!(aus_taste(Taste::Minus, KEINE, false), Some(Aktion::Langsamer));
        assert_eq!(aus_taste(Taste::Gleich, m(true, false, false), false), Some(Aktion::TempoNormal));
        assert_eq!(aus_taste(Taste::Plus, m(false, true, false), false), None);
    }

    #[test]
    fn bildmodus_blaettert() {
        assert_eq!(aus_taste(Taste::Rechts, KEINE, true), Some(Aktion::Weiter));
        assert_eq!(aus_taste(Taste::Links, KEINE, true), Some(Aktion::Zurueck));
        assert_eq!(aus_taste(Taste::Links, m(true, false, false), true), None);
        assert_eq!(aus_taste(Taste::Entf, KEINE, true), Some(Aktion::EntfTaste));
    }

    #[test]
    fn wechseln() {
        assert_eq!(weiter_index(0, 9), 1);
        assert_eq!(weiter_index(8, 9), 0);
        assert_eq!(weiter_index(5, 0), 0);
        assert_eq!(weiter_index(usize::MAX, 3), 0);
    }

    #[test]
    fn verhaeltnisse() {
        assert!((verhaeltnis("16:9").unwrap() - 16.0 / 9.0).abs() < 1e-9);
        assert!((verhaeltnis("2.35:1").unwrap() - 2.35).abs() < 1e-9);
        assert!((verhaeltnis("1,85:1").unwrap() - 1.85).abs() < 1e-9);
        assert_eq!(verhaeltnis(""), None);
        assert_eq!(verhaeltnis("4:0"), None);
        assert_eq!(verhaeltnis("x:1"), None);
        for s in SEITEN.iter().chain(ZUSCHNITTE.iter()).filter(|s| !s.is_empty()) {
            assert!(verhaeltnis(s).is_some(), "{s}");
        }
    }

    #[test]
    fn geometrien() {
        // Standard: 1920×1080 quadratische Pixel
        let (r, uv) = geometrie(1920, 1080, 1.0, None, None);
        assert!((r - 16.0 / 9.0).abs() < 1e-9);
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
        // DVD 720×576 mit SAR 64:45 → 16:9
        let (r, _) = geometrie(720, 576, 64.0 / 45.0, None, None);
        assert!((r - 16.0 / 9.0).abs() < 1e-6);
        // Vorgabe 4:3
        let (r, _) = geometrie(1920, 1080, 1.0, Some(4.0 / 3.0), None);
        assert!((r - 4.0 / 3.0).abs() < 1e-9);
        // 16:9 auf 4:3 zuschneiden: links und rechts je 1/8 weg
        let (r, uv) = geometrie(1920, 1080, 1.0, None, Some(4.0 / 3.0));
        assert!((r - 4.0 / 3.0).abs() < 1e-9);
        assert!((uv[0] - 0.125).abs() < 1e-6 && (uv[2] - 0.875).abs() < 1e-6);
        assert_eq!((uv[1], uv[3]), (0.0, 1.0));
        // 16:9 auf 2.35:1 zuschneiden: oben und unten weg
        let (_, uv) = geometrie(1920, 1080, 1.0, None, Some(2.35));
        assert_eq!((uv[0], uv[2]), (0.0, 1.0));
        assert!(uv[1] > 0.1 && uv[3] < 0.9);
        // Unsinn bleibt harmlos
        let (r, uv) = geometrie(0, 0, f64::NAN, Some(f64::INFINITY), Some(-1.0));
        assert!(r.is_finite() && r > 0.0);
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn tempi() {
        assert_eq!(schneller(1.0), 1.25);
        assert_eq!(schneller(4.0), 4.0);
        assert_eq!(langsamer(1.0), 0.75);
        assert_eq!(langsamer(0.25), 0.25);
        assert_eq!(schneller(1.1), 1.25);
        assert_eq!(tempo_text(1.0), "1×");
        assert_eq!(tempo_text(1.25), "1.25×");
        assert_eq!(tempo_text(0.5), "0.5×");
    }

    #[test]
    fn umordnen() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        assert_eq!(verschieben(&mut v, 0, 3), Some(2));
        assert_eq!(v, ['b', 'c', 'a', 'd']);
        assert_eq!(verschieben(&mut v, 3, 0), Some(0));
        assert_eq!(v, ['d', 'b', 'c', 'a']);
        assert_eq!(verschieben(&mut v, 1, 4), Some(3));
        assert_eq!(v, ['d', 'c', 'a', 'b']);
        assert_eq!(verschieben(&mut v, 1, 1), None);
        assert_eq!(verschieben(&mut v, 1, 2), None);
        assert_eq!(verschieben(&mut v, 9, 0), None);
        assert_eq!(verschieben(&mut v, 0, 9), None);
        // Index des laufenden Eintrags folgt
        assert_eq!(index_nach(2, 0, 3), 1);
        assert_eq!(index_nach(0, 0, 3), 3);
        assert_eq!(index_nach(1, 3, 0), 2);
        assert_eq!(index_nach(5, 0, 3), 5);
        // gegen Nachbau prüfen
        for len in 1..6 {
            for von in 0..len {
                for vor in 0..=len {
                    let mut v: Vec<usize> = (0..len).collect();
                    if let Some(nach) = verschieben(&mut v, von, vor) {
                        for i in 0..len {
                            assert_eq!(v[index_nach(i, von, nach)], i, "len {len} von {von} vor {vor}");
                        }
                    }
                }
            }
        }
    }
}
