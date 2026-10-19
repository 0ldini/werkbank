//! Gemeinsame Bedienung der eigenen Werkbank-Programme (Vorlage: eigene/stil.rs, in allen
//! Programmen gleich). Aufbau jedes Programms:
//!
//! - **Kopfleiste** (`kopfleiste`): Logo und Name, dann Aktionen in Gruppen (`knopf`, `menue`,
//!   `trenner`), rechts leise Symbol-Knöpfe (`leise`).
//! - **Auswahlleiste** (`auswahlleiste`): nur sichtbar, wenn etwas gewählt ist; leicht getönt.
//! - **Einstellungen rechts** (`seitenleiste` mit `karte`): aufklappbare Karten mit Symbol,
//!   darin `feld` (Beschriftung über dem Feld), `schalter`, `schieber` (mit Wert), `wahl`.
//! - **Fussleiste** (`fussleiste`): links die Zusammenfassung, rechts Ziel und der eine
//!   Hauptknopf (`haupt`) in der Logofarbe.
//!
//! Beim Start nach `thema::apply` einmal `stil::init(ctx, [r, g, b])` mit der Logofarbe.

#![allow(dead_code)]

use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::thema;

static FARBE: AtomicU32 = AtomicU32::new(0x2F6FDE);

/// Logofarbe des Programms.
pub fn farbe() -> Color32 {
    let v = FARBE.load(Ordering::Relaxed);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// `c` zu Anteil `a` über `base` gelegt.
pub fn mix(c: Color32, base: Color32, a: f32) -> Color32 {
    let m = |x: u8, b: u8| (b as f32 + (x as f32 - b as f32) * a).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(c.r(), base.r()), m(c.g(), base.g()), m(c.b(), base.b()))
}

/// Logofarbe für Schrift und Symbole: im dunklen Design etwas heller.
pub fn akzent() -> Color32 {
    let c = farbe();
    if thema::is_dark() {
        mix(c, Color32::WHITE, 0.72)
    } else {
        c
    }
}

/// Getönter Grund in der Logofarbe (Auswahl, Auswahlleiste).
pub fn toenung() -> Color32 {
    mix(farbe(), thema::panel(), if thema::is_dark() { 0.22 } else { 0.11 })
}

pub fn rot() -> Color32 {
    thema::pick(Color32::from_rgb(0xC8, 0x10, 0x2E), Color32::from_rgb(0xF8, 0x71, 0x71))
}
pub fn gruen() -> Color32 {
    thema::pick(Color32::from_rgb(0x0F, 0x76, 0x6E), Color32::from_rgb(0x5E, 0xEA, 0xD4))
}
pub fn orange() -> Color32 {
    thema::pick(Color32::from_rgb(0xB4, 0x53, 0x09), Color32::from_rgb(0xFB, 0xBF, 0x24))
}
pub fn violett() -> Color32 {
    thema::pick(Color32::from_rgb(0x6D, 0x49, 0xE0), Color32::from_rgb(0xA7, 0x8B, 0xFA))
}
pub fn blau() -> Color32 {
    thema::pick(Color32::from_rgb(0x25, 0x63, 0xEB), Color32::from_rgb(0x60, 0xA5, 0xFA))
}

/// Logofarbe festlegen und den egui-Stil der Werkbank setzen (Abstände, Ecken, Auswahl).
pub fn init(ctx: &egui::Context, logo: [u8; 3]) {
    FARBE.store((logo[0] as u32) << 16 | (logo[1] as u32) << 8 | logo[2] as u32, Ordering::Relaxed);
    let f = farbe();
    let dark = thema::is_dark();
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 28.0;
        let r = egui::Rounding::same(8.0);
        let v = &mut s.visuals;
        v.widgets.noninteractive.rounding = r;
        v.widgets.inactive.rounding = r;
        v.widgets.hovered.rounding = r;
        v.widgets.active.rounding = r;
        v.widgets.open.rounding = r;
        v.widgets.inactive.weak_bg_fill = thema::panel();
        v.widgets.inactive.bg_fill = thema::pick(Color32::from_rgb(0xE6, 0xE8, 0xEC), Color32::from_rgb(0x32, 0x35, 0x3D));
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, thema::border());
        v.widgets.hovered.weak_bg_fill = thema::hover();
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, mix(f, thema::border(), 0.45));
        v.widgets.active.weak_bg_fill = toenung();
        v.selection.bg_fill = if dark { mix(f, Color32::BLACK, 0.75) } else { f };
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = akzent();
        v.window_rounding = egui::Rounding::same(12.0);
        v.menu_rounding = egui::Rounding::same(10.0);
    });
}

// ------------------------------------------------------------------------------- Symbole

