//! Gemeinsame Schnittstelle für FTP/FTPS und SFTP: alles, was FTPwerk auf einem Server tut.

use std::path::Path;
use std::sync::mpsc::Sender;

use crate::model::{Entry, Proto, RErr, Res, Site};

/// Fortschrittsmeldung während einer Übertragung.
pub enum Tick {
    /// Es geht los (bei «Fortsetzen» mit `offset` > 0); `total` = Gesamtgrösse, 0 = unbekannt.
    Start { offset: u64, total: u64 },
    /// So viele Bytes sind seit der letzten Meldung dazugekommen.
    Bytes(u64),
}

/// Rückgabe `false` = Übertragung abbrechen.
pub type Prog<'a> = &'a mut dyn FnMut(Tick) -> bool;

pub trait Remote: Send {
    /// Aktueller Ordner (Startordner nach dem Anmelden).
    fn pwd(&mut self) -> Res<String>;
    fn list(&mut self, path: &str) -> Res<Vec<Entry>>;
    fn mkdir(&mut self, path: &str) -> Res<()>;
    fn rename(&mut self, from: &str, to: &str) -> Res<()>;
    fn remove_file(&mut self, path: &str) -> Res<()>;
    fn remove_dir(&mut self, path: &str) -> Res<()>;
    fn chmod(&mut self, path: &str, mode: u32) -> Res<()>;
    /// Grösse einer Datei, falls der Server sie nennt.
    fn size(&mut self, path: &str) -> Res<Option<u64>>;
    /// Lädt herunter. `resume`: vorhandene Teildatei fortsetzen. Rückgabe: übertragene Bytes (0 = war schon komplett).
    fn download(&mut self, remote: &str, local: &Path, resume: bool, prog: Prog) -> Res<u64>;
    /// Lädt hoch. `resume`: angefangene Datei auf dem Server fortsetzen.
    fn upload(&mut self, local: &Path, remote: &str, resume: bool, prog: Prog) -> Res<u64>;
    /// Hält die Verbindung wach.
    fn keepalive(&mut self) -> Res<()>;
    fn close(&mut self);
}

/// Baut eine Verbindung auf und meldet sich an. `trusted` = Fingerabdrücke, denen der Benutzer für
/// diesen Server schon vertraut (Zertifikat bei FTPS, Schlüssel bei SFTP). `log` bekommt die
/// Protokollzeilen (ohne Passwörter).
pub fn connect(site: &Site, trusted: &[String], log: Option<Sender<String>>) -> Res<Box<dyn Remote>> {
    if site.host.trim().is_empty() {
        return Err(RErr::Msg(crate::sprache::t("Bitte einen Server eintragen.").to_string()));
    }
    match site.proto {
        Proto::Sftp => {
            #[cfg(feature = "sftp")]
            {
                Ok(Box::new(crate::sftp::Sftp::connect(site, trusted, log)?))
            }
            #[cfg(not(feature = "sftp"))]
            {
                let _ = (trusted, log);
                Err(RErr::Msg(crate::sprache::t("SFTP ist in dieser Ausführung nicht enthalten.").to_string()))
            }
        }
        _ => Ok(Box::new(crate::ftp::Ftp::connect(site, trusted, log)?)),
    }
}

/// Zeile ins Protokoll schreiben, falls eines angeschlossen ist.
pub fn log_line(log: &Option<Sender<String>>, line: impl Into<String>) {
    if let Some(l) = log {
        let _ = l.send(line.into());
    }
}
