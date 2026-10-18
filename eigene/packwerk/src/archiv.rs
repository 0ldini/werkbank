//! Motor: Archive lesen, entpacken und erstellen (ZIP, 7z, TAR, TAR.GZ, GZ).
//! Keine Oberfläche; läuft in Hintergrund-Threads und meldet über [`Fortschritt`].
//!
//! Sicherheit: Jeder Eintragsname geht durch `regeln::sichere_teile` (kein «..», keine
//! absoluten Pfade) und wird zusätzlich lexikalisch gegen den Zielordner geprüft.
//! Verknüpfungen (Symlinks, Hardlinks) werden nie angelegt. Jede Datei wird zuerst als
//! versteckte Teil-Datei geschrieben und erst am Ende umbenannt; bei Abbruch oder Fehler
//! wird die Teil-Datei gelöscht. Bestehende Dateien werden nur mit ausdrücklicher Wahl
//! (Konflikt::Ueberschreiben) ersetzt. Pro Eintrag wird höchstens die angegebene Grösse
//! geschrieben (Schutz vor Archiven, die beim Entpacken «wachsen»).

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::anzeige::{self, Zeit};
use crate::regeln::{self, Art, PfadFehler};
use crate::sprache::{t, tf};

const WINDOWS: bool = cfg!(windows);
const PUFFER: usize = 128 * 1024;
/// Kennung der AES-Verschlüsselung in 7z-Archiven.
const SZ_AES_ID: [u8; 4] = [0x06, 0xF1, 0x07, 0x01];
/// Grösste Blockgrösse für «solide» 7z-Blöcke (wie in sevenz-rust2).
const SZ_BLOCK_MAX: u64 = 4 * 1024 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Fortschritt und Fehler
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Fortschritt {
    pub abbrechen: AtomicBool,
    pub erledigt: AtomicU64,
    pub gesamt: AtomicU64,
    aktuell: Mutex<String>,
}

impl Fortschritt {
    pub fn abgebrochen(&self) -> bool {
        self.abbrechen.load(Ordering::Relaxed)
    }
    pub fn plus(&self, n: u64) {
        self.erledigt.fetch_add(n, Ordering::Relaxed);
    }
    pub fn setze_gesamt(&self, n: u64) {
        self.gesamt.store(n, Ordering::Relaxed);
        self.erledigt.store(0, Ordering::Relaxed);
    }
    pub fn setze_aktuell(&self, s: &str) {
        let mut g = self.aktuell.lock().unwrap_or_else(|e| e.into_inner());
        g.clear();
        g.push_str(s);
    }
    pub fn aktuell(&self) -> String {
        self.aktuell
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    /// 0.0 … 1.0, oder `None`, solange die Gesamtmenge unbekannt ist.
    pub fn anteil(&self) -> Option<f32> {
        let g = self.gesamt.load(Ordering::Relaxed);
        if g == 0 {
            return None;
        }
        let e = self.erledigt.load(Ordering::Relaxed).min(g);
        Some((e as f64 / g as f64) as f32)
    }
}

#[derive(Debug, Clone)]
pub enum Fehler {
    Io(String),
    /// Archiv ist verschlüsselt, es fehlt ein Passwort.
    PasswortNoetig,
    /// Passwort stimmt nicht (oder Daten beschädigt).
    PasswortFalsch,
    /// Archiv beschädigt oder Format-Fehler.
    Kaputt(String),
    /// Keine bekannte Archivart.
    Unbekannt,
    /// Kompressionsart oder Funktion wird nicht unterstützt.
    NichtUnterstuetzt(String),
    Abgebrochen,
    /// Beim Entpacken gibt es schon Dateien gleichen Namens (Anzahl, Beispiele).
    Konflikte(usize, Vec<PathBuf>),
    /// Zieldatei beim Packen existiert schon.
    ZielExistiert(PathBuf),
    /// Ein Eintrag lieferte mehr Daten als angegeben.
    ZuGross(String),
    /// GZ kann nur eine einzelne Datei packen.
    GzNurEineDatei,
    /// Nichts zu tun.
    Leer,
}

impl Fehler {
    pub fn text(&self) -> String {
        match self {
            Fehler::Io(s) => tf("Dateifehler: {s}", &[("s", s)]),
            Fehler::PasswortNoetig => {
                t("Das Archiv ist verschlüsselt. Bitte Passwort eingeben.").to_string()
            }
            Fehler::PasswortFalsch => {
                t("Das Passwort stimmt nicht (oder das Archiv ist beschädigt).").to_string()
            }
            Fehler::Kaputt(s) => tf("Das Archiv ist beschädigt oder ungültig: {s}", &[("s", s)]),
            Fehler::Unbekannt => {
                t("Unbekanntes Format. Packwerk kennt ZIP, 7z, TAR, TAR.GZ und GZ.").to_string()
            }
            Fehler::NichtUnterstuetzt(s) => tf("Nicht unterstützt: {s}", &[("s", s)]),
            Fehler::Abgebrochen => t("Abgebrochen.").to_string(),
            Fehler::Konflikte(n, _) => tf("{n} Dateien gibt es im Ziel schon.", &[("n", n)]),
            Fehler::ZielExistiert(p) => {
                let p = p.display().to_string();
                tf("Die Datei gibt es schon: {p}", &[("p", &p)])
            }
            Fehler::ZuGross(s) => tf(
                "Eintrag ist grösser als angegeben (mögliche Archiv-Bombe): {s}",
                &[("s", s)],
            ),
            Fehler::GzNurEineDatei => {
                t("GZ packt genau eine Datei. Für mehrere Dateien oder Ordner TAR.GZ wählen.")
                    .to_string()
            }
            Fehler::Leer => t("Nichts zu tun.").to_string(),
        }
    }
}

impl From<io::Error> for Fehler {
    fn from(e: io::Error) -> Self {
        Fehler::Io(e.to_string())
    }
}

fn zip_fehler(e: zip::result::ZipError) -> Fehler {
    use zip::result::ZipError;
    match e {
        ZipError::Io(e) => Fehler::Io(e.to_string()),
        ZipError::InvalidPassword => Fehler::PasswortFalsch,
        ZipError::UnsupportedArchive(s) if s == ZipError::PASSWORD_REQUIRED => {
            Fehler::PasswortNoetig
        }
        ZipError::UnsupportedArchive(s) => Fehler::NichtUnterstuetzt(s.to_string()),
        ZipError::CompressionMethodNotSupported(m) => {
            Fehler::NichtUnterstuetzt(tf("ZIP-Methode {m}", &[("m", &m)]))
        }
        ZipError::InvalidArchive(s) => Fehler::Kaputt(s.to_string()),
        andere => Fehler::Kaputt(andere.to_string()),
    }
}

fn sz_fehler(e: sevenz_rust2::Error, mit_passwort: bool) -> Fehler {
    use sevenz_rust2::Error as E;
    match e {
        E::PasswordRequired => Fehler::PasswortNoetig,
        E::MaybeBadPassword(_) => Fehler::PasswortFalsch,
        E::FileOpen(e, _) => Fehler::Io(e.to_string()),
        E::UnsupportedCompressionMethod(m) => Fehler::NichtUnterstuetzt(m),
        E::Unsupported(s) => Fehler::NichtUnterstuetzt(s.to_string()),
        E::Io(e, _) if !mit_passwort => Fehler::Io(e.to_string()),
        _ if mit_passwort => Fehler::PasswortFalsch,
        andere => Fehler::Kaputt(andere.to_string()),
    }
}

fn pfad_grund(f: &PfadFehler) -> String {
    match f {
        PfadFehler::Leer => t("leerer Name").to_string(),
        PfadFehler::Absolut => t("absoluter Pfad").to_string(),
        PfadFehler::Elternverweis => t("zeigt mit «..» aus dem Zielordner").to_string(),
        PfadFehler::Ungueltig(teil) => tf("ungültiger Name «{n}»", &[("n", teil)]),
    }
}

/// Liest und zählt mit; bricht ab, wenn der Benutzer abbricht.
struct Zaehler<'a, R> {
    inner: R,
    fp: &'a Fortschritt,
    /// Gelesene Bytes zum Fortschritt zählen (sonst nur auf Abbruch achten).
    zaehlen: bool,
}