/// Gezeichnete Symbole (24 × 24-Raster), unabhängig von der Schrift.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sym {
    Plus,
    Ordner,
    OrdnerAuf,
    Datei,
    Sortieren,
    Mischen,
    Play,
    Pause,
    Stopp,
    Auge,
    Mehr,
    Hoch,
    Runter,
    Pfeil,
    PfeilRunter,
    Anfang,
    Ende,
    Papierkorb,
    Bild,
    Film,
    Musik,
    Panorama,
    Uebergang,
    Text,
    Haken,
    Kreuz,
    Zahnrad,
    Suche,
    Rueckgaengig,
    Umwandeln,
    Archiv,
    Umbenennen,
    Download,
    Upload,
    Speichern,
    Aktualisieren,
    Mail,
    Senden,
    Kalender,
    Uhr,
    Stern,
    Stift,
    Link,
    Extern,
    Ebenen,
    Kopieren,
    Schild,
    Schluessel,
    Kamera,
    Aufnahme,
    Raster,
    Diagramm,
    Info,
    Warnung,
    Zeit,
}

/// Zeichnet ein Symbol in `r` in der Farbe `c`.
pub fn sym(p: &egui::Painter, r: Rect, s: Sym, c: Color32) {
    let st = Stroke::new((r.width() / 24.0 * 1.8).max(1.2), c);
    let u = r.width() / 24.0;
    let at = |x: f32, y: f32| Pos2::new(r.left() + x * u, r.top() + y * u);
    let line = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), st));
    };
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32, rr: f32| {
        p.rect_stroke(Rect::from_min_max(at(x0, y0), at(x1, y1)), rr * u, st);
    };
    let circle = |x: f32, y: f32, rad: f32| {
        p.circle_stroke(at(x, y), rad * u, st);
    };
    let arc = |cx: f32, cy: f32, rad: f32, a0: f32, a1: f32| {
        let n = (((a1 - a0).abs() / 12.0).ceil() as usize).max(2);
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                at(cx + rad * a.cos(), cy + rad * a.sin())
            })
            .collect();
        p.add(egui::Shape::line(pts, st));
    };
    match s {
        Sym::Plus => {
            line(&[(12.0, 5.0), (12.0, 19.0)]);
            line(&[(5.0, 12.0), (19.0, 12.0)]);
        }
        Sym::Ordner => line(&[(3.0, 7.0), (3.0, 19.0), (21.0, 19.0), (21.0, 8.5), (11.0, 8.5), (9.0, 6.0), (3.0, 6.0), (3.0, 7.0)]),
        Sym::OrdnerAuf => {
            line(&[(3.0, 19.0), (3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (19.0, 8.0), (19.0, 11.0)]);
            line(&[(3.0, 19.0), (6.0, 11.0), (22.0, 11.0), (19.0, 19.0), (3.0, 19.0)]);
        }
        Sym::Datei => {
            line(&[(6.0, 3.0), (14.0, 3.0), (18.0, 7.0), (18.0, 21.0), (6.0, 21.0), (6.0, 3.0)]);
            line(&[(14.0, 3.0), (14.0, 7.0), (18.0, 7.0)]);
        }
        Sym::Sortieren => {
            line(&[(4.0, 6.0), (14.0, 6.0)]);
            line(&[(4.0, 12.0), (11.0, 12.0)]);
            line(&[(4.0, 18.0), (8.0, 18.0)]);
            line(&[(17.0, 5.0), (17.0, 19.0)]);
            line(&[(14.0, 16.0), (17.0, 19.0), (20.0, 16.0)]);
        }
        Sym::Mischen => {
            line(&[(4.0, 7.0), (8.0, 7.0), (15.0, 17.0), (20.0, 17.0)]);
            line(&[(4.0, 17.0), (8.0, 17.0), (15.0, 7.0), (20.0, 7.0)]);
            line(&[(17.5, 4.5), (20.0, 7.0), (17.5, 9.5)]);
            line(&[(17.5, 14.5), (20.0, 17.0), (17.5, 19.5)]);
        }
        Sym::Play => {
            p.add(egui::Shape::convex_polygon(vec![at(8.0, 5.5), at(18.5, 12.0), at(8.0, 18.5)], c, Stroke::NONE));
        }
        Sym::Pause => {
            p.rect_filled(Rect::from_min_max(at(7.0, 5.5), at(10.5, 18.5)), 1.0, c);
            p.rect_filled(Rect::from_min_max(at(13.5, 5.5), at(17.0, 18.5)), 1.0, c);
        }
        Sym::Stopp => {
            p.rect_filled(Rect::from_min_max(at(6.5, 6.5), at(17.5, 17.5)), 2.0 * u, c);
        }
        Sym::Auge => {
            arc(12.0, 19.0, 11.0, 220.0, 320.0);
            arc(12.0, 5.0, 11.0, 40.0, 140.0);
            circle(12.0, 12.0, 3.0);
        }
        Sym::Mehr => {
            for x in [6.0, 12.0, 18.0] {
                p.circle_filled(at(x, 12.0), 1.5 * u, c);
            }
        }
        Sym::Hoch => line(&[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]),
        Sym::Runter => line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Sym::Pfeil => {
            line(&[(12.0, 19.0), (12.0, 5.0)]);
            line(&[(6.0, 11.0), (12.0, 5.0), (18.0, 11.0)]);
        }
        Sym::PfeilRunter => {
            line(&[(12.0, 5.0), (12.0, 19.0)]);
            line(&[(6.0, 13.0), (12.0, 19.0), (18.0, 13.0)]);
        }
        Sym::Anfang => {
            line(&[(5.0, 4.0), (19.0, 4.0)]);
            line(&[(12.0, 20.0), (12.0, 9.0)]);
            line(&[(6.0, 14.0), (12.0, 8.0), (18.0, 14.0)]);
        }
        Sym::Ende => {
            line(&[(5.0, 20.0), (19.0, 20.0)]);
            line(&[(12.0, 4.0), (12.0, 15.0)]);
            line(&[(6.0, 10.0), (12.0, 16.0), (18.0, 10.0)]);
        }
        Sym::Papierkorb => {
            line(&[(4.0, 7.0), (20.0, 7.0)]);
            line(&[(9.0, 7.0), (9.0, 4.0), (15.0, 4.0), (15.0, 7.0)]);
            line(&[(6.0, 7.0), (7.0, 20.0), (17.0, 20.0), (18.0, 7.0)]);
        }
        Sym::Bild => {
            rect(3.5, 5.0, 20.5, 19.0, 2.0);
            circle(9.0, 10.0, 1.6);
            line(&[(4.0, 17.0), (9.0, 12.0), (13.0, 16.0), (15.5, 13.5), (20.0, 17.0)]);
        }
        Sym::Film => {
            rect(3.5, 5.0, 20.5, 19.0, 2.0);
            line(&[(7.0, 5.0), (7.0, 19.0)]);
            line(&[(17.0, 5.0), (17.0, 19.0)]);
            line(&[(3.5, 9.5), (7.0, 9.5)]);
            line(&[(3.5, 14.5), (7.0, 14.5)]);
            line(&[(17.0, 9.5), (20.5, 9.5)]);
            line(&[(17.0, 14.5), (20.5, 14.5)]);
        }
        Sym::Musik => {
            line(&[(9.0, 18.0), (9.0, 6.0), (19.0, 4.0), (19.0, 16.0)]);
            circle(7.0, 18.0, 2.0);
            circle(17.0, 16.0, 2.0);
        }
        Sym::Panorama => {
            arc(12.0, -12.0, 21.0, 55.0, 125.0);
            arc(12.0, 36.0, 21.0, 235.0, 305.0);
            line(&[(3.0, 5.5), (3.0, 18.5)]);
            line(&[(21.0, 5.5), (21.0, 18.5)]);
        }
        Sym::Uebergang => {
            rect(3.5, 6.0, 12.5, 18.0, 1.5);
            rect(11.5, 6.0, 20.5, 18.0, 1.5);
        }
        Sym::Text => {
            line(&[(5.0, 6.0), (19.0, 6.0)]);
            line(&[(12.0, 6.0), (12.0, 19.0)]);
            line(&[(9.0, 19.0), (15.0, 19.0)]);
        }
        Sym::Haken => line(&[(5.0, 12.5), (10.0, 17.5), (19.0, 7.0)]),
        Sym::Kreuz => {
            line(&[(6.0, 6.0), (18.0, 18.0)]);
            line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Sym::Zahnrad => {
            circle(12.0, 12.0, 3.0);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                line(&[(12.0 + a.cos() * 6.0, 12.0 + a.sin() * 6.0), (12.0 + a.cos() * 9.5, 12.0 + a.sin() * 9.5)]);
            }
        }
        Sym::Suche => {
            circle(11.0, 11.0, 6.0);
            line(&[(15.5, 15.5), (20.0, 20.0)]);
        }
        Sym::Rueckgaengig => {
            line(&[(9.0, 14.0), (4.0, 9.0), (9.0, 4.0)]);
            line(&[(4.0, 9.0), (14.0, 9.0)]);
            arc(14.0, 14.5, 5.5, -90.0, 90.0);
            line(&[(14.0, 20.0), (11.0, 20.0)]);
        }
        Sym::Umwandeln => {
            line(&[(4.0, 8.0), (18.0, 8.0)]);
            line(&[(15.0, 5.0), (18.0, 8.0), (15.0, 11.0)]);
            line(&[(20.0, 16.0), (6.0, 16.0)]);
            line(&[(9.0, 13.0), (6.0, 16.0), (9.0, 19.0)]);
        }
        Sym::Archiv => {
            rect(3.5, 4.0, 20.5, 9.0, 1.2);
            line(&[(5.0, 9.0), (5.0, 19.0), (19.0, 19.0), (19.0, 9.0)]);
            line(&[(10.0, 13.0), (14.0, 13.0)]);
        }
        Sym::Umbenennen | Sym::Stift => {
            line(&[(4.0, 20.0), (8.0, 20.0), (19.0, 9.0), (15.0, 5.0), (4.0, 16.0), (4.0, 20.0)]);
            line(&[(13.5, 6.5), (17.5, 10.5)]);
        }
        Sym::Download => {
            line(&[(12.0, 4.0), (12.0, 15.0)]);
            line(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]);
            line(&[(5.0, 20.0), (19.0, 20.0)]);
        }
        Sym::Upload => {
            line(&[(12.0, 15.0), (12.0, 4.0)]);
            line(&[(7.0, 9.0), (12.0, 4.0), (17.0, 9.0)]);
            line(&[(5.0, 20.0), (19.0, 20.0)]);
        }
        Sym::Speichern => {
            line(&[(4.0, 4.0), (16.0, 4.0), (20.0, 8.0), (20.0, 20.0), (4.0, 20.0), (4.0, 4.0)]);
            rect(8.0, 4.0, 15.0, 9.0, 0.5);
            rect(7.0, 13.0, 17.0, 20.0, 0.5);
        }
        Sym::Aktualisieren => {
            arc(12.0, 12.0, 8.0, -40.0, 250.0);
            line(&[(20.0, 4.0), (19.2, 8.0), (15.2, 7.2)]);
        }
        Sym::Mail => {
            rect(3.5, 5.5, 20.5, 18.5, 2.0);
            line(&[(4.0, 7.0), (12.0, 13.0), (20.0, 7.0)]);
        }
        Sym::Senden => {
            line(&[(3.0, 11.0), (21.0, 3.0), (14.0, 21.0), (11.0, 13.0), (3.0, 11.0)]);
            line(&[(11.0, 13.0), (21.0, 3.0)]);
        }
        Sym::Kalender => {
            rect(4.0, 5.0, 20.0, 20.0, 2.0);
            line(&[(4.0, 10.0), (20.0, 10.0)]);
            line(&[(8.0, 3.0), (8.0, 7.0)]);
            line(&[(16.0, 3.0), (16.0, 7.0)]);
        }
        Sym::Uhr | Sym::Zeit => {
            circle(12.0, 12.0, 8.5);
            line(&[(12.0, 7.5), (12.0, 12.0), (15.0, 14.0)]);
        }
        Sym::Stern => {
            let pts: Vec<(f32, f32)> = (0..=10)
                .map(|i| {
                    let a = (-90.0 + i as f32 * 36.0_f32).to_radians();
                    let rad = if i % 2 == 0 { 8.5 } else { 3.8 };
                    (12.0 + rad * a.cos(), 12.6 + rad * a.sin())
                })
                .collect();
            line(&pts);
        }
        Sym::Link => {
            arc(8.5, 15.5, 4.0, 15.0, 255.0);
            arc(15.5, 8.5, 4.0, 195.0, 435.0);
            line(&[(10.0, 14.0), (14.0, 10.0)]);
        }
        Sym::Extern => {
            line(&[(14.0, 4.0), (20.0, 4.0), (20.0, 10.0)]);
            line(&[(20.0, 4.0), (11.0, 13.0)]);
            line(&[(18.0, 14.0), (18.0, 20.0), (4.0, 20.0), (4.0, 6.0), (10.0, 6.0)]);
        }
        Sym::Ebenen => {
            line(&[(12.0, 4.0), (20.0, 8.0), (12.0, 12.0), (4.0, 8.0), (12.0, 4.0)]);
            line(&[(4.0, 12.0), (12.0, 16.0), (20.0, 12.0)]);
            line(&[(4.0, 16.0), (12.0, 20.0), (20.0, 16.0)]);
        }
        Sym::Kopieren => {
            rect(8.0, 8.0, 20.0, 20.0, 2.0);
            line(&[(16.0, 8.0), (16.0, 4.0), (4.0, 4.0), (4.0, 16.0), (8.0, 16.0)]);
        }
        Sym::Schild => {
            line(&[(12.0, 3.0), (19.0, 6.0), (19.0, 11.0), (17.0, 17.0), (12.0, 21.0), (7.0, 17.0), (5.0, 11.0), (5.0, 6.0), (12.0, 3.0)]);
            line(&[(9.0, 12.0), (11.0, 14.0), (15.0, 10.0)]);
        }
        Sym::Schluessel => {
            circle(8.0, 15.0, 4.5);
            line(&[(11.2, 11.8), (20.0, 3.0)]);
            line(&[(16.0, 7.0), (19.0, 10.0)]);
        }
        Sym::Kamera => {
            rect(3.0, 7.0, 21.0, 19.0, 2.0);
            line(&[(8.0, 7.0), (9.5, 4.5), (14.5, 4.5), (16.0, 7.0)]);
            circle(12.0, 13.0, 3.5);
        }
        Sym::Aufnahme => {
            circle(12.0, 12.0, 8.5);
            p.circle_filled(at(12.0, 12.0), 4.0 * u, c);
        }
        Sym::Raster => {
            for (x, y) in [(4.0, 4.0), (14.0, 4.0), (4.0, 14.0), (14.0, 14.0)] {
                rect(x, y, x + 6.0, y + 6.0, 1.5);
            }
        }
        Sym::Diagramm => {
            rect(9.0, 3.5, 15.0, 8.5, 1.0);
            rect(3.0, 15.5, 9.0, 20.5, 1.0);
            rect(15.0, 15.5, 21.0, 20.5, 1.0);
            line(&[(12.0, 8.5), (12.0, 12.0), (6.0, 12.0), (6.0, 15.5)]);
            line(&[(12.0, 12.0), (18.0, 12.0), (18.0, 15.5)]);
        }
        Sym::Info => {
            circle(12.0, 12.0, 8.5);
            line(&[(12.0, 11.0), (12.0, 16.5)]);
            p.circle_filled(at(12.0, 7.8), 1.1 * u, c);
        }
        Sym::Warnung => {
            line(&[(12.0, 4.0), (21.0, 19.5), (3.0, 19.5), (12.0, 4.0)]);
            line(&[(12.0, 10.0), (12.0, 14.0)]);
            p.circle_filled(at(12.0, 16.8), 1.1 * u, c);
        }
    }
}

