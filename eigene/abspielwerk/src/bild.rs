//! Bilder laden (im Hilfsthread): Datei lesen, entschlüsseln, Drehung aus der Kamera
//! (EXIF) anwenden und für die Grafikkarte verkleinern.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// Grössere Dateien liest der Betrachter nicht (Schutz vor Speichermangel).
const MAX_DATEI: u64 = 600 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zweck {
    /// Bild der Wiedergabeliste
    Bild,
    /// Albumbild (Cover) eines Musikstücks
    Cover,
}

pub struct Geladen {
    pub pfad: PathBuf,
    pub zweck: Zweck,
    pub erg: Result<egui::ColorImage, String>,
}

/// Lädt `pfad` in einem eigenen Thread und meldet das Ergebnis über `tx`.
pub fn starten(pfad: PathBuf, zweck: Zweck, max_seite: usize, tx: Sender<Geladen>, ctx: egui::Context) {
    let name = pfad.clone();
    let r = std::thread::Builder::new().name("bild".into()).spawn(move || {
        // Ein Fehler in einer Bibliothek darf das Programm nicht beenden.
        let erg = std::panic::catch_unwind(|| laden(&pfad, max_seite)).unwrap_or_else(|_| Err("Bild nicht lesbar".to_string()));
        let _ = tx.send(Geladen { pfad, zweck, erg });
        ctx.request_repaint();
    });
    if r.is_err() {
        eprintln!("Bildlader startet nicht: {}", name.display());
    }
}

/// Bild lesen, nach EXIF drehen, auf höchstens `max_seite` Pixel verkleinern.
pub fn laden(pfad: &Path, max_seite: usize) -> Result<egui::ColorImage, String> {
    let gr = std::fs::metadata(pfad).map_err(|e| e.to_string())?.len();
    if gr > MAX_DATEI {
        return Err("Datei zu gross".to_string());
    }
    let daten = std::fs::read(pfad).map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&daten).map_err(|e| e.to_string())?;
    let img = drehen(img, orientierung(&daten));
    let max = max_seite.clamp(256, 16384) as u32;
    let img = if img.width() > max || img.height() > max { img.resize(max, max, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    if size[0] == 0 || size[1] == 0 {
        return Err("Bild ist leer".to_string());
    }
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// EXIF-Orientierung 1–8 (1 = unverändert).
fn orientierung(daten: &[u8]) -> u32 {
    let mut c = Cursor::new(daten);
    exif::Reader::new()
        .read_from_container(&mut c)
        .ok()
        .and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)))
        .unwrap_or(1)
}

fn drehen(img: image::DynamicImage, o: u32) -> image::DynamicImage {
    match o {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}