impl<R: Read> Read for Zaehler<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.fp.abgebrochen() {
            return Err(io::Error::other("abgebrochen"));
        }
        let n = self.inner.read(buf)?;
        if self.zaehlen {
            self.fp.plus(n as u64);
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// Inhalt lesen
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EintragArt {
    Datei,
    Ordner,
    /// Symlink oder Hardlink – wird nie angelegt.
    Verknuepfung,
    /// Gerätedatei, FIFO, defekter Eintrag …
    Sonstiges,
}

#[derive(Clone, Debug)]
pub struct Eintrag {
    pub name: String,
    pub art: EintragArt,
    pub groesse: u64,
    pub gepackt: Option<u64>,
    pub zeit: Option<Zeit>,
    pub verschluesselt: bool,
    pub modus: Option<u32>,
    /// Geprüfte Pfadteile, oder warum der Name abgelehnt wird.
    pub sicher: Result<Vec<String>, PfadFehler>,
}

impl Eintrag {
    /// Wird dieser Eintrag wegen seines Pfads abgelehnt? (Der Ordner «./» selbst nicht.)
    pub fn unsicher(&self) -> bool {
        match &self.sicher {
            Ok(_) => false,
            Err(PfadFehler::Leer) => self.art != EintragArt::Ordner,
            Err(_) => true,
        }
    }

    fn neu(name: String, art: EintragArt) -> Eintrag {
        let sicher = regeln::sichere_teile(&name, WINDOWS);
        Eintrag {
            name,
            art,
            groesse: 0,
            gepackt: None,
            zeit: None,
            verschluesselt: false,
            modus: None,
            sicher,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Inhalt {
    pub pfad: PathBuf,
    pub art: Art,
    pub eintraege: Vec<Eintrag>,
    pub verschluesselt: bool,
    pub archivgroesse: u64,
    /// ZIP mit sich überlappenden Einträgen (typische Zip-Bombe).
    pub ueberlappend: bool,
}

impl Inhalt {
    pub fn unsichere(&self) -> usize {
        self.eintraege.iter().filter(|e| e.unsicher()).count()
    }
}

fn kopf_lesen(pfad: &Path, n: u64) -> io::Result<Vec<u8>> {
    let mut v = Vec::new();
    File::open(pfad)?.take(n).read_to_end(&mut v)?;
    Ok(v)
}

/// Ermittelt die Archivart (Inhalt vor Name; bei gzip ein Blick ins Entpackte).
pub fn art_ermitteln(pfad: &Path) -> Result<Art, Fehler> {
    let kopf = kopf_lesen(pfad, 512)?;
    let name = pfad
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let art = regeln::art_bestimmen(&name, &kopf).ok_or(Fehler::Unbekannt)?;
    if matches!(art, Art::Gz | Art::TarGz) {
        let mut innen = Vec::new();
        let ok = File::open(pfad)
            .and_then(|f| {
                flate2::read::MultiGzDecoder::new(BufReader::new(f))
                    .take(512)
                    .read_to_end(&mut innen)
            })
            .is_ok();
        let name_sagt_tar = regeln::art_aus_name(&name) == Some(Art::TarGz);
        if ok && (regeln::ist_tar_kopf(&innen) || (name_sagt_tar && innen.iter().all(|b| *b == 0)))
        {
            return Ok(Art::TarGz);
        }
        return Ok(Art::Gz);
    }
    Ok(art)
}

/// Liest das Inhaltsverzeichnis eines Archivs.
pub fn lesen(pfad: &Path, passwort: Option<&str>, fp: &Fortschritt) -> Result<Inhalt, Fehler> {
    let art = art_ermitteln(pfad)?;
    let archivgroesse = fs::metadata(pfad)?.len();
    fp.setze_aktuell(
        &pfad
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let mut inhalt = Inhalt {
        pfad: pfad.to_path_buf(),
        art,
        eintraege: Vec::new(),
        verschluesselt: false,
        archivgroesse,
        ueberlappend: false,
    };
    match art {
        Art::Zip => zip_lesen(&mut inhalt, fp)?,
        Art::SiebenZ => sz_lesen(&mut inhalt, passwort, fp)?,
        Art::Tar | Art::TarGz => tar_lesen(&mut inhalt, fp)?,
        Art::Gz => gz_lesen(&mut inhalt, fp)?,
    }
    if fp.abgebrochen() {
        return Err(Fehler::Abgebrochen);
    }
    Ok(inhalt)
}

fn zip_zeit(d: zip::DateTime) -> Option<Zeit> {
    if !d.is_valid() {
        return None;
    }
    Some(Zeit {
        jahr: d.year() as i32,
        monat: d.month(),
        tag: d.day(),
        stunde: d.hour(),
        minute: d.minute(),
        sekunde: d.second(),
    })
}

fn zip_lesen(inhalt: &mut Inhalt, fp: &Fortschritt) -> Result<(), Fehler> {
    let f = File::open(&inhalt.pfad)?;
    let mut za = zip::ZipArchive::new(BufReader::new(f)).map_err(zip_fehler)?;
    fp.setze_gesamt(za.len() as u64);
    for i in 0..za.len() {
        if fp.abgebrochen() {
            return Err(Fehler::Abgebrochen);
        }
        let gelesen = za.by_index_raw(i).map(|z| {
            let art = if z.is_dir() {
                EintragArt::Ordner
            } else if z.is_symlink() {
                EintragArt::Verknuepfung
            } else {
                EintragArt::Datei
            };
            let mut e = Eintrag::neu(z.name().to_string(), art);
            e.groesse = z.size();
            e.gepackt = Some(z.compressed_size());
            e.zeit = z.last_modified().and_then(zip_zeit);
            e.verschluesselt = z.encrypted();
            e.modus = z.unix_mode();
            e
        });
        let e = match gelesen {
            Ok(e) => e,
            // Ein defekter Eintrag soll nicht das ganze Verzeichnis verhindern.
            Err(_) => {
                let name = za.name_for_index(i).unwrap_or("?").to_string();
                Eintrag::neu(name, EintragArt::Sonstiges)
            }
        };
        inhalt.verschluesselt |= e.verschluesselt;
        inhalt.eintraege.push(e);
        fp.plus(1);
    }
    inhalt.ueberlappend = za.has_overlapping_files().unwrap_or(false);
    Ok(())
}

fn sz_passwort(passwort: Option<&str>) -> sevenz_rust2::Password {
    match passwort {
        Some(p) if !p.is_empty() => sevenz_rust2::Password::new(p),
        _ => sevenz_rust2::Password::empty(),
    }
}

fn sz_lesen(inhalt: &mut Inhalt, passwort: Option<&str>, fp: &Fortschritt) -> Result<(), Fehler> {
    let mit_pw = passwort.is_some_and(|p| !p.is_empty());
    let leser = sevenz_rust2::ArchiveReader::open(&inhalt.pfad, sz_passwort(passwort))
        .map_err(|e| sz_fehler(e, mit_pw))?;
    let a = leser.archive();
    let verschl = a.blocks.iter().any(|b| {
        b.coders
            .iter()
            .any(|c| c.encoder_method_id() == SZ_AES_ID.as_slice())
    });
    inhalt.verschluesselt = verschl;
    fp.setze_gesamt(a.files.len() as u64);
    for f in &a.files {
        if fp.abgebrochen() {
            return Err(Fehler::Abgebrochen);
        }
        let art = if f.is_anti_item() {
            EintragArt::Sonstiges
        } else if f.is_directory() {
            EintragArt::Ordner
        } else {
            EintragArt::Datei
        };
        let mut e = Eintrag::neu(f.name().to_string(), art);
        e.groesse = f.size();
        e.gepackt = (f.compressed_size > 0).then_some(f.compressed_size);
        if f.has_last_modified_date {
            e.zeit = ortszeit(anzeige::unix_aus_nt(u64::from(f.last_modified_date())));
        }
        e.verschluesselt = verschl;
        inhalt.eintraege.push(e);
        fp.plus(1);
    }
    Ok(())
}

/// Öffnet TAR bzw. TAR.GZ als Lesestrom; Fortschritt = gelesene Bytes der Archivdatei.
fn tar_strom<'a>(
    pfad: &Path,
    art: Art,
    fp: &'a Fortschritt,
    zaehlen: bool,
) -> Result<Box<dyn Read + 'a>, Fehler> {
    let f = File::open(pfad)?;
    if zaehlen {
        fp.setze_gesamt(f.metadata().map(|m| m.len()).unwrap_or(0));
    }
    let z = Zaehler {
        inner: BufReader::with_capacity(PUFFER, f),
        fp,
        zaehlen,
    };
    Ok(match art {
        Art::TarGz | Art::Gz => Box::new(flate2::read::MultiGzDecoder::new(z)),
        _ => Box::new(z),
    })
}

fn tar_name<R: Read>(e: &tar::Entry<'_, R>) -> String {
    String::from_utf8_lossy(&e.path_bytes()).into_owned()
}

fn tar_lesen(inhalt: &mut Inhalt, fp: &Fortschritt) -> Result<(), Fehler> {
    let strom = tar_strom(&inhalt.pfad, inhalt.art, fp, true)?;
    let mut ar = tar::Archive::new(strom);
    let eintraege = ar.entries().map_err(|e| lesefehler(e, fp))?;
    for e in eintraege {
        let e = e.map_err(|e| lesefehler(e, fp))?;
        let h = e.header();
        let typ = h.entry_type();
        let art = if typ.is_dir() {
            EintragArt::Ordner
        } else if typ.is_file() {
            EintragArt::Datei
        } else if typ.is_symlink() || typ.is_hard_link() {
            EintragArt::Verknuepfung
        } else {
            EintragArt::Sonstiges
        };
        let mut ein = Eintrag::neu(tar_name(&e), art);
        ein.groesse = if art == EintragArt::Datei {
            e.size()
        } else {
            0
        };
        ein.zeit = h
            .mtime()
            .ok()
            .and_then(|s| i64::try_from(s).ok())
            .and_then(ortszeit);
        ein.modus = h.mode().ok();
        inhalt.eintraege.push(ein);
    }
    Ok(())
}

fn lesefehler(e: io::Error, fp: &Fortschritt) -> Fehler {
    if fp.abgebrochen() {
        Fehler::Abgebrochen
    } else if e.kind() == io::ErrorKind::NotFound || e.kind() == io::ErrorKind::PermissionDenied {
        Fehler::Io(e.to_string())
    } else {
        Fehler::Kaputt(e.to_string())
    }
}

fn gz_lesen(inhalt: &mut Inhalt, fp: &Fortschritt) -> Result<(), Fehler> {
    // Die entpackte Grösse steht nicht verlässlich in der Datei: einmal durchzählen.
    let mut strom = tar_strom(&inhalt.pfad, Art::Gz, fp, true)?;
    let n = io::copy(&mut strom, &mut io::sink()).map_err(|e| lesefehler(e, fp))?;
    let name = inhalt
        .pfad
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut e = Eintrag::neu(regeln::gz_zielname(&name), EintragArt::Datei);
    e.groesse = n;
    e.gepackt = Some(inhalt.archivgroesse);
    e.zeit = fs::metadata(&inhalt.pfad)
        .and_then(|m| m.modified())
        .ok()
        .and_then(unix_von)
        .and_then(ortszeit);
    inhalt.eintraege.push(e);
    Ok(())
}

fn unix_von(t: SystemTime) -> Option<i64> {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).ok(),
        Err(e) => i64::try_from(e.duration().as_secs()).ok().map(|s| -s),
    }
}

/// Sekunden seit 1970 → Ortszeit des Rechners (ZIP speichert Ortszeit, TAR und 7z UTC;
/// angezeigt wird alles in Ortszeit).
fn ortszeit(sek: i64) -> Option<Zeit> {
    use chrono::{Datelike, TimeZone, Timelike};
    match chrono::Local.timestamp_opt(sek, 0).earliest() {
        Some(d) => {
            let n = d.naive_local();
            Some(Zeit {
                jahr: n.year(),
                monat: n.month() as u8,
                tag: n.day() as u8,
                stunde: n.hour() as u8,
                minute: n.minute() as u8,
                sekunde: n.second().min(59) as u8,
            })
        }
        None => Zeit::aus_unix(sek),
    }
}

/// Ortszeit → Sekunden seit 1970.
fn unix_aus_ortszeit(z: Zeit) -> Option<i64> {
    use chrono::TimeZone;
    let n = chrono::NaiveDate::from_ymd_opt(z.jahr, z.monat.into(), z.tag.into())?.and_hms_opt(
        z.stunde.into(),
        z.minute.into(),
        z.sekunde.min(59).into(),
    )?;
    chrono::Local
        .from_local_datetime(&n)
        .earliest()
        .map(|d| d.timestamp())
}

fn systemzeit(z: Zeit) -> Option<SystemTime> {
    let s = unix_aus_ortszeit(z)?;
    if s >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(s as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(s.unsigned_abs()))
    }
}