/// Symbol als eigenes kleines Element (z. B. vor einem Text).
pub fn symbol(ui: &mut Ui, s: Sym, size: f32, c: Color32) -> Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    sym(ui.painter(), r, s, c);
    resp
}

// ------------------------------------------------------------------------------- Knöpfe

fn galley(ui: &Ui, text: &str, size: f32, c: Color32, strong: bool) -> std::sync::Arc<egui::Galley> {
    let font = if strong { egui::FontId::new(size, egui::FontFamily::Proportional) } else { egui::FontId::proportional(size) };
    ui.painter().layout_no_wrap(text.to_string(), font, c)
}

#[derive(Clone, Copy, PartialEq)]
enum Art {
    Normal,
    Leise,
    Haupt,
    Gefahr,
}

fn knopf_roh(ui: &mut Ui, s: Option<Sym>, text: &str, art: Art, enabled: bool, chevron: bool, hint: Option<&str>) -> Response {
    let f = farbe();
    let (fg, sym_c) = match art {
        Art::Haupt => (Color32::WHITE, Color32::WHITE),
        Art::Gefahr => (rot(), rot()),
        _ => (thema::text(), akzent()),
    };
    let (fg, sym_c) = if enabled { (fg, sym_c) } else { (thema::muted(), thema::muted()) };
    let g = (!text.is_empty()).then(|| galley(ui, text, if art == Art::Haupt { 14.0 } else { 13.5 }, fg, art == Art::Haupt));
    let h = if art == Art::Haupt { 34.0 } else { 30.0 };
    let pad = if text.is_empty() { 7.0 } else { 11.0 };
    let w = pad * 2.0 + s.map(|_| 17.0).unwrap_or(0.0) + g.as_ref().map(|g| g.size().x + if s.is_some() { 7.0 } else { 0.0 }).unwrap_or(0.0) + if chevron { 16.0 } else { 0.0 };
    let w = w.max(h);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), sense);
    let hov = enabled && resp.hovered();
    let p = ui.painter();
    match art {
        Art::Haupt => {
            let fill = if !enabled { mix(f, thema::panel(), 0.35) } else if hov { mix(f, Color32::BLACK, 0.88) } else { f };
            p.rect_filled(rect, 9.0, fill);
        }
        Art::Normal => {
            p.rect(rect, 8.0, if hov { thema::hover() } else { thema::panel() }, Stroke::new(1.0, if hov { mix(f, thema::border(), 0.45) } else { thema::border() }));
        }
        Art::Leise | Art::Gefahr => {
            if hov {
                p.rect_filled(rect, 8.0, thema::hover());
            }
        }
    }
    let mut x = rect.left() + pad;
    if let Some(s) = s {
        sym(p, Rect::from_center_size(Pos2::new(x + 8.5, rect.center().y), Vec2::splat(17.0)), s, sym_c);
        x += 17.0 + 7.0;
    }
    if let Some(g) = g {
        let y = rect.center().y - g.size().y / 2.0;
        p.galley(Pos2::new(x, y), g.clone(), fg);
        x += g.size().x;
    }
    if chevron {
        sym(p, Rect::from_center_size(Pos2::new(x + 9.0, rect.center().y), Vec2::splat(13.0)), Sym::Runter, thema::muted());
    }
    let resp = if let Some(h) = hint { resp.on_hover_text(h) } else { resp };
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Normaler Knopf: Symbol in der Logofarbe plus Wort, mit Rahmen.
pub fn knopf(ui: &mut Ui, s: Sym, text: &str) -> Response {
    knopf_roh(ui, Some(s), text, Art::Normal, true, false, None)
}

