//! Speicher: Aufträge und Protokolle in einer JSON-Datei im Benutzerordner
//! (Windows `%APPDATA%\Sicherwerk\sicherwerk.json`, Linux `~/.config/sicherwerk/sicherwerk.json`).
//!
//! Fenster und Zeitplan-Läufe können gleichzeitig schreiben: Jede Änderung holt eine
//! Sperre, liest die Datei frisch, ändert und ersetzt sie dann in einem Schritt (Zwischendatei
//! + Umbenennen). Davor wird die alte Fassung als `.bak` aufbewahrt. Eine kaputte Datei wird
//! nie überschrieben, sondern beiseitegelegt.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::auftrag::Daten;
use crate::sperre::Sperre;

/// Fehler mit deutschem Text (die Oberfläche übersetzt `text`) und Einzelheiten.
#[derive(Clone, Debug)]
pub struct StoreFehler {
    pub text: &'static str,
    pub detail: String,
}

impl StoreFehler {
    fn neu(text: &'static str, detail: impl ToString) -> Self {
        StoreFehler { text, detail: detail.to_string() }
    }
}

/// Einstellungsordner von Sicherwerk.
pub fn ordner() -> Option<PathBuf> {
    if cfg!(windows) {
        let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("LOCALAPPDATA"))?;
        Some(PathBuf::from(base).join("Sicherwerk"))
    } else if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/Sicherwerk"))
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("sicherwerk"))
    }
}

pub fn datei() -> Option<PathBuf> {
    ordner().map(|o| o.join("sicherwerk.json"))
}

fn bak(p: &std::path::Path) -> PathBuf {
    p.with_extension("json.bak")
}

/// Sperrdatei für einen laufenden Auftrag.
pub fn lauf_sperre(id: u64) -> Option<PathBuf> {
    ordner().map(|o| o.join(format!("lauf-{id}.lock")))
}

/// Hinweis nach dem Laden (z. B. Datei war beschädigt).
#[derive(Clone, Debug)]
pub enum Hinweis {
    /// Datei war beschädigt (sie wird beim Schreiben als `.json.defekt-<zeit>` beiseitegelegt);
    /// geladen wurde die `.bak` (`aus_bak`) oder ein leerer Stand.
    Defekt { aus_bak: bool },
}

enum Lesen {
    Fehlt,
    Ok(Daten),
    Kaputt,
}

fn lesen(p: &std::path::Path) -> Lesen {
    match fs::read(p) {
        Ok(b) => match serde_json::from_slice::<Daten>(&b) {
            Ok(d) => Lesen::Ok(d),
            Err(_) => Lesen::Kaputt,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Lesen::Fehlt,
        Err(_) => Lesen::Kaputt,
    }
}

/// Lädt die Daten. Eine beschädigte Datei wird umbenannt (nie gelöscht) und die letzte
/// gute Fassung (`.bak`) genommen.
fn laden_intern(sperre_halten: bool) -> Result<(Daten, Option<Hinweis>), StoreFehler> {
    let Some(p) = datei() else {
        return Err(StoreFehler::neu("Benutzerordner nicht gefunden", ""));
    };
    match lesen(&p) {
        Lesen::Ok(d) => Ok((d, None)),
        Lesen::Fehlt => match lesen(&bak(&p)) {
            Lesen::Ok(d) => Ok((d, None)),
            _ => Ok((Daten::default(), None)),
        },
        Lesen::Kaputt => {
            // Ohne Sperre nur lesen (die .bak), beiseitelegen erst beim Schreiben.
            let aus_bak = lesen(&bak(&p));
            if !sperre_halten {
                return match aus_bak {
                    Lesen::Ok(d) => Ok((d, Some(Hinweis::Defekt { aus_bak: true }))),
                    _ => Ok((Daten::default(), Some(Hinweis::Defekt { aus_bak: false }))),
                };
            }
            let stempel = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let ziel = p.with_extension(format!("json.defekt-{stempel}"));
            fs::rename(&p, &ziel).map_err(|e| StoreFehler::neu("Beschädigte Datendatei konnte nicht beiseitegelegt werden", e))?;
            match aus_bak {
                Lesen::Ok(d) => Ok((d, Some(Hinweis::Defekt { aus_bak: true }))),
                _ => Ok((Daten::default(), Some(Hinweis::Defekt { aus_bak: false }))),
            }
        }
    }
}

/// Liest die Daten (ohne Sperre – die Datei wird immer in einem Schritt ersetzt).
pub fn laden() -> Result<(Daten, Option<Hinweis>), StoreFehler> {
    laden_intern(false)
}

/// Ändert die Daten unter Sperre: frisch lesen, `f` anwenden, sicher schreiben.
pub fn aendern<R>(f: impl FnOnce(&mut Daten) -> R) -> Result<(Daten, R, Option<Hinweis>), StoreFehler> {
    let (Some(dir), Some(p)) = (ordner(), datei()) else {
        return Err(StoreFehler::neu("Benutzerordner nicht gefunden", ""));
    };
    fs::create_dir_all(&dir).map_err(|e| StoreFehler::neu("Einstellungsordner konnte nicht angelegt werden", e))?;
    let sperre = Sperre::warten(&dir.join("sicherwerk.lock"), Duration::from_secs(30), Duration::from_secs(10))
        .map_err(|e| StoreFehler::neu("Datendatei ist gesperrt", e))?;
    let Some(_sperre) = sperre else {
        return Err(StoreFehler::neu("Datendatei ist gesperrt", ""));
    };
    let (mut d, hinweis) = laden_intern(true)?;
    let r = f(&mut d);
    let json = serde_json::to_vec_pretty(&d).map_err(|e| StoreFehler::neu("Speichern fehlgeschlagen", e))?;
    let tmp = p.with_extension("json.tmp");
    let schreiben = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        drop(f);
        if p.exists() {
            let _ = fs::copy(&p, bak(&p));
        }
        fs::rename(&tmp, &p)
    })();
    if let Err(e) = schreiben {
        let _ = fs::remove_file(&tmp);
        return Err(StoreFehler::neu("Speichern fehlgeschlagen", e));
    }
    Ok((d, r, hinweis))
}