// ---------------------------------------------------------------------------
// Entpacken
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Konflikt {
    /// Erst prüfen; gibt es Konflikte, endet der Auftrag mit `Fehler::Konflikte`.
    Fragen,
    Ueberschreiben,
    /// Beide behalten: neue Datei bekommt «(2)».
    Umbenennen,
    Ueberspringen,
}

pub struct EntpackAuftrag {
    pub inhalt: Inhalt,
    pub passwort: Option<String>,
    pub ziel: PathBuf,
    /// Nur diese Einträge (Indizes in `inhalt.eintraege`), `None` = alle.
    pub auswahl: Option<HashSet<usize>>,
    /// So viele führende Pfadteile weglassen (Ordner, in dem man in der Ansicht steht).
    pub weglassen: usize,
    pub konflikt: Konflikt,
}

#[derive(Clone, Debug, Default)]
pub struct Bericht {
    pub dateien: u64,
    pub ordner: u64,
    pub bytes: u64,
    /// (Name, Grund) – abgelehnte oder fehlgeschlagene Einträge.
    pub abgelehnt: Vec<(String, String)>,
    /// Übersprungene Einträge (Verknüpfungen, vorhandene Dateien).
    pub uebersprungen: Vec<String>,
    pub ziel: PathBuf,
}