/// Wie `knopf`, aber ggf. ausgegraut.
pub fn knopf_an(ui: &mut Ui, enabled: bool, s: Sym, text: &str) -> Response {
    knopf_roh(ui, Some(s), text, Art::Normal, enabled, false, None)
}

/// Knopf nur mit Wort (ohne Symbol), mit Rahmen.
pub fn knopf_text(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    knopf_roh(ui, None, text, Art::Normal, enabled, false, None)
}

/// Leiser Knopf: Symbol (und optional Wort) ohne Rahmen; `hinweis` erscheint beim Darüberfahren.
pub fn leise(ui: &mut Ui, enabled: bool, s: Sym, text: &str, hinweis: &str) -> Response {
    knopf_roh(ui, Some(s), text, Art::Leise, enabled, false, (!hinweis.is_empty()).then_some(hinweis))
}

/// Leiser Knopf in Rot (Entfernen, Löschen).
pub fn gefahr(ui: &mut Ui, enabled: bool, s: Sym, text: &str) -> Response {
    knopf_roh(ui, Some(s), text, Art::Gefahr, enabled, false, None)
}

/// Der eine Hauptknopf (gefüllt in der Logofarbe), in der Fussleiste rechts.
pub fn haupt(ui: &mut Ui, enabled: bool, s: Sym, text: &str) -> Response {
    knopf_roh(ui, Some(s), text, Art::Haupt, enabled, false, None)
}

