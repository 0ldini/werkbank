//! Sperrdateien, damit ein Auftrag nicht zweimal gleichzeitig läuft (Fenster und Zeitplan)
//! und die Datendatei nicht von zwei Prozessen zugleich geschrieben wird. Eine Sperre, deren
//! Datei länger als `veraltet` nicht aufgefrischt wurde, gilt als verwaist (Absturz) und wird
//! übernommen. Nur std; Test: `rustc --edition 2021 --test src/sperre.rs`.

#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug)]
pub struct Sperre {
    pfad: PathBuf,
}

fn ist_veraltet(pfad: &Path, veraltet: Duration) -> bool {
    let Ok(t) = fs::metadata(pfad).and_then(|m| m.modified()) else {
        return false;
    };
    let jetzt = SystemTime::now();
    let abstand = match jetzt.duration_since(t) {
        Ok(d) => d,
        Err(e) => e.duration(), // Zeit in der Zukunft (Uhr verstellt)
    };
    abstand > veraltet
}

impl Sperre {
    /// Versucht die Sperre zu bekommen; `Ok(None)`, wenn sie jemand anderes hält.
    pub fn versuchen(pfad: &Path, veraltet: Duration) -> io::Result<Option<Sperre>> {
        for versuch in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(pfad) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(Some(Sperre { pfad: pfad.to_path_buf() }));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    if versuch == 0 && ist_veraltet(pfad, veraltet) {
                        let _ = fs::remove_file(pfad);
                        continue;
                    }
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// Wartet höchstens `max` auf die Sperre.
    pub fn warten(pfad: &Path, veraltet: Duration, max: Duration) -> io::Result<Option<Sperre>> {
        let start = Instant::now();
        loop {
            if let Some(s) = Self::versuchen(pfad, veraltet)? {
                return Ok(Some(s));
            }
            if start.elapsed() >= max {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    /// Zeigt, dass der Halter noch lebt (Änderungszeit der Datei neu setzen).
    pub fn auffrischen(&self) {
        if let Ok(mut f) = OpenOptions::new().write(true).truncate(true).open(&self.pfad) {
            let _ = writeln!(f, "{}", std::process::id());
        }
    }
}

impl Drop for Sperre {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.pfad);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sperren() {
        let d = std::env::temp_dir().join(format!("sicherwerk-sperre-{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        let p = d.join("x.lock");
        let _ = fs::remove_file(&p);
        let a = Sperre::versuchen(&p, Duration::from_secs(60)).unwrap();
        assert!(a.is_some());
        assert!(Sperre::versuchen(&p, Duration::from_secs(60)).unwrap().is_none());
        assert!(Sperre::warten(&p, Duration::from_secs(60), Duration::from_millis(100)).unwrap().is_none());
        a.as_ref().unwrap().auffrischen();
        drop(a);
        assert!(!p.exists());
        let b = Sperre::versuchen(&p, Duration::from_secs(60)).unwrap();
        assert!(b.is_some());
        // Verwaiste Sperre (veraltet = 0) wird übernommen.
        std::thread::sleep(Duration::from_millis(20));
        let c = Sperre::versuchen(&p, Duration::from_millis(1)).unwrap();
        assert!(c.is_some());
        std::mem::forget(b); // b würde sonst c's Datei löschen
        drop(c);
        let _ = fs::remove_dir_all(&d);
    }
}