enum Plan {
    Datei(PathBuf),
    Ordner(PathBuf),
}

enum SchreibFehler {
    Lesen(io::Error),
    Schreiben(io::Error),
    ZuGross,
    Abgebrochen,
}

/// Name der versteckten Teil-Datei neben `ziel`.
fn teil_pfad(ziel: &Path) -> PathBuf {
    let name = ziel
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let kurz: String = name.chars().take(80).collect();
    let eltern = ziel.parent().map(Path::to_path_buf).unwrap_or_default();
    regeln::freier_pfad(
        &eltern.join(format!(".{kurz}.packwerk-{}.teil", std::process::id())),
        |p| p.symlink_metadata().is_ok(),
    )
}

/// Kopiert höchstens `max` Bytes aus `quelle` in eine neue Datei `ziel` (über eine Teil-Datei).
fn datei_schreiben(
    ziel: &Path,
    quelle: &mut dyn Read,
    max: u64,
    zeit: Option<Zeit>,
    modus: Option<u32>,
    fp: &Fortschritt,
) -> Result<u64, SchreibFehler> {
    let teil = teil_pfad(ziel);
    let ergebnis = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&teil)
            .map_err(SchreibFehler::Schreiben)?;
        let mut buf = vec![0u8; PUFFER];
        let mut total: u64 = 0;
        loop {
            if fp.abgebrochen() {
                return Err(SchreibFehler::Abgebrochen);
            }
            let n = match quelle.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(SchreibFehler::Lesen(e)),
            };
            total = total.saturating_add(n as u64);
            if total > max {
                return Err(SchreibFehler::ZuGross);
            }
            f.write_all(buf.get(..n).unwrap_or(&[]))
                .map_err(SchreibFehler::Schreiben)?;
            fp.plus(n as u64);
        }
        f.flush().map_err(SchreibFehler::Schreiben)?;
        if let Some(t) = zeit.and_then(systemzeit) {
            let _ = f.set_modified(t);
        }
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if modus.is_some_and(|m| m & 0o111 != 0) {
                let _ = fs::set_permissions(&teil, fs::Permissions::from_mode(0o755));
            }
        }
        #[cfg(not(unix))]
        let _ = modus;
        fs::rename(&teil, ziel).map_err(SchreibFehler::Schreiben)?;
        Ok(total)
    })();
    if ergebnis.is_err() {
        let _ = fs::remove_file(&teil);
    }
    ergebnis
}

struct Entpacker<'a> {
    fp: &'a Fortschritt,
    bericht: Bericht,
    konflikt: Konflikt,
    verschluesselt: bool,
}

impl Entpacker<'_> {
    fn ordner(&mut self, name: &str, ziel: &Path) {
        if ziel.is_dir() {
            return;
        }
        match fs::create_dir_all(ziel) {
            Ok(()) => self.bericht.ordner += 1,
            Err(e) => self
                .bericht
                .abgelehnt
                .push((name.to_string(), e.to_string())),
        }
    }

    /// Schreibt eine Datei; `Err` nur bei Fehlern, die das ganze Entpacken beenden.
    fn datei(&mut self, e: &Eintrag, ziel: &Path, quelle: &mut dyn Read) -> Result<(), Fehler> {
        self.fp.setze_aktuell(&e.name);
        let mut ziel = ziel.to_path_buf();
        if ziel.symlink_metadata().is_ok() {
            match self.konflikt {
                Konflikt::Ueberspringen | Konflikt::Fragen => {
                    self.bericht.uebersprungen.push(e.name.clone());
                    self.fp.plus(e.groesse);
                    return Ok(());
                }
                Konflikt::Umbenennen => {
                    ziel = regeln::freier_pfad(&ziel, |p| p.symlink_metadata().is_ok())
                }
                Konflikt::Ueberschreiben => {
                    if ziel.is_dir() {
                        self.bericht.abgelehnt.push((
                            e.name.clone(),
                            t("Im Ziel ist ein Ordner gleichen Namens.").to_string(),
                        ));
                        return Ok(());
                    }
                }
            }
        }
        if let Some(eltern) = ziel.parent() {
            if let Err(err) = fs::create_dir_all(eltern) {
                self.bericht
                    .abgelehnt
                    .push((e.name.clone(), err.to_string()));
                return Ok(());
            }
        }
        match datei_schreiben(&ziel, quelle, e.groesse, e.zeit, e.modus, self.fp) {
            Ok(n) => {
                self.bericht.dateien += 1;
                self.bericht.bytes = self.bericht.bytes.saturating_add(n);
                Ok(())
            }
            Err(SchreibFehler::Abgebrochen) => Err(Fehler::Abgebrochen),
            Err(SchreibFehler::ZuGross) => Err(Fehler::ZuGross(e.name.clone())),
            Err(SchreibFehler::Schreiben(err)) => {
                self.bericht
                    .abgelehnt
                    .push((e.name.clone(), err.to_string()));
                Ok(())
            }
            Err(SchreibFehler::Lesen(err)) => {
                if self.fp.abgebrochen() {
                    Err(Fehler::Abgebrochen)
                } else if self.verschluesselt || e.verschluesselt {
                    Err(Fehler::PasswortFalsch)
                } else {
                    Err(Fehler::Kaputt(format!("{}: {err}", e.name)))
                }
            }
        }
    }
}