/// Knopf mit Klappmenü darunter.
pub fn menue(ui: &mut Ui, enabled: bool, s: Sym, text: &str, add: impl FnOnce(&mut Ui)) -> Response {
    let resp = knopf_roh(ui, Some(s), text, Art::Normal, enabled, true, None);
    let id = resp.id.with("menue");
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    egui::popup_below_widget(ui, id, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(200.0);
        add(ui);
    });
    resp
}

/// Eintrag in einem Klappmenü: Symbol plus Text, ganze Breite.
pub fn eintrag(ui: &mut Ui, enabled: bool, s: Sym, text: &str) -> Response {
    let w = ui.available_width().max(180.0);
    let g = galley(ui, text, 13.5, if enabled { thema::text() } else { thema::muted() }, false);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, thema::hover());
    }
    sym(ui.painter(), Rect::from_center_size(Pos2::new(rect.left() + 16.0, rect.center().y), Vec2::splat(16.0)), s, if enabled { akzent() } else { thema::muted() });
    ui.painter().galley(Pos2::new(rect.left() + 32.0, rect.center().y - g.size().y / 2.0), g, thema::text());
    resp
}

/// Senkrechter Trennstrich zwischen Knopfgruppen.
pub fn trenner(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(9.0, 22.0), Sense::hover());
    ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, thema::border()));
}

