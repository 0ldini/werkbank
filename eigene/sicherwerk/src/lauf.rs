//! Ein Lauf mit allem Drumherum: Sperre je Auftrag (kein Doppellauf von Fenster und
//! Zeitplan), Uhrzeit, Motor, Protokoll in die Datendatei. Genutzt vom Fenster (im
//! Hintergrund-Thread) und von der Befehlszeile (`--auftrag`, ohne Fenster).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Datelike, TimeZone, Timelike};

use crate::auftrag::{Auftrag, Ausloeser, Fehler, FehlerArt, Protokoll, Taetigkeit};
use crate::motor::{self, Bilanz, Fortschritt, Stand, Umgebung};
use crate::sperre::Sperre;
use crate::store;

fn dos_lokal(unix: i64) -> (u16, u16) {
    match chrono::Local.timestamp_opt(unix, 0).earliest() {
        Some(d) => crate::zipper::dos_aus_teilen(d.year(), d.month(), d.day(), d.hour(), d.minute(), d.second()),
        None => crate::zipper::dos_aus_unix_utc(unix),
    }
}

pub fn umgebung() -> Umgebung {
    let jetzt = chrono::Local::now();
    Umgebung {
        jetzt_name: jetzt.format("%Y-%m-%d_%H-%M-%S").to_string(),
        jetzt_unix: jetzt.timestamp(),
        gross_klein_egal: cfg!(windows),
        dos_zeit: dos_lokal,
    }
}

/// «2026-10-18 14:30:05»
pub fn jetzt_text() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// «2026-10-18»
pub fn heute() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Unix-Zeit als Ortszeit «2026-10-18 14:30».
pub fn zeit_text(unix: i64) -> String {
    match chrono::Local.timestamp_opt(unix, 0).earliest() {
        Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
        None => String::new(),
    }
}

fn protokoll(a: &Auftrag, tk: Taetigkeit, ausl: Ausloeser, start: &str, start_unix: i64, dauer: Duration, b: &Bilanz) -> Protokoll {
    Protokoll {
        auftrag_id: a.id,
        auftrag_name: a.name.clone(),
        taetigkeit: tk,
        ausloeser: ausl,
        start: start.to_string(),
        start_unix,
        dauer_s: dauer.as_secs(),
        ergebnis: b.ergebnis(),
        neu: b.neu,
        geaendert: b.geaendert,
        entfernt: b.entfernt,
        verknuepft: b.verknuepft,
        unveraendert: b.unveraendert,
        bytes: b.bytes,
        version: b.version.clone(),
        versionen_geloescht: b.versionen_geloescht,
        geprueft: b.geprueft,
        fehler_anzahl: b.fehler_anzahl,
        fehler: b.fehler.clone(),
    }
}

/// Hält die Lauf-Sperre und frischt sie im Hintergrund auf, bis `fertig`.
struct GehalteneSperre {
    _sperre: Arc<Sperre>,
    lebt: Arc<AtomicBool>,
    faden: Option<std::thread::JoinHandle<()>>,
}

impl GehalteneSperre {
    fn holen(id: u64) -> Result<Option<GehalteneSperre>, String> {
        let Some(p) = store::lauf_sperre(id) else { return Ok(None) };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        match Sperre::versuchen(&p, Duration::from_secs(120)) {
            Ok(Some(s)) => {
                let s = Arc::new(s);
                let lebt = Arc::new(AtomicBool::new(true));
                let (s2, l2) = (s.clone(), lebt.clone());
                let faden = std::thread::spawn(move || {
                    let mut n = 0u32;
                    while l2.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(250));
                        n += 1;
                        if n % 80 == 0 {
                            s2.auffrischen();
                        }
                    }
                });
                Ok(Some(GehalteneSperre { _sperre: s, lebt, faden: Some(faden) }))
            }
            Ok(None) => Err(String::new()),
            // Sperrdatei nicht anlegbar (z. B. Ordner schreibgeschützt): ohne Sperre weiter.
            Err(_) => Ok(None),
        }
    }
}

impl Drop for GehalteneSperre {
    fn drop(&mut self) {
        self.lebt.store(false, Ordering::SeqCst);
        if let Some(f) = self.faden.take() {
            let _ = f.join();
        }
    }
}

/// Sichern oder Prüfen mit Sperre und Protokoll. Liefert das Protokoll und ggf. einen
/// Fehler beim Speichern des Protokolls.
pub fn ausfuehren(a: &Auftrag, tk: Taetigkeit, ausl: Ausloeser, fs_: &Fortschritt) -> (Protokoll, Option<store::StoreFehler>) {
    let start = jetzt_text();
    let u = umgebung();
    let t0 = Instant::now();
    let b = match GehalteneSperre::holen(a.id) {
        Err(_) => {
            let mut b = Bilanz { fatal: true, fehler_anzahl: 1, ..Default::default() };
            b.fehler.push(Fehler::neu(FehlerArt::Sperre, &a.name, ""));
            b
        }
        Ok(_sperre) => match tk {
            Taetigkeit::Pruefen => motor::pruefen(a, &u, fs_),
            _ => motor::sichern(a, &u, fs_),
        },
    };
    let p = protokoll(a, tk, ausl, &start, u.jetzt_unix, t0.elapsed(), &b);
    let fehler = store::aendern(|d| d.protokoll_anfuegen(p.clone())).err();
    (p, fehler)
}

/// Wiederherstellen mit Protokoll.
pub fn wiederherstellen(a: &Auftrag, s: &Stand, auswahl: &[String], ziel: &Path, fs_: &Fortschritt) -> (Protokoll, Option<store::StoreFehler>) {
    let start = jetzt_text();
    let u = umgebung();
    let t0 = Instant::now();
    let b = motor::wiederherstellen(a, s, auswahl, ziel, fs_);
    let p = protokoll(a, Taetigkeit::Wiederherstellen, Ausloeser::Hand, &start, u.jetzt_unix, t0.elapsed(), &b);
    let fehler = store::aendern(|d| d.protokoll_anfuegen(p.clone())).err();
    (p, fehler)
}