/// Entpackt ein Archiv (ganz oder die Auswahl) in den Zielordner.
pub fn entpacken(a: &EntpackAuftrag, fp: &Fortschritt) -> Result<Bericht, Fehler> {
    let inhalt = &a.inhalt;
    let mut bericht = Bericht {
        ziel: a.ziel.clone(),
        ..Default::default()
    };

    // 1. Planen: Pfade prüfen, Ziele bestimmen.
    let mut plan: HashMap<usize, Plan> = HashMap::new();
    let mut gesamt: u64 = 0;
    for (i, e) in inhalt.eintraege.iter().enumerate() {
        if a.auswahl.as_ref().is_some_and(|s| !s.contains(&i)) {
            continue;
        }
        let teile = match &e.sicher {
            Ok(t) => t,
            // «./» in TAR-Archiven: das ist der Zielordner selbst.
            Err(PfadFehler::Leer) if e.art == EintragArt::Ordner => continue,
            Err(f) => {
                bericht.abgelehnt.push((e.name.clone(), pfad_grund(f)));
                continue;
            }
        };
        let Some(ziel) = regeln::zielpfad(&a.ziel, teile, a.weglassen) else {
            continue;
        };
        if !regeln::liegt_innerhalb(&a.ziel, &ziel) {
            bericht.abgelehnt.push((
                e.name.clone(),
                t("liegt ausserhalb des Zielordners").to_string(),
            ));
            continue;
        }
        match e.art {
            EintragArt::Datei => {
                gesamt = gesamt.saturating_add(e.groesse);
                plan.insert(i, Plan::Datei(ziel));
            }
            EintragArt::Ordner => {
                plan.insert(i, Plan::Ordner(ziel));
            }
            EintragArt::Verknuepfung => bericht.uebersprungen.push(tf(
                "{n} (Verknüpfung, wird nicht angelegt)",
                &[("n", &e.name)],
            )),
            EintragArt::Sonstiges => bericht
                .uebersprungen
                .push(tf("{n} (keine normale Datei)", &[("n", &e.name)])),
        }
    }
    if plan.is_empty() {
        return if bericht.abgelehnt.is_empty() && bericht.uebersprungen.is_empty() {
            Err(Fehler::Leer)
        } else {
            Ok(bericht)
        };
    }

    // 2. Konflikte melden, bevor etwas geschrieben wird.
    if a.konflikt == Konflikt::Fragen {
        let mut vorhanden: Vec<PathBuf> = plan
            .values()
            .filter_map(|p| match p {
                Plan::Datei(z) if z.symlink_metadata().is_ok() => Some(z.clone()),
                _ => None,
            })
            .collect();
        if !vorhanden.is_empty() {
            vorhanden.sort();
            let n = vorhanden.len();
            vorhanden.truncate(8);
            return Err(Fehler::Konflikte(n, vorhanden));
        }
    }

    fs::create_dir_all(&a.ziel)?;
    fp.setze_gesamt(gesamt);
    let mut ep = Entpacker {
        fp,
        bericht,
        konflikt: a.konflikt,
        verschluesselt: inhalt.verschluesselt,
    };
    match inhalt.art {
        Art::Zip => zip_entpacken(a, &plan, &mut ep)?,
        Art::SiebenZ => sz_entpacken(a, &plan, &mut ep)?,
        Art::Tar | Art::TarGz => tar_entpacken(a, &plan, &mut ep)?,
        Art::Gz => gz_entpacken(a, &plan, &mut ep)?,
    }
    if fp.abgebrochen() {
        return Err(Fehler::Abgebrochen);
    }
    Ok(ep.bericht)
}

fn zip_entpacken(
    a: &EntpackAuftrag,
    plan: &HashMap<usize, Plan>,
    ep: &mut Entpacker<'_>,
) -> Result<(), Fehler> {
    let f = File::open(&a.inhalt.pfad)?;
    let mut za = zip::ZipArchive::new(BufReader::with_capacity(PUFFER, f)).map_err(zip_fehler)?;
    let mut reihe: Vec<usize> = plan.keys().copied().collect();
    reihe.sort_unstable();
    for i in reihe {
        if ep.fp.abgebrochen() {
            return Err(Fehler::Abgebrochen);
        }
        let (Some(e), Some(p)) = (a.inhalt.eintraege.get(i), plan.get(&i)) else {
            continue;
        };
        match p {
            Plan::Ordner(z) => ep.ordner(&e.name, z),
            Plan::Datei(z) => {
                let geoeffnet = if e.verschluesselt {
                    match a.passwort.as_deref() {
                        Some(pw) if !pw.is_empty() => za.by_index_decrypt(i, pw.as_bytes()),
                        _ => return Err(Fehler::PasswortNoetig),
                    }
                } else {
                    za.by_index(i)
                };
                let mut zf = geoeffnet.map_err(zip_fehler)?;
                ep.datei(e, z, &mut zf)?;
            }
        }
    }
    Ok(())
}