// ------------------------------------------------------------------------------- Leisten

fn logo_textur(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("werk-stil-logo");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([64, 64], include_bytes!("../icon.rgba"));
    let t = ctx.load_texture("werk-logo", img, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

/// Logo und Name links in der Kopfleiste.
pub fn titel(ui: &mut Ui, name: &str) {
    let t = logo_textur(ui.ctx());
    ui.add(egui::Image::new((t.id(), Vec2::splat(22.0))));
    ui.label(egui::RichText::new(name).size(15.5).strong().color(thema::text()));
    ui.add_space(4.0);
}

/// Kopfleiste oben: `add` füllt die Zeile (zuerst `titel`, dann Knöpfe; mit
/// `rechts(ui, |ui| …)` rechts ausgerichtet).
pub fn kopfleiste(ctx: &egui::Context, add: impl FnOnce(&mut Ui)) {
    egui::TopBottomPanel::top("werk-kopf")
        .frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::symmetric(12.0, 8.0)).stroke(Stroke::new(1.0, thema::border())))
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                add(ui);
            });
        });
}

/// Rest der Zeile rechtsbündig.
pub fn rechts(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        add(ui);
    });
}

/// Auswahlleiste unter der Kopfleiste (nur aufrufen, wenn etwas gewählt ist).
pub fn auswahlleiste(ctx: &egui::Context, anzahl_text: &str, add: impl FnOnce(&mut Ui)) {
    egui::TopBottomPanel::top("werk-auswahl")
        .frame(egui::Frame::none().fill(toenung()).inner_margin(egui::Margin::symmetric(12.0, 5.0)).stroke(Stroke::new(1.0, mix(farbe(), thema::border(), 0.3))))
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(egui::RichText::new(anzahl_text).strong().color(thema::text()));
                ui.add_space(6.0);
                add(ui);
            });
        });
}

/// Fussleiste unten: links Zusammenfassung, rechts (über `rechts`) Ziel und Hauptknopf.
pub fn fussleiste(ctx: &egui::Context, add: impl FnOnce(&mut Ui)) {
    egui::TopBottomPanel::bottom("werk-fuss")
        .frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::symmetric(14.0, 9.0)).stroke(Stroke::new(1.0, thema::border())))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            add(ui);
        });
}

/// Seitenleiste rechts für die Einstellungs-Karten (mit Rollbalken).
pub fn seitenleiste(ctx: &egui::Context, breite: f32, add: impl FnOnce(&mut Ui)) {
    egui::SidePanel::right("werk-einstellungen")
        .resizable(true)
        .default_width(breite)
        .min_width(breite * 0.8)
        .frame(egui::Frame::none().fill(thema::bg()).inner_margin(egui::Margin::same(10.0)).stroke(Stroke::new(1.0, thema::border())))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                add(ui);
            });
        });
}

/// Hauptbereich in der Grundfarbe.
pub fn mitte(ctx: &egui::Context, add: impl FnOnce(&mut Ui)) {
    egui::CentralPanel::default().frame(egui::Frame::none().fill(thema::panel()).inner_margin(egui::Margin::same(10.0))).show(ctx, add);
}

// ------------------------------------------------------------------------------- Inhalte

/// Aufklappbare Karte mit Symbol; der Zustand (offen/zu) bleibt während der Sitzung erhalten.
pub fn karte(ui: &mut Ui, id: &str, s: Sym, titel: &str, offen: bool, add: impl FnOnce(&mut Ui)) {
    karte_farbe(ui, id, s, akzent(), titel, offen, add)
}

/// Karte mit eigener Symbolfarbe.
pub fn karte_farbe(ui: &mut Ui, id: &str, s: Sym, c: Color32, titel: &str, offen: bool, add: impl FnOnce(&mut Ui)) {
    let key = egui::Id::new(("werk-karte", id));
    let mut auf = ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(offen);
    egui::Frame::none().fill(thema::panel()).stroke(Stroke::new(1.0, thema::border())).rounding(12.0).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        sym(ui.painter(), Rect::from_center_size(Pos2::new(rect.left() + 9.0, rect.center().y), Vec2::splat(17.0)), s, c);
        let g = galley(ui, titel, 14.0, thema::text(), true);
        ui.painter().galley(Pos2::new(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g, thema::text());
        sym(ui.painter(), Rect::from_center_size(Pos2::new(rect.right() - 8.0, rect.center().y), Vec2::splat(13.0)), if auf { Sym::Hoch } else { Sym::Runter }, thema::muted());
        if resp.clicked() {
            auf = !auf;
            ui.ctx().data_mut(|d| d.insert_temp(key, auf));
        }
        if auf {
            ui.add_space(4.0);
            add(ui);
            ui.add_space(4.0);
        }
    });
}

/// Beschriftung über einem Feld.
pub fn feld(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.label(egui::RichText::new(label).size(12.0).color(thema::muted()));
    add(ui);
}

/// Kleiner Hinweis unter einem Feld.
pub fn hinweis(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.5).color(thema::muted()));
}