fn sz_entpacken(
    a: &EntpackAuftrag,
    plan: &HashMap<usize, Plan>,
    ep: &mut Entpacker<'_>,
) -> Result<(), Fehler> {
    let pw = a.passwort.as_deref();
    let mit_pw = pw.is_some_and(|p| !p.is_empty());
    if a.inhalt.verschluesselt && !mit_pw {
        return Err(Fehler::PasswortNoetig);
    }
    let mut leser = sevenz_rust2::ArchiveReader::open(&a.inhalt.pfad, sz_passwort(pw))
        .map_err(|e| sz_fehler(e, mit_pw))?;
    // Einträge kommen in Block-Reihenfolge: Namen den Indizes der Liste zuordnen.
    let mut nach_name: HashMap<&str, VecDeque<usize>> = HashMap::new();
    for (i, e) in a.inhalt.eintraege.iter().enumerate() {
        nach_name.entry(e.name.as_str()).or_default().push_back(i);
    }
    let mut offen = plan.len();
    let mut stopp: Option<Fehler> = None;
    let fp = ep.fp;
    let ergebnis = leser.for_each_entries(|entry, reader| {
        if fp.abgebrochen() {
            stopp = Some(Fehler::Abgebrochen);
            return Ok(false);
        }
        let idx = nach_name
            .get_mut(entry.name())
            .and_then(VecDeque::pop_front);
        let geplant = idx.and_then(|i| plan.get(&i).map(|p| (i, p)));
        match geplant {
            Some((i, Plan::Ordner(z))) => {
                if let Some(e) = a.inhalt.eintraege.get(i) {
                    ep.ordner(&e.name, z);
                }
                offen = offen.saturating_sub(1);
            }
            Some((i, Plan::Datei(z))) => {
                if let Some(e) = a.inhalt.eintraege.get(i) {
                    if let Err(f) = ep.datei(e, z, reader) {
                        stopp = Some(f);
                        return Ok(false);
                    }
                    // Nicht gelesene Reste (z. B. übersprungen) verwerfen, damit der Block stimmt.
                    io::copy(reader, &mut io::sink())?;
                }
                offen = offen.saturating_sub(1);
            }
            None => {
                if offen == 0 {
                    return Ok(false);
                }
                // In soliden Blöcken muss der Strom bis zum nächsten Eintrag gelesen werden.
                io::copy(reader, &mut io::sink())?;
            }
        }
        Ok(offen > 0)
    });
    if let Some(f) = stopp {
        return Err(f);
    }
    if fp.abgebrochen() {
        return Err(Fehler::Abgebrochen);
    }
    ergebnis.map_err(|e| sz_fehler(e, mit_pw))
}

fn tar_entpacken(
    a: &EntpackAuftrag,
    plan: &HashMap<usize, Plan>,
    ep: &mut Entpacker<'_>,
) -> Result<(), Fehler> {
    // Der Fortschritt zählt hier die geschriebenen Bytes, der Lesestrom achtet nur auf Abbruch.
    let strom = tar_strom(&a.inhalt.pfad, a.inhalt.art, ep.fp, false)?;
    let mut ar = tar::Archive::new(strom);
    let mut offen = plan.len();
    let eintraege = ar.entries().map_err(|e| lesefehler(e, ep.fp))?;
    for (i, e) in eintraege.enumerate() {
        if ep.fp.abgebrochen() {
            return Err(Fehler::Abgebrochen);
        }
        if offen == 0 {
            break;
        }
        let mut te = e.map_err(|e| lesefehler(e, ep.fp))?;
        let (Some(ein), Some(p)) = (a.inhalt.eintraege.get(i), plan.get(&i)) else {
            continue;
        };
        // Schutz gegen Archive, die sich zwischen Lesen und Entpacken verändert haben.
        if tar_name(&te) != ein.name {
            return Err(Fehler::Kaputt(
                t("Archiv hat sich seit dem Öffnen verändert.").to_string(),
            ));
        }
        match p {
            Plan::Ordner(z) => ep.ordner(&ein.name, z),
            Plan::Datei(z) => ep.datei(ein, z, &mut te)?,
        }
        offen -= 1;
    }
    Ok(())
}

fn gz_entpacken(
    a: &EntpackAuftrag,
    plan: &HashMap<usize, Plan>,
    ep: &mut Entpacker<'_>,
) -> Result<(), Fehler> {
    let (Some(e), Some(Plan::Datei(z))) = (a.inhalt.eintraege.first(), plan.get(&0)) else {
        return Ok(());
    };
    let f = File::open(&a.inhalt.pfad)?;
    let mut strom = flate2::read::MultiGzDecoder::new(BufReader::with_capacity(PUFFER, f));
    ep.datei(e, z, &mut strom)
}

// ---------------------------------------------------------------------------
// Packen
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stufe {
    Schnell,
    Normal,
    Stark,
}

pub struct PackAuftrag {
    pub quellen: Vec<PathBuf>,
    pub ziel: PathBuf,
    pub art: Art,
    pub stufe: Stufe,
    pub passwort: Option<String>,
    /// Bestehende Zieldatei ersetzen (vom Benutzer bestätigt).
    pub ueberschreiben: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PackBericht {
    pub dateien: u64,
    pub ordner: u64,
    pub bytes: u64,
    pub archivgroesse: u64,
    /// (Pfad, Grund)
    pub uebersprungen: Vec<(String, String)>,
    pub ziel: PathBuf,
}

struct Quelle {
    pfad: PathBuf,
    name: String,
    ordner: bool,
    groesse: u64,
    zeit: Option<i64>,
    modus: Option<u32>,
}

#[cfg(unix)]
fn modus_von(m: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(m.permissions().mode())
}
#[cfg(not(unix))]
fn modus_von(_m: &fs::Metadata) -> Option<u32> {
    None
}

fn gleich(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Sammelt alle Dateien und Ordner unter den Quellen (Verknüpfungen werden übersprungen).
fn sammeln(
    a: &PackAuftrag,
    teil: &Path,
    fp: &Fortschritt,
    bericht: &mut PackBericht,
) -> Result<Vec<Quelle>, Fehler> {
    let oben_namen: Vec<String> = a
        .quellen
        .iter()
        .map(|q| {
            q.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".into())
        })
        .collect();
    let oben_namen = regeln::eindeutige_namen(&oben_namen);
    let mut out = Vec::new();
    for (q, oben) in a.quellen.iter().zip(oben_namen) {
        // (Pfad, Archivname)
        let mut stapel: Vec<(PathBuf, String)> = vec![(q.clone(), oben)];
        while let Some((pfad, name)) = stapel.pop() {
            if fp.abgebrochen() {
                return Err(Fehler::Abgebrochen);
            }
            let anzeige = pfad.display().to_string();
            let meta = match fs::symlink_metadata(&pfad) {
                Ok(m) => m,
                Err(e) => {
                    bericht.uebersprungen.push((anzeige, e.to_string()));
                    continue;
                }
            };
            if meta.file_type().is_symlink() {
                bericht
                    .uebersprungen
                    .push((anzeige, t("Verknüpfung").to_string()));
                continue;
            }
            if gleich(&pfad, &a.ziel) || gleich(&pfad, teil) {
                continue;
            }
            let zeit = meta.modified().ok().and_then(unix_von);
            if meta.is_dir() {
                out.push(Quelle {
                    pfad: pfad.clone(),
                    name: name.clone(),
                    ordner: true,
                    groesse: 0,
                    zeit,
                    modus: modus_von(&meta),
                });
                let mut kinder: Vec<(PathBuf, String)> = match fs::read_dir(&pfad) {
                    Ok(rd) => rd
                        .flatten()
                        .map(|d| {
                            let n = d.file_name().to_string_lossy().into_owned();
                            (d.path(), regeln::archiv_eintragsname(&name, &[n]))
                        })
                        .collect(),
                    Err(e) => {
                        bericht.uebersprungen.push((anzeige, e.to_string()));
                        continue;
                    }
                };
                // Rückwärts sortiert auf den Stapel, damit die Reihenfolge im Archiv stimmt.
                kinder.sort_by(|x, y| anzeige::natuerlich(&y.1, &x.1));
                stapel.extend(kinder);
            } else if meta.is_file() {
                out.push(Quelle {
                    pfad,
                    name,
                    ordner: false,
                    groesse: meta.len(),
                    zeit,
                    modus: modus_von(&meta),
                });
            } else {
                bericht
                    .uebersprungen
                    .push((anzeige, t("keine normale Datei").to_string()));
            }
        }
    }
    Ok(out)
}

/// Erstellt ein neues Archiv. Geschrieben wird in eine Teil-Datei, die erst am Ende
/// den richtigen Namen bekommt.
pub fn packen(a: &PackAuftrag, fp: &Fortschritt) -> Result<PackBericht, Fehler> {
    if a.quellen.is_empty() {
        return Err(Fehler::Leer);
    }
    if !a.ueberschreiben && a.ziel.symlink_metadata().is_ok() {
        return Err(Fehler::ZielExistiert(a.ziel.clone()));
    }
    if a.ziel.is_dir() {
        return Err(Fehler::ZielExistiert(a.ziel.clone()));
    }
    let mut bericht = PackBericht {
        ziel: a.ziel.clone(),
        ..Default::default()
    };
    let teil = teil_pfad(&a.ziel);
    fp.setze_aktuell(t("Dateien werden gesammelt …"));
    let quellen = sammeln(a, &teil, fp, &mut bericht)?;
    if quellen.is_empty() {
        return Err(Fehler::Leer);
    }
    if a.art == Art::Gz && (quellen.len() != 1 || quellen[0].ordner) {
        return Err(Fehler::GzNurEineDatei);
    }
    let gesamt = quellen
        .iter()
        .map(|q| q.groesse)
        .fold(0u64, u64::saturating_add);
    fp.setze_gesamt(gesamt);

    let ergebnis = (|| -> Result<(), Fehler> {
        let datei = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&teil)?;
        match a.art {
            Art::Zip => zip_packen(datei, &quellen, a, fp, &mut bericht),
            Art::SiebenZ => sz_packen(datei, &quellen, a, fp, &mut bericht),
            Art::Tar => {
                let w = tar_packen(
                    BufWriter::with_capacity(PUFFER, datei),
                    &quellen,
                    fp,
                    &mut bericht,
                )?;
                fertig(w)
            }
            Art::TarGz => {
                let gz = flate2::write::GzEncoder::new(
                    BufWriter::with_capacity(PUFFER, datei),
                    gz_stufe(a.stufe),
                );
                let gz = tar_packen(gz, &quellen, fp, &mut bericht)?;
                fertig(gz.finish().map_err(|e| schreibfehler(e, fp))?)
            }
            Art::Gz => {
                let q = &quellen[0];
                fp.setze_aktuell(&q.name);
                let mut gz = flate2::write::GzEncoder::new(
                    BufWriter::with_capacity(PUFFER, datei),
                    gz_stufe(a.stufe),
                );
                let mut r = Zaehler {
                    inner: File::open(&q.pfad)?,
                    fp,
                    zaehlen: true,
                };
                io::copy(&mut r, &mut gz).map_err(|e| schreibfehler(e, fp))?;
                bericht.dateien = 1;
                bericht.bytes = q.groesse;
                fertig(gz.finish().map_err(|e| schreibfehler(e, fp))?)
            }
        }
    })();
    if let Err(e) = ergebnis {
        let _ = fs::remove_file(&teil);
        return Err(if fp.abgebrochen() {
            Fehler::Abgebrochen
        } else {
            e
        });
    }
    if fp.abgebrochen() {
        let _ = fs::remove_file(&teil);
        return Err(Fehler::Abgebrochen);
    }
    if let Err(e) = fs::rename(&teil, &a.ziel) {
        let _ = fs::remove_file(&teil);
        return Err(e.into());
    }
    bericht.archivgroesse = fs::metadata(&a.ziel).map(|m| m.len()).unwrap_or(0);
    Ok(bericht)
}

fn schreibfehler(e: io::Error, fp: &Fortschritt) -> Fehler {
    if fp.abgebrochen() {
        Fehler::Abgebrochen
    } else {
        Fehler::Io(e.to_string())
    }
}

fn fertig(w: BufWriter<File>) -> Result<(), Fehler> {
    let f = w
        .into_inner()
        .map_err(|e| Fehler::Io(e.into_error().to_string()))?;
    f.sync_all()?;
    Ok(())
}

fn gz_stufe(s: Stufe) -> flate2::Compression {
    flate2::Compression::new(match s {
        Stufe::Schnell => 1,
        Stufe::Normal => 6,
        Stufe::Stark => 9,
    })
}

fn zip_datum(unix: i64) -> Option<zip::DateTime> {
    let z = ortszeit(unix)?;
    let jahr = u16::try_from(z.jahr).ok()?;
    zip::DateTime::from_date_and_time(jahr, z.monat, z.tag, z.stunde, z.minute, z.sekunde).ok()
}

fn zip_packen(
    datei: File,
    quellen: &[Quelle],
    a: &PackAuftrag,
    fp: &Fortschritt,
    bericht: &mut PackBericht,
) -> Result<(), Fehler> {
    use zip::write::SimpleFileOptions;
    let mut zw = zip::ZipWriter::new(BufWriter::with_capacity(PUFFER, datei));
    let stufe: i64 = match a.stufe {
        Stufe::Schnell => 1,
        Stufe::Normal => 6,
        Stufe::Stark => 9,
    };
    let passwort = a.passwort.as_deref().filter(|p| !p.is_empty());
    for q in quellen {
        if fp.abgebrochen() {
            return Err(Fehler::Abgebrochen);
        }
        fp.setze_aktuell(&q.name);
        let mut opt = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(stufe))
            .large_file(q.groesse >= 0xFFFF_FFFF);
        if let Some(d) = q.zeit.and_then(zip_datum) {
            opt = opt.last_modified_time(d);
        }
        if let Some(m) = q.modus {
            opt = opt.unix_permissions(m & 0o7777);
        }
        if q.ordner {
            zw.add_directory(q.name.clone(), opt).map_err(zip_fehler)?;
            bericht.ordner += 1;
            continue;
        }
        let quelle = match File::open(&q.pfad) {
            Ok(f) => f,
            Err(e) => {
                bericht
                    .uebersprungen
                    .push((q.pfad.display().to_string(), e.to_string()));
                continue;
            }
        };
        let begonnen = match passwort {
            Some(pw) => zw.start_file(
                q.name.clone(),
                opt.with_aes_encryption(zip::AesMode::Aes256, pw),
            ),
            None => zw.start_file(q.name.clone(), opt),
        };
        begonnen.map_err(zip_fehler)?;
        let mut r = Zaehler {
            inner: quelle,
            fp,
            zaehlen: true,
        };
        let n = io::copy(&mut r, &mut zw).map_err(|e| schreibfehler(e, fp))?;
        bericht.dateien += 1;
        bericht.bytes = bericht.bytes.saturating_add(n);
    }
    let w = zw.finish().map_err(zip_fehler)?;
    fertig(w)
}