/// Schalter (statt Häkchen) mit Text links; liefert true bei Änderung.
pub fn schalter(ui: &mut Ui, wert: &mut bool, text: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let w = ui.available_width();
        let g = ui.painter().layout(text.to_string(), egui::FontId::proportional(13.5), thema::text(), (w - 48.0).max(40.0));
        let h = g.size().y.max(20.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h + 4.0), Sense::click());
        ui.painter().galley(Pos2::new(rect.left(), rect.center().y - g.size().y / 2.0), g, thema::text());
        let sw = Rect::from_center_size(Pos2::new(rect.right() - 18.0, rect.center().y), egui::vec2(34.0, 20.0));
        let t = ui.ctx().animate_bool(resp.id, *wert);
        let bg = if *wert { farbe() } else { thema::pick(Color32::from_rgb(0xD5, 0xD8, 0xDE), Color32::from_rgb(0x45, 0x48, 0x50)) };
        ui.painter().rect_filled(sw, 10.0, bg);
        let x = egui::lerp(sw.left() + 10.0..=sw.right() - 10.0, t);
        ui.painter().circle_filled(Pos2::new(x, sw.center().y), 7.0, Color32::WHITE);
        if resp.clicked() {
            *wert = !*wert;
            changed = true;
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    });
    changed
}

/// Schieber mit Wertfeld daneben; liefert true bei Änderung.
pub fn schieber(ui: &mut Ui, wert: &mut f32, bereich: std::ops::RangeInclusive<f32>, einheit: &str, stellen: usize) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let vw = 64.0;
        ui.spacing_mut().slider_width = (ui.available_width() - vw - 10.0).max(60.0);
        changed |= ui.add(egui::Slider::new(wert, bereich.clone()).show_value(false).trailing_fill(true)).changed();
        let suffix = if einheit.is_empty() { String::new() } else { format!(" {einheit}") };
        changed |= ui
            .add_sized([vw, 26.0], egui::DragValue::new(wert).range(bereich).suffix(suffix).fixed_decimals(stellen).speed(0.1))
            .changed();
    });
    changed
}

/// Wahlknöpfe nebeneinander (eine Option aktiv); liefert true bei Änderung.
pub fn wahl<T: PartialEq + Copy>(ui: &mut Ui, wert: &mut T, optionen: &[(T, &str)]) -> bool {
    let mut changed = false;
    let w = ui.available_width();
    let n = optionen.len().max(1) as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 28.0), Sense::hover());
    ui.painter().rect(rect, 8.0, thema::panel(), Stroke::new(1.0, thema::border()));
    let cw = w / n;
    for (i, (v, label)) in optionen.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(rect.left() + cw * i as f32, rect.top()), egui::vec2(cw, 28.0));
        let resp = ui.interact(r, ui.id().with(("wahl", i, label)), Sense::click());
        let an = *wert == *v;
        if an {
            ui.painter().rect_filled(r.shrink(2.0), 6.0, toenung());
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), 6.0, thema::hover());
        }
        if i > 0 {
            ui.painter().vline(r.left(), r.shrink(6.0).y_range(), Stroke::new(1.0, thema::border()));
        }
        let g = galley(ui, label, 12.5, if an { thema::text() } else { thema::muted() }, an);
        ui.painter().galley(r.center() - g.size() / 2.0, g, thema::text());
        if resp.clicked() && !an {
            *wert = *v;
            changed = true;
        }
    }
    changed
}

/// Auswahlfeld (Klappliste) über die ganze Breite; liefert true bei Änderung.
pub fn liste<T: PartialEq + Copy>(ui: &mut Ui, id: &str, wert: &mut T, optionen: &[(T, &str)]) -> bool {
    let mut changed = false;
    let text = optionen.iter().find(|(v, _)| v == wert).map(|(_, l)| *l).unwrap_or("–");
    egui::ComboBox::from_id_salt(id).width(ui.available_width() - 4.0).selected_text(text).show_ui(ui, |ui| {
        for (v, l) in optionen {
            if ui.selectable_value(wert, *v, *l).changed() {
                changed = true;
            }
        }
    });
    changed
}

/// Farbige Marke (z. B. BILD, VIDEO) in fester Breite.
pub fn marke(ui: &mut Ui, text: &str, c: Color32, breite: f32) -> Response {
    let g = galley(ui, text, 10.5, c, true);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(breite.max(g.size().x + 12.0), 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 6.0, mix(c, thema::panel(), if thema::is_dark() { 0.22 } else { 0.12 }));
    ui.painter().galley(rect.center() - g.size() / 2.0, g, c);
    resp
}

/// Grauer Text (Zusammenfassung, Zusatz).
pub fn grau(ui: &mut Ui, text: &str) -> Response {
    ui.label(egui::RichText::new(text).size(13.0).color(thema::muted()))
}

/// Leerer Zustand in der Mitte: grosses Symbol, Titel, Text.
pub fn leer(ui: &mut Ui, s: Sym, titel: &str, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.25).min(120.0));
        let (r, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 28.0, toenung());
        sym(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(30.0)), s, akzent());
        ui.add_space(10.0);
        ui.label(egui::RichText::new(titel).size(18.0).strong());
        ui.label(egui::RichText::new(text).color(thema::muted()));
    });
}