/// Öffnet die Datei erst beim ersten Lesen und schliesst sie am Ende (viele Dateien in
/// einem soliden 7z-Block, ohne alle gleichzeitig offen zu halten).
struct FaulLeser<'a> {
    pfad: PathBuf,
    datei: Option<File>,
    fertig: bool,
    fp: &'a Fortschritt,
}

impl Read for FaulLeser<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.fp.abgebrochen() {
            return Err(io::Error::other("abgebrochen"));
        }
        if self.fertig {
            return Ok(0);
        }
        if self.datei.is_none() {
            self.fp.setze_aktuell(
                &self
                    .pfad
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
            self.datei = Some(File::open(&self.pfad)?);
        }
        let n = match self.datei.as_mut() {
            Some(f) => f.read(buf)?,
            None => 0,
        };
        if n == 0 {
            self.fertig = true;
            self.datei = None;
        }
        self.fp.plus(n as u64);
        Ok(n)
    }
}

fn sz_eintrag(q: &Quelle) -> sevenz_rust2::ArchiveEntry {
    sevenz_rust2::ArchiveEntry::from_path(&q.pfad, q.name.clone())
}

fn sz_packen(
    datei: File,
    quellen: &[Quelle],
    a: &PackAuftrag,
    fp: &Fortschritt,
    bericht: &mut PackBericht,
) -> Result<(), Fehler> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    use sevenz_rust2::{ArchiveWriter, EncoderConfiguration, SourceReader};
    let fehler = |e: sevenz_rust2::Error| {
        if fp.abgebrochen() {
            Fehler::Abgebrochen
        } else {
            Fehler::Io(e.to_string())
        }
    };

    let mut sw = ArchiveWriter::new(datei).map_err(fehler)?;
    let stufe = match a.stufe {
        Stufe::Schnell => 1,
        Stufe::Normal => 5,
        Stufe::Stark => 9,
    };
    let mut methoden: Vec<EncoderConfiguration> = Vec::new();
    let passwort = a.passwort.as_deref().filter(|p| !p.is_empty());
    if let Some(pw) = passwort {
        methoden.push(AesEncoderOptions::new(sevenz_rust2::Password::new(pw)).into());
    }
    methoden.push(Lzma2Options::from_level(stufe).into());
    sw.set_content_methods(methoden);
    sw.set_encrypt_header(passwort.is_some());

    let mut eintraege: Vec<sevenz_rust2::ArchiveEntry> = Vec::new();
    let mut leser: Vec<SourceReader<FaulLeser<'_>>> = Vec::new();
    let mut blockgroesse: u64 = 0;
    for q in quellen {
        if fp.abgebrochen() {
            return Err(Fehler::Abgebrochen);
        }
        if q.ordner {
            sw.push_archive_entry::<&[u8]>(sz_eintrag(q), None)
                .map_err(fehler)?;
            bericht.ordner += 1;
            continue;
        }
        let faul = FaulLeser {
            pfad: q.pfad.clone(),
            datei: None,
            fertig: false,
            fp,
        };
        if q.groesse >= SZ_BLOCK_MAX {
            sw.push_archive_entry(sz_eintrag(q), Some(faul))
                .map_err(fehler)?;
        } else {
            if blockgroesse.saturating_add(q.groesse) >= SZ_BLOCK_MAX && !eintraege.is_empty() {
                sw.push_archive_entries(std::mem::take(&mut eintraege), std::mem::take(&mut leser))
                    .map_err(fehler)?;
                blockgroesse = 0;
            }
            blockgroesse = blockgroesse.saturating_add(q.groesse);
            eintraege.push(sz_eintrag(q));
            leser.push(SourceReader::new(faul));
        }
        bericht.dateien += 1;
        bericht.bytes = bericht.bytes.saturating_add(q.groesse);
    }
    if !eintraege.is_empty() {
        sw.push_archive_entries(eintraege, leser).map_err(fehler)?;
    }
    let f = sw.finish().map_err(|e| schreibfehler(e, fp))?;
    f.sync_all()?;
    Ok(())
}

fn tar_packen<W: Write>(
    w: W,
    quellen: &[Quelle],
    fp: &Fortschritt,
    bericht: &mut PackBericht,
) -> Result<W, Fehler> {
    let mut b = tar::Builder::new(w);
    for q in quellen {
        if fp.abgebrochen() {
            return Err(Fehler::Abgebrochen);
        }
        fp.setze_aktuell(&q.name);
        let mut h = tar::Header::new_gnu();
        h.set_mtime(q.zeit.and_then(|z| u64::try_from(z).ok()).unwrap_or(0));
        if q.ordner {
            h.set_entry_type(tar::EntryType::Directory);
            h.set_size(0);
            h.set_mode(q.modus.map(|m| m & 0o7777).unwrap_or(0o755));
            b.append_data(&mut h, &q.name, io::empty())
                .map_err(|e| schreibfehler(e, fp))?;
            bericht.ordner += 1;
            continue;
        }
        let quelle = match File::open(&q.pfad) {
            Ok(f) => f,
            Err(e) => {
                bericht
                    .uebersprungen
                    .push((q.pfad.display().to_string(), e.to_string()));
                continue;
            }
        };
        h.set_entry_type(tar::EntryType::Regular);
        h.set_size(q.groesse);
        h.set_mode(q.modus.map(|m| m & 0o7777).unwrap_or(0o644));
        // Genau `groesse` Bytes liefern, auch wenn sich die Datei inzwischen geändert hat.
        let r = Zaehler {
            inner: quelle,
            fp,
            zaehlen: true,
        }
        .chain(io::repeat(0))
        .take(q.groesse);
        b.append_data(&mut h, &q.name, r)
            .map_err(|e| schreibfehler(e, fp))?;
        bericht.dateien += 1;
        bericht.bytes = bericht.bytes.saturating_add(q.groesse);
    }
    b.into_inner().map_err(|e| schreibfehler(e, fp))
}

// ---------------------------------------------------------------------------
// Ordner im Dateimanager zeigen
// ---------------------------------------------------------------------------

pub fn ordner_zeigen(pfad: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("explorer")
            .arg(pfad)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(pfad).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(pfad).spawn();
    }
}
