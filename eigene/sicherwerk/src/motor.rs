//! Motor: Ordner lesen, Spiegeln, Versionen (Ordner oder ZIP), Aufbewahrung, Prüfen per
//! Stichprobe und Wiederherstellen. Läuft in einem Hintergrund-Thread (Fenster) oder direkt
//! (Zeitplan), meldet den Stand über `Fortschritt` und lässt sich abbrechen.
//!
//! Grundsatz: In den Quellordnern wird nie etwas geschrieben, verschoben oder gelöscht.
//! Geschrieben wird nur im Auftragsordner im Ziel (und beim Wiederherstellen in einem frei
//! gewählten Ordner, ohne etwas zu überschreiben).
//!
//! Nur std; Test mit echten Ordnern im Temp-Verzeichnis:
//! `rustc --edition 2021 --cfg ohne_serde --test src/motor.rs`.

#![allow(dead_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auftrag::{ordnername, Art, Auftrag, Entfernte, Ergebnis, Fehler, FehlerArt, Quelle, VersionForm, MAX_FEHLER_JE_LAUF};
use crate::plan::Eintrag;
use crate::regeln::Regeln;

// Für den Einzeltest mit rustc ist diese Datei die Wurzel und bindet die Module selbst ein.
#[cfg(test)]
#[path = "auftrag.rs"]
mod auftrag;
#[cfg(test)]
#[path = "plan.rs"]
mod plan;
#[cfg(test)]
#[path = "pruefsumme.rs"]
mod pruefsumme;
#[cfg(test)]
#[path = "regeln.rs"]
mod regeln;
#[cfg(test)]
#[path = "zipper.rs"]
mod zipper;

/// Name des Ordners für entfernte Dateien beim Spiegeln.
pub const ENTFERNT: &str = "_entfernt";
const TMP_ENDUNG: &str = ".sicherwerk-tmp";
/// Grenze der Ordnertiefe (Schutz vor Endlosschleifen).
const MAX_TIEFE: u32 = 400;
/// Höchstzahl Einträge je Liste in der Vorschau.
const MAX_VORSCHAU: usize = 2000;

// ------------------------------------------------------------------ Fortschritt

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Start,
    Lesen,
    Vergleichen,
    Kopieren,
    Aufraeumen,
    Pruefen,
    Wiederherstellen,
    Fertig,
}

impl Phase {
    const ALLE: [Phase; 8] = [
        Phase::Start,
        Phase::Lesen,
        Phase::Vergleichen,
        Phase::Kopieren,
        Phase::Aufraeumen,
        Phase::Pruefen,
        Phase::Wiederherstellen,
        Phase::Fertig,
    ];
    pub fn text(self) -> &'static str {
        match self {
            Phase::Start => "Vorbereiten …",
            Phase::Lesen => "Ordner lesen …",
            Phase::Vergleichen => "Vergleichen …",
            Phase::Kopieren => "Kopieren …",
            Phase::Aufraeumen => "Aufräumen …",
            Phase::Pruefen => "Prüfsummen vergleichen …",
            Phase::Wiederherstellen => "Wiederherstellen …",
            Phase::Fertig => "Fertig",
        }
    }
}

/// Gemeinsamer Stand zwischen Arbeits-Thread und Oberfläche.
#[derive(Default)]
pub struct Fortschritt {
    abbrechen: AtomicBool,
    phase: AtomicU8,
    pub dateien_gesamt: AtomicU64,
    pub dateien_fertig: AtomicU64,
    pub bytes_gesamt: AtomicU64,
    pub bytes_fertig: AtomicU64,
    aktuell: Mutex<String>,
}

impl Fortschritt {
    pub fn neu() -> Arc<Fortschritt> {
        Arc::new(Fortschritt::default())
    }
    pub fn abbrechen(&self) {
        self.abbrechen.store(true, Ordering::SeqCst);
    }
    pub fn abgebrochen(&self) -> bool {
        self.abbrechen.load(Ordering::SeqCst)
    }
    pub fn phase(&self) -> Phase {
        Phase::ALLE.get(self.phase.load(Ordering::Relaxed) as usize).copied().unwrap_or(Phase::Start)
    }
    fn setze_phase(&self, p: Phase) {
        let i = Phase::ALLE.iter().position(|x| *x == p).unwrap_or(0);
        self.phase.store(i as u8, Ordering::Relaxed);
    }
    pub fn aktuell(&self) -> String {
        match self.aktuell.lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }
    fn setze_aktuell(&self, s: &str) {
        match self.aktuell.lock() {
            Ok(mut g) => {
                g.clear();
                g.push_str(s);
            }
            Err(p) => {
                let mut g = p.into_inner();
                g.clear();
                g.push_str(s);
            }
        }
    }
    fn zaehler_neu(&self, dateien: u64, bytes: u64) {
        self.dateien_gesamt.store(dateien, Ordering::Relaxed);
        self.bytes_gesamt.store(bytes, Ordering::Relaxed);
        self.dateien_fertig.store(0, Ordering::Relaxed);
        self.bytes_fertig.store(0, Ordering::Relaxed);
    }
    fn datei_fertig(&self) {
        self.dateien_fertig.fetch_add(1, Ordering::Relaxed);
    }
    fn bytes_dazu(&self, n: u64) {
        self.bytes_fertig.fetch_add(n, Ordering::Relaxed);
    }
    /// Anteil 0..1, wenn bekannt.
    pub fn anteil(&self) -> Option<f32> {
        let bg = self.bytes_gesamt.load(Ordering::Relaxed);
        let dg = self.dateien_gesamt.load(Ordering::Relaxed);
        let df = self.dateien_fertig.load(Ordering::Relaxed);
        let bf = self.bytes_fertig.load(Ordering::Relaxed);
        if bg > 0 {
            // Bytes und Dateien mischen, damit viele kleine Dateien auch vorwärts gehen.
            let b = (bf as f64 / bg as f64).min(1.0);
            let d = if dg > 0 { (df as f64 / dg as f64).min(1.0) } else { b };
            Some((0.8 * b + 0.2 * d) as f32)
        } else if dg > 0 {
            Some((df as f64 / dg as f64).min(1.0) as f32)
        } else {
            None
        }
    }
}

/// Werte von aussen (Uhrzeit kommt aus der Oberfläche, damit der Motor ohne Zeitzonen-Crate auskommt).
#[derive(Clone, Debug)]
pub struct Umgebung {
    /// Ortszeit «2026-10-18_14-30-05» – Name neuer Versionen und von `_entfernt/<datum>`.
    pub jetzt_name: String,
    pub jetzt_unix: i64,
    /// Windows: Gross-/Kleinschreibung in Pfaden egal.
    pub gross_klein_egal: bool,
    /// Unix-Zeit → DOS-Zeit (Ortszeit) für ZIP.
    pub dos_zeit: fn(i64) -> (u16, u16),
}

// ------------------------------------------------------------------ Ergebnis

#[derive(Clone, Debug, Default)]
pub struct Bilanz {
    pub neu: u64,
    pub geaendert: u64,
    pub entfernt: u64,
    pub verknuepft: u64,
    pub unveraendert: u64,
    pub bytes: u64,
    pub versionen_geloescht: u64,
    pub geprueft: u64,
    pub version: String,
    /// Fehler (gezählt) und Hinweise (nicht gezählt), höchstens MAX_FEHLER_JE_LAUF.
    pub fehler: Vec<Fehler>,
    pub fehler_anzahl: u64,
    pub abgebrochen: bool,
    /// Lauf konnte gar nicht arbeiten (Ziel fehlt …).
    pub fatal: bool,
}

impl Bilanz {
    fn fehler(&mut self, f: Fehler) {
        self.fehler_anzahl += 1;
        if self.fehler.len() < MAX_FEHLER_JE_LAUF {
            self.fehler.push(f);
        }
    }
    fn hinweis(&mut self, text: &str, pfad: &str) {
        if self.fehler.len() < MAX_FEHLER_JE_LAUF {
            self.fehler.push(Fehler::neu(FehlerArt::Hinweis, pfad, text));
        }
    }
    fn fatal(&mut self, f: Fehler) {
        self.fatal = true;
        self.fehler(f);
    }
    pub fn ergebnis(&self) -> Ergebnis {
        if self.fatal {
            Ergebnis::Fehlgeschlagen
        } else if self.abgebrochen {
            Ergebnis::Abgebrochen
        } else if self.fehler_anzahl > 0 {
            Ergebnis::MitFehlern
        } else {
            Ergebnis::Ok
        }
    }
}

/// Was ein Lauf tun würde.
#[derive(Clone, Debug, Default)]
pub struct Vorschau {
    pub art: Option<Art>,
    pub ordner: String,
    /// (Pfad, Grösse) – höchstens MAX_VORSCHAU je Liste.
    pub neu: Vec<(String, u64)>,
    pub ersetzen: Vec<(String, u64)>,
    pub entfernen: Vec<String>,
    pub neu_anzahl: u64,
    pub ersetzen_anzahl: u64,
    pub entfernen_anzahl: u64,
    pub gleich: u64,
    pub verknuepfen: u64,
    /// Zu kopierende Bytes.
    pub bytes: u64,
    /// Dateien gesamt (Versionen).
    pub dateien: u64,
    pub version: String,
    pub versionen_weg: Vec<String>,
    pub fehler: Vec<Fehler>,
    pub fehler_anzahl: u64,
    pub abgebrochen: bool,
    pub fatal: bool,
}

// ------------------------------------------------------------------ Hilfen

fn unix_sek(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_secs()).map(|s| -s).unwrap_or(i64::MIN),
    }
}

fn zeit_aus_unix(s: i64) -> Option<SystemTime> {
    if s >= 0 {
        UNIX_EPOCH.checked_add(std::time::Duration::from_secs(s as u64))
    } else {
        UNIX_EPOCH.checked_sub(std::time::Duration::from_secs(s.unsigned_abs()))
    }
}

fn meta_zeit(m: &fs::Metadata) -> i64 {
    m.modified().map(unix_sek).unwrap_or(0)
}

/// Pfad aus Basis und relativem Pfad mit `/`.
pub fn rel_pfad(basis: &Path, rel: &str) -> PathBuf {
    let mut p = basis.to_path_buf();
    for t in rel.split('/').filter(|t| !t.is_empty()) {
        p.push(t);
    }
    p
}

fn anzeigen(p: &Path) -> String {
    p.display().to_string()
}

fn io_text(e: &io::Error) -> String {
    e.to_string()
}

/// Auftragsordner im Ziel: `<ziel>/<Auftragsname>`.
pub fn auftrag_ordner(a: &Auftrag) -> PathBuf {
    Path::new(a.ziel.trim()).join(ordnername(&a.name))
}

/// Ist das Ziel (Laufwerk/Ordner) erreichbar?
pub fn ziel_da(a: &Auftrag) -> bool {
    let z = a.ziel.trim();
    !z.is_empty() && Path::new(z).is_dir()
}

fn ziel_pruefen(a: &Auftrag) -> Result<PathBuf, Fehler> {
    let z = a.ziel.trim();
    if z.is_empty() {
        return Err(Fehler::neu(FehlerArt::ZielFehlt, "", "Kein Ziel gewählt"));
    }
    if !Path::new(z).is_dir() {
        return Err(Fehler::neu(FehlerArt::ZielFehlt, z, "Laufwerk nicht angeschlossen oder Ordner nicht vorhanden"));
    }
    Ok(auftrag_ordner(a))
}

/// Pfad als Vergleichsform (absolut, wenn möglich aufgelöst).
fn aufgeloest(p: &Path) -> PathBuf {
    if let Ok(c) = fs::canonicalize(p) {
        return c;
    }
    // Noch nicht vorhanden: Eltern auflösen und den Rest anhängen.
    if let (Some(eltern), Some(name)) = (p.parent(), p.file_name()) {
        if let Ok(c) = fs::canonicalize(eltern) {
            return c.join(name);
        }
    }
    p.to_path_buf()
}

fn gleicher_pfad(a: &Path, b: &Path, gross_klein_egal: bool) -> bool {
    if gross_klein_egal {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

fn liegt_in(innen: &Path, aussen: &Path, gross_klein_egal: bool) -> bool {
    innen.ancestors().any(|x| gleicher_pfad(x, aussen, gross_klein_egal))
}

/// Liegt der Auftragsordner in einer Quelle oder eine Quelle im Auftragsordner? Dann würde
/// Sicherwerk in die Quelle schreiben – das wird abgelehnt.
fn ueberschneidung(a: &Auftrag, ord: &Path, gke: bool) -> Option<Fehler> {
    let o = aufgeloest(ord);
    for q in &a.quellen {
        let qp = aufgeloest(Path::new(q.pfad.trim()));
        if liegt_in(&o, &qp, gke) {
            return Some(Fehler::neu(FehlerArt::Sonstiges, &q.pfad, "Das Ziel liegt in diesem Quellordner – bitte ein anderes Ziel wählen"));
        }
        if liegt_in(&qp, &o, gke) {
            return Some(Fehler::neu(FehlerArt::Sonstiges, &q.pfad, "Dieser Quellordner liegt im Sicherungsordner"));
        }
    }
    None
}

// ------------------------------------------------------------------ Lesen

#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub dateien: Vec<Eintrag>,
    pub ordner: Vec<String>,
    /// Pfade, die nicht vollständig gelesen werden konnten («» = alles).
    pub schutz: Vec<String>,
}

/// Liest einen Ordner rekursiv (ohne symbolischen Verknüpfungen zu folgen).
/// `auslassen`: relative Pfade, die übersprungen werden. `ist_ziel`: Reste abgebrochener
/// Kopien (`*.sicherwerk-tmp`) werden dann gelöscht und nicht aufgeführt.
pub fn scannen(
    wurzel: &Path,
    regeln: &Regeln,
    auslassen: &[&str],
    ist_ziel: bool,
    fs_: &Fortschritt,
    fehler: &mut dyn FnMut(Fehler),
) -> Scan {
    let mut scan = Scan::default();
    let mut stapel: Vec<(PathBuf, String, u32)> = vec![(wurzel.to_path_buf(), String::new(), 0)];
    let mut zaehler = 0u64;
    while let Some((pfad, rel, tiefe)) = stapel.pop() {
        if fs_.abgebrochen() {
            return scan;
        }
        let rd = match fs::read_dir(&pfad) {
            Ok(r) => r,
            Err(e) => {
                fehler(Fehler::neu(FehlerArt::Lesen, anzeigen(&pfad), io_text(&e)));
                scan.schutz.push(rel.clone());
                continue;
            }
        };
        for eintrag in rd {
            let e = match eintrag {
                Ok(e) => e,
                Err(err) => {
                    fehler(Fehler::neu(FehlerArt::Lesen, anzeigen(&pfad), io_text(&err)));
                    scan.schutz.push(rel.clone());
                    continue;
                }
            };
            let name_os = e.file_name();
            let Some(name) = name_os.to_str() else {
                fehler(Fehler::neu(
                    FehlerArt::Lesen,
                    anzeigen(&e.path()),
                    "Dateiname mit ungültigen Zeichen – übersprungen",
                ));
                continue;
            };
            let r = if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") };
            if auslassen.iter().any(|a| *a == r) {
                continue;
            }
            let ft = match e.file_type() {
                Ok(t) => t,
                Err(err) => {
                    fehler(Fehler::neu(FehlerArt::Lesen, anzeigen(&e.path()), io_text(&err)));
                    scan.schutz.push(r);
                    continue;
                }
            };
            if ft.is_symlink() {
                continue;
            }
            if ist_ziel && name.ends_with(TMP_ENDUNG) && ft.is_file() {
                let _ = fs::remove_file(e.path());
                continue;
            }
            if regeln.ausgeschlossen(&r) {
                continue;
            }
            zaehler += 1;
            if zaehler % 200 == 0 {
                fs_.setze_aktuell(&r);
            }
            if ft.is_dir() {
                if tiefe >= MAX_TIEFE {
                    fehler(Fehler::neu(FehlerArt::Lesen, anzeigen(&e.path()), "Ordner zu tief verschachtelt"));
                    scan.schutz.push(r);
                    continue;
                }
                scan.ordner.push(r.clone());
                stapel.push((e.path(), r, tiefe + 1));
            } else if ft.is_file() {
                match e.metadata() {
                    Ok(m) => {
                        scan.dateien.push(Eintrag { rel: r, groesse: m.len(), zeit: meta_zeit(&m) });
                        fs_.dateien_gesamt.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(err) => {
                        fehler(Fehler::neu(FehlerArt::Lesen, anzeigen(&e.path()), io_text(&err)));
                        scan.schutz.push(r);
                    }
                }
            }
        }
    }
    scan.dateien.sort_by(|a, b| a.rel.cmp(&b.rel));
    scan.ordner.sort();
    scan
}

// ------------------------------------------------------------------ Kopieren

fn tmp_pfad(ziel: &Path) -> PathBuf {
    let name = ziel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let kurz: String = name.chars().take(120).collect();
    ziel.with_file_name(format!(".{kurz}{TMP_ENDUNG}"))
}

/// Kopiert Daten blockweise; bricht bei `abbrechen` mit `Interrupted` ab.
fn strom<R: Read, W: Write>(r: &mut R, w: &mut W, fs_: &Fortschritt) -> io::Result<u64> {
    let mut buf = vec![0u8; 1024 * 1024];
    let mut n_ges = 0u64;
    loop {
        if fs_.abgebrochen() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
        }
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        w.write_all(buf.get(..n).unwrap_or(&[]))?;
        n_ges = n_ges.saturating_add(n as u64);
        fs_.bytes_dazu(n as u64);
    }
    Ok(n_ges)
}

/// Ersetzt `ziel` durch `tmp` (Windows: auch schreibgeschützte Ziele).
fn ersetzen(tmp: &Path, ziel: &Path) -> io::Result<()> {
    match fs::rename(tmp, ziel) {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Ok(m) = fs::metadata(ziel) {
                let mut p = m.permissions();
                if p.readonly() {
                    #[allow(clippy::permissions_set_readonly_false)]
                    p.set_readonly(false);
                    if fs::set_permissions(ziel, p).is_ok() {
                        return fs::rename(tmp, ziel);
                    }
                }
            }
            Err(e)
        }
    }
}

/// Kopiert `quelle` nach `ziel` über eine Zwischendatei (das Ziel ist nie halb geschrieben)
/// und übernimmt die Änderungszeit. Liefert die Anzahl Bytes.
pub fn kopieren(quelle: &Path, ziel: &Path, fs_: &Fortschritt) -> io::Result<u64> {
    let mut ein = File::open(quelle)?;
    let meta = ein.metadata()?;
    let tmp = tmp_pfad(ziel);
    let ergebnis = (|| -> io::Result<u64> {
        let mut aus = File::create(&tmp)?;
        let n = strom(&mut ein, &mut aus, fs_)?;
        aus.flush()?;
        if let Ok(t) = meta.modified() {
            let _ = aus.set_modified(t);
        }
        Ok(n)
    })();
    match ergebnis {
        Ok(n) => {
            if let Err(e) = ersetzen(&tmp, ziel) {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
            Ok(n)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Schreibt in eine neue Datei (nie überschreiben). Bei Fehler/Abbruch wird sie entfernt.
fn neu_schreiben<R: Read>(r: &mut R, ziel: &Path, zeit: Option<i64>, fs_: &Fortschritt) -> io::Result<u64> {
    let mut aus = OpenOptions::new().write(true).create_new(true).open(ziel)?;
    let ergebnis = (|| -> io::Result<u64> {
        let n = strom(r, &mut aus, fs_)?;
        aus.flush()?;
        if let Some(t) = zeit.and_then(zeit_aus_unix) {
            let _ = aus.set_modified(t);
        }
        Ok(n)
    })();
    if ergebnis.is_err() {
        drop(aus);
        let _ = fs::remove_file(ziel);
    }
    ergebnis
}

// ------------------------------------------------------------------ Spiegeln

struct QuellPlan {
    quelle: Quelle,
    wurzel: PathBuf,
    ziel_wurzel: PathBuf,
    scan_q: Scan,
    scan_z: Scan,
    plan: crate::plan::SpiegelPlan,
    /// Quelle leer, Ziel nicht: nichts entfernen.
    leer_schutz: bool,
}

fn quelle_ok(q: &Quelle, b: &mut Bilanz) -> Option<PathBuf> {
    let p = PathBuf::from(q.pfad.trim());
    if q.pfad.trim().is_empty() || !p.is_dir() {
        b.fehler(Fehler::neu(FehlerArt::QuelleFehlt, &q.pfad, "Ordner nicht gefunden – nichts davon gesichert, im Ziel nichts entfernt"));
        return None;
    }
    Some(p)
}

fn spiegel_planen(a: &Auftrag, ord: &Path, u: &Umgebung, fs_: &Fortschritt, b: &mut Bilanz) -> Vec<QuellPlan> {
    let regeln = Regeln::neu(&a.ausschluesse);
    let mut out = Vec::new();
    fs_.setze_phase(Phase::Lesen);
    for q in &a.quellen {
        if fs_.abgebrochen() {
            break;
        }
        let Some(wurzel) = quelle_ok(q, b) else { continue };
        let ziel_wurzel = ord.join(&q.name);
        let mut fe = |f: Fehler| b.fehler(f);
        let scan_q = scannen(&wurzel, &regeln, &[], false, fs_, &mut fe);
        let scan_z = if ziel_wurzel.is_dir() {
            scannen(&ziel_wurzel, &regeln, &[], true, fs_, &mut fe)
        } else {
            Scan::default()
        };
        let mut schutz = scan_q.schutz.clone();
        // Schutz: Ein leerer Quellordner (z. B. Netzlaufwerk nicht eingebunden) räumt das Ziel nicht leer.
        let leer_schutz = scan_q.dateien.is_empty() && scan_q.ordner.is_empty() && !scan_z.dateien.is_empty();
        if leer_schutz {
            schutz.push(String::new());
        }
        fs_.setze_phase(Phase::Vergleichen);
        let p = crate::plan::spiegeln(&scan_q.dateien, &scan_q.ordner, &scan_z.dateien, &scan_z.ordner, &schutz, u.gross_klein_egal);
        out.push(QuellPlan { quelle: q.clone(), wurzel, ziel_wurzel, scan_q, scan_z, plan: p, leer_schutz });
        fs_.setze_phase(Phase::Lesen);
    }
    out
}

fn spiegeln_ausfuehren(a: &Auftrag, ord: &Path, u: &Umgebung, fs_: &Fortschritt, b: &mut Bilanz) {
    let plaene = spiegel_planen(a, ord, u, fs_, b);
    if fs_.abgebrochen() {
        b.abgebrochen = true;
        return;
    }
    let dateien: u64 = plaene.iter().map(|p| (p.plan.neu.len() + p.plan.ersetzen.len()) as u64).sum();
    let bytes: u64 = plaene.iter().map(|p| p.plan.bytes(&p.scan_q.dateien)).sum();
    fs_.zaehler_neu(dateien, bytes);
    for p in &plaene {
        if p.leer_schutz {
            b.hinweis("Quellordner ist leer – im Ziel wurde nichts entfernt", &p.quelle.pfad);
        }
        b.unveraendert += p.plan.gleich;
        // 1. Entfernte Dateien wegräumen (nur im Ziel).
        if a.entfernte != Entfernte::Behalten {
            fs_.setze_phase(Phase::Aufraeumen);
            let ablage = ord.join(ENTFERNT).join(&u.jetzt_name).join(&p.quelle.name);
            for &zi in &p.plan.entfernen {
                if fs_.abgebrochen() {
                    b.abgebrochen = true;
                    return;
                }
                let Some(z) = p.scan_z.dateien.get(zi) else { continue };
                let von = rel_pfad(&p.ziel_wurzel, &z.rel);
                fs_.setze_aktuell(&z.rel);
                match a.entfernte {
                    Entfernte::Verschieben => {
                        let nach = rel_pfad(&ablage, &z.rel);
                        let r = nach
                            .parent()
                            .map(fs::create_dir_all)
                            .unwrap_or(Ok(()))
                            .and_then(|_| fs::rename(&von, &nach));
                        match r {
                            Ok(()) => b.entfernt += 1,
                            Err(e) => b.fehler(Fehler::neu(FehlerArt::Verschieben, anzeigen(&von), io_text(&e))),
                        }
                    }
                    Entfernte::Loeschen => match fs::remove_file(&von) {
                        Ok(()) => b.entfernt += 1,
                        Err(e) => b.fehler(Fehler::neu(FehlerArt::Loeschen, anzeigen(&von), io_text(&e))),
                    },
                    Entfernte::Behalten => {}
                }
            }
            // Leere Ordner, die es in der Quelle nicht mehr gibt (nur wenn wirklich leer).
            for o in &p.plan.ordner_weg {
                let _ = fs::remove_dir(rel_pfad(&p.ziel_wurzel, o));
            }
        }
        // 2. Ordner anlegen.
        fs_.setze_phase(Phase::Kopieren);
        if let Err(e) = fs::create_dir_all(&p.ziel_wurzel) {
            b.fehler(Fehler::neu(FehlerArt::OrdnerAnlegen, anzeigen(&p.ziel_wurzel), io_text(&e)));
            continue;
        }
        for o in &p.plan.ordner_neu {
            let z = rel_pfad(&p.ziel_wurzel, o);
            if let Err(e) = fs::create_dir_all(&z) {
                b.fehler(Fehler::neu(FehlerArt::OrdnerAnlegen, anzeigen(&z), io_text(&e)));
            }
        }
        // 3. Neues und Geändertes kopieren.
        let liste = p.plan.neu.iter().map(|i| (*i, true)).chain(p.plan.ersetzen.iter().map(|i| (*i, false)));
        for (i, ist_neu) in liste {
            let Some(e) = p.scan_q.dateien.get(i) else { continue };
            fs_.setze_aktuell(&e.rel);
            let von = rel_pfad(&p.wurzel, &e.rel);
            let nach = rel_pfad(&p.ziel_wurzel, &e.rel);
            if let Some(eltern) = nach.parent() {
                if !eltern.is_dir() {
                    let _ = fs::create_dir_all(eltern);
                }
            }
            match kopieren(&von, &nach, fs_) {
                Ok(n) => {
                    b.bytes = b.bytes.saturating_add(n);
                    if ist_neu {
                        b.neu += 1;
                    } else {
                        b.geaendert += 1;
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted && fs_.abgebrochen() => {
                    b.abgebrochen = true;
                    return;
                }
                Err(err) => b.fehler(Fehler::neu(FehlerArt::Kopieren, anzeigen(&von), io_text(&err))),
            }
            fs_.datei_fertig();
        }
    }
}

fn spiegel_vorschau(a: &Auftrag, ord: &Path, u: &Umgebung, fs_: &Fortschritt, v: &mut Vorschau) {
    let mut b = Bilanz::default();
    let plaene = spiegel_planen(a, ord, u, fs_, &mut b);
    for p in &plaene {
        let pre = &p.quelle.name;
        for &i in &p.plan.neu {
            if let Some(e) = p.scan_q.dateien.get(i) {
                v.neu_anzahl += 1;
                v.bytes = v.bytes.saturating_add(e.groesse);
                if v.neu.len() < MAX_VORSCHAU {
                    v.neu.push((format!("{pre}/{}", e.rel), e.groesse));
                }
            }
        }
        for &i in &p.plan.ersetzen {
            if let Some(e) = p.scan_q.dateien.get(i) {
                v.ersetzen_anzahl += 1;
                v.bytes = v.bytes.saturating_add(e.groesse);
                if v.ersetzen.len() < MAX_VORSCHAU {
                    v.ersetzen.push((format!("{pre}/{}", e.rel), e.groesse));
                }
            }
        }
        if a.entfernte != Entfernte::Behalten {
            for &i in &p.plan.entfernen {
                if let Some(e) = p.scan_z.dateien.get(i) {
                    v.entfernen_anzahl += 1;
                    if v.entfernen.len() < MAX_VORSCHAU {
                        v.entfernen.push(format!("{pre}/{}", e.rel));
                    }
                }
            }
        }
        if p.leer_schutz {
            b.hinweis("Quellordner ist leer – im Ziel wurde nichts entfernt", &p.quelle.pfad);
        }
        v.gleich += p.plan.gleich;
        v.dateien += p.scan_q.dateien.len() as u64;
    }
    v.fehler = b.fehler;
    v.fehler_anzahl = b.fehler_anzahl;
}

// ------------------------------------------------------------------ Versionen

fn eintraege(ord: &Path) -> Vec<String> {
    match fs::read_dir(ord) {
        Ok(rd) => rd.flatten().filter_map(|e| e.file_name().to_str().map(|s| s.to_string())).collect(),
        Err(_) => Vec::new(),
    }
}

struct VersQuelle {
    quelle: Quelle,
    wurzel: PathBuf,
    scan: Scan,
}

fn versionen_lesen(a: &Auftrag, fs_: &Fortschritt, b: &mut Bilanz) -> Vec<VersQuelle> {
    let regeln = Regeln::neu(&a.ausschluesse);
    fs_.setze_phase(Phase::Lesen);
    let mut out = Vec::new();
    for q in &a.quellen {
        if fs_.abgebrochen() {
            break;
        }
        let Some(wurzel) = quelle_ok(q, b) else { continue };
        let mut fe = |f: Fehler| b.fehler(f);
        let scan = scannen(&wurzel, &regeln, &[], false, fs_, &mut fe);
        out.push(VersQuelle { quelle: q.clone(), wurzel, scan });
    }
    out
}

/// Liegt in der letzten Ordner-Version dieselbe Datei (Grösse+Zeit)? Dann Pfad dorthin.
fn vorige_datei(vorige: Option<&Path>, q: &str, e: &Eintrag) -> Option<PathBuf> {
    let v = vorige?;
    let p = rel_pfad(&v.join(q), &e.rel);
    let m = fs::symlink_metadata(&p).ok()?;
    if m.is_file() && crate::plan::gleich(m.len(), meta_zeit(&m), e.groesse, e.zeit) {
        Some(p)
    } else {
        None
    }
}

fn versionen_ausfuehren(a: &Auftrag, ord: &Path, u: &Umgebung, fs_: &Fortschritt, b: &mut Bilanz) {
    let quellen = versionen_lesen(a, fs_, b);
    if fs_.abgebrochen() {
        b.abgebrochen = true;
        return;
    }
    let vorh = eintraege(ord);
    let fertige = crate::plan::versionen(&vorh);
    let vorige = fertige.iter().rev().find(|n| !n.ends_with(".zip")).map(|n| ord.join(n));
    let name = crate::plan::neuer_versionsname(&u.jetzt_name, &vorh);
    let dateien: u64 = quellen.iter().map(|q| q.scan.dateien.len() as u64).sum();
    let bytes: u64 = quellen.iter().flat_map(|q| q.scan.dateien.iter()).fold(0u64, |s, e| s.saturating_add(e.groesse));
    fs_.zaehler_neu(dateien, bytes);
    fs_.setze_phase(Phase::Kopieren);

    let endname = match a.form {
        VersionForm::Ordner => name.clone(),
        VersionForm::Zip => format!("{name}.zip"),
    };
    let arbeit = ord.join(format!("{endname}{}", crate::plan::UNVOLLSTAENDIG));
    match a.form {
        VersionForm::Ordner => {
            if let Err(e) = fs::create_dir(&arbeit) {
                b.fatal(Fehler::neu(FehlerArt::OrdnerAnlegen, anzeigen(&arbeit), io_text(&e)));
                return;
            }
            let mut links_gehen = a.verknuepfen;
            'quellen: for q in &quellen {
                let basis = arbeit.join(&q.quelle.name);
                if let Err(e) = fs::create_dir_all(&basis) {
                    b.fehler(Fehler::neu(FehlerArt::OrdnerAnlegen, anzeigen(&basis), io_text(&e)));
                    continue;
                }
                for o in &q.scan.ordner {
                    let z = rel_pfad(&basis, o);
                    if let Err(e) = fs::create_dir_all(&z) {
                        b.fehler(Fehler::neu(FehlerArt::OrdnerAnlegen, anzeigen(&z), io_text(&e)));
                    }
                }
                for e in &q.scan.dateien {
                    if fs_.abgebrochen() {
                        b.abgebrochen = true;
                        break 'quellen;
                    }
                    fs_.setze_aktuell(&e.rel);
                    let nach = rel_pfad(&basis, &e.rel);
                    if links_gehen {
                        if let Some(alt) = vorige_datei(vorige.as_deref(), &q.quelle.name, e) {
                            match fs::hard_link(&alt, &nach) {
                                Ok(()) => {
                                    b.verknuepft += 1;
                                    fs_.bytes_dazu(e.groesse);
                                    fs_.datei_fertig();
                                    continue;
                                }
                                Err(_) => {
                                    // Laufwerk kann keine harten Verknüpfungen (FAT, exFAT, manche NAS).
                                    links_gehen = false;
                                    b.hinweis("Das Ziellaufwerk kann keine harten Verknüpfungen – es wird alles kopiert", "");
                                }
                            }
                        }
                    }
                    let von = rel_pfad(&q.wurzel, &e.rel);
                    match kopieren(&von, &nach, fs_) {
                        Ok(n) => {
                            b.bytes = b.bytes.saturating_add(n);
                            b.neu += 1;
                        }
                        Err(err) if err.kind() == io::ErrorKind::Interrupted && fs_.abgebrochen() => {
                            b.abgebrochen = true;
                            break 'quellen;
                        }
                        Err(err) => b.fehler(Fehler::neu(FehlerArt::Kopieren, anzeigen(&von), io_text(&err))),
                    }
                    fs_.datei_fertig();
                }
            }
        }
        VersionForm::Zip => {
            let datei = match OpenOptions::new().write(true).create_new(true).open(&arbeit) {
                Ok(f) => f,
                Err(e) => {
                    b.fatal(Fehler::neu(FehlerArt::Zip, anzeigen(&arbeit), io_text(&e)));
                    return;
                }
            };
            let mut z = match crate::zipper::ZipSchreiber::neu(io::BufWriter::with_capacity(1 << 20, datei)) {
                Ok(z) => z,
                Err(e) => {
                    b.fatal(Fehler::neu(FehlerArt::Zip, anzeigen(&arbeit), io_text(&e)));
                    let _ = fs::remove_file(&arbeit);
                    return;
                }
            };
            'q: for q in &quellen {
                for o in &q.scan.ordner {
                    if let Err(e) = z.ordner(&format!("{}/{o}", q.quelle.name), (u.dos_zeit)(u.jetzt_unix), u.jetzt_unix) {
                        b.fehler(Fehler::neu(FehlerArt::Zip, o, io_text(&e)));
                    }
                }
                for e in &q.scan.dateien {
                    if fs_.abgebrochen() {
                        b.abgebrochen = true;
                        break 'q;
                    }
                    fs_.setze_aktuell(&e.rel);
                    let von = rel_pfad(&q.wurzel, &e.rel);
                    let mut f = match File::open(&von) {
                        Ok(f) => f,
                        Err(err) => {
                            b.fehler(Fehler::neu(FehlerArt::Lesen, anzeigen(&von), io_text(&err)));
                            fs_.datei_fertig();
                            continue;
                        }
                    };
                    let name = format!("{}/{}", q.quelle.name, e.rel);
                    let r = z.datei(&name, &mut f, (u.dos_zeit)(e.zeit), e.zeit, |n| {
                        fs_.bytes_dazu(n);
                        !fs_.abgebrochen()
                    });
                    match r {
                        Ok((n, _)) => {
                            b.bytes = b.bytes.saturating_add(n);
                            b.neu += 1;
                        }
                        Err(err) if err.kind() == io::ErrorKind::Interrupted && fs_.abgebrochen() => {
                            b.abgebrochen = true;
                            break 'q;
                        }
                        Err(err) => b.fehler(Fehler::neu(FehlerArt::Zip, anzeigen(&von), io_text(&err))),
                    }
                    fs_.datei_fertig();
                }
            }
            if !b.abgebrochen {
                let r = z.fertig().and_then(|(w, ende)| {
                    let f = w.into_inner().map_err(|e| e.into_error())?;
                    f.set_len(ende)?;
                    f.sync_all()
                });
                if let Err(e) = r {
                    b.fatal(Fehler::neu(FehlerArt::Zip, anzeigen(&arbeit), io_text(&e)));
                }
            }
        }
    }
    // Abbruch oder schwerer Fehler: die halbe Version entfernen.
    if b.abgebrochen || b.fatal {
        fs_.setze_phase(Phase::Aufraeumen);
        let _ = if arbeit.is_dir() { fs::remove_dir_all(&arbeit) } else { fs::remove_file(&arbeit) };
        return;
    }
    let fertig_pfad = ord.join(&endname);
    if let Err(e) = fs::rename(&arbeit, &fertig_pfad) {
        b.fatal(Fehler::neu(FehlerArt::Sonstiges, anzeigen(&arbeit), io_text(&e)));
        return;
    }
    b.version = endname.clone();
    // Aufbewahrung – nur nach einem fehlerfreien Lauf, damit nie gute alte Versionen für
    // eine lückenhafte neue weichen.
    fs_.setze_phase(Phase::Aufraeumen);
    if b.fehler_anzahl > 0 {
        b.hinweis("Wegen Fehlern wurden keine alten Versionen entfernt", "");
        return;
    }
    let mut alle = fertige.clone();
    alle.push(endname);
    for alt in crate::plan::zu_loeschen(&alle, a.behalten) {
        if crate::plan::versionsname(&alt).is_none() {
            continue;
        }
        let p = ord.join(&alt);
        fs_.setze_aktuell(&alt);
        let r = if alt.ends_with(".zip") { fs::remove_file(&p) } else { fs::remove_dir_all(&p) };
        match r {
            Ok(()) => b.versionen_geloescht += 1,
            Err(e) => b.fehler(Fehler::neu(FehlerArt::Loeschen, anzeigen(&p), io_text(&e))),
        }
    }
    // Reste abgebrochener Läufe (z. B. nach Stromausfall).
    for rest in vorh.iter().filter(|n| n.ends_with(crate::plan::UNVOLLSTAENDIG)) {
        let stamm = rest.trim_end_matches(crate::plan::UNVOLLSTAENDIG);
        if crate::plan::versionsname(stamm).is_none() {
            continue;
        }
        let p = ord.join(rest);
        let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
    }
}

fn versionen_vorschau(a: &Auftrag, ord: &Path, u: &Umgebung, fs_: &Fortschritt, v: &mut Vorschau) {
    let mut b = Bilanz::default();
    let quellen = versionen_lesen(a, fs_, &mut b);
    let vorh = eintraege(ord);
    let fertige = crate::plan::versionen(&vorh);
    let vorige = fertige.iter().rev().find(|n| !n.ends_with(".zip")).map(|n| ord.join(n));
    let name = crate::plan::neuer_versionsname(&u.jetzt_name, &vorh);
    let endname = if a.form == VersionForm::Zip { format!("{name}.zip") } else { name };
    fs_.setze_phase(Phase::Vergleichen);
    for q in &quellen {
        for e in &q.scan.dateien {
            v.dateien += 1;
            if a.form == VersionForm::Ordner && a.verknuepfen && vorige_datei(vorige.as_deref(), &q.quelle.name, e).is_some() {
                v.verknuepfen += 1;
                continue;
            }
            v.bytes = v.bytes.saturating_add(e.groesse);
            v.neu_anzahl += 1;
            if v.neu.len() < MAX_VORSCHAU {
                v.neu.push((format!("{}/{}", q.quelle.name, e.rel), e.groesse));
            }
        }
    }
    let mut alle = fertige;
    alle.push(endname.clone());
    v.versionen_weg = crate::plan::zu_loeschen(&alle, a.behalten);
    v.version = endname;
    v.fehler = b.fehler;
    v.fehler_anzahl = b.fehler_anzahl;
}

// ------------------------------------------------------------------ Öffentliche Läufe

fn vorbereiten(a: &Auftrag, u: &Umgebung, b: &mut Bilanz) -> Option<PathBuf> {
    let ord = match ziel_pruefen(a) {
        Ok(o) => o,
        Err(f) => {
            b.fatal(f);
            return None;
        }
    };
    if a.quellen.is_empty() {
        b.fatal(Fehler::neu(FehlerArt::QuelleFehlt, "", "Keine Quellordner gewählt"));
        return None;
    }
    if let Some(f) = ueberschneidung(a, &ord, u.gross_klein_egal) {
        b.fatal(f);
        return None;
    }
    Some(ord)
}

/// Führt eine Sicherung aus.
pub fn sichern(a: &Auftrag, u: &Umgebung, fs_: &Fortschritt) -> Bilanz {
    let mut b = Bilanz::default();
    fs_.setze_phase(Phase::Start);
    let Some(ord) = vorbereiten(a, u, &mut b) else {
        fs_.setze_phase(Phase::Fertig);
        return b;
    };
    if let Err(e) = fs::create_dir_all(&ord) {
        b.fatal(Fehler::neu(FehlerArt::OrdnerAnlegen, anzeigen(&ord), io_text(&e)));
        fs_.setze_phase(Phase::Fertig);
        return b;
    }
    match a.art {
        Art::Spiegeln => spiegeln_ausfuehren(a, &ord, u, fs_, &mut b),
        Art::Versionen => versionen_ausfuehren(a, &ord, u, fs_, &mut b),
    }
    if fs_.abgebrochen() {
        b.abgebrochen = true;
    }
    fs_.setze_phase(Phase::Fertig);
    b
}

/// Zeigt, was `sichern` tun würde – ohne etwas zu schreiben.
pub fn vorschau(a: &Auftrag, u: &Umgebung, fs_: &Fortschritt) -> Vorschau {
    let mut v = Vorschau { art: Some(a.art), ..Default::default() };
    let mut b = Bilanz::default();
    fs_.setze_phase(Phase::Start);
    let Some(ord) = vorbereiten(a, u, &mut b) else {
        v.fehler = b.fehler;
        v.fehler_anzahl = b.fehler_anzahl;
        v.fatal = true;
        fs_.setze_phase(Phase::Fertig);
        return v;
    };
    v.ordner = anzeigen(&ord);
    match a.art {
        Art::Spiegeln => spiegel_vorschau(a, &ord, u, fs_, &mut v),
        Art::Versionen => versionen_vorschau(a, &ord, u, fs_, &mut v),
    }
    v.abgebrochen = fs_.abgebrochen();
    fs_.setze_phase(Phase::Fertig);
    v
}

// ------------------------------------------------------------------ Stände (Wiederherstellen)

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stand {
    /// Der aktuelle Spiegel.
    Spiegel,
    /// Beim Spiegeln weggeräumte Dateien eines Laufs.
    Entfernt(String),
    /// Versionsordner.
    Ordner(String),
    /// Versions-ZIP.
    Zip(String),
}

impl Stand {
    /// Datum/Name für die Anzeige.
    pub fn name(&self) -> &str {
        match self {
            Stand::Spiegel => "",
            Stand::Entfernt(n) | Stand::Ordner(n) | Stand::Zip(n) => n,
        }
    }
}

/// Alle Stände eines Auftrags, neueste zuerst.
pub fn staende(a: &Auftrag) -> Result<Vec<Stand>, Fehler> {
    let ord = ziel_pruefen(a)?;
    if !ord.is_dir() {
        return Ok(Vec::new());
    }
    let vorh = eintraege(&ord);
    let mut out = Vec::new();
    // Spiegel: Unterordner der Quellen vorhanden?
    let spiegel_da = vorh.iter().any(|n| crate::plan::versionsname(n).is_none() && n != ENTFERNT && !n.ends_with(crate::plan::UNVOLLSTAENDIG) && ord.join(n).is_dir());
    if spiegel_da {
        out.push(Stand::Spiegel);
    }
    for v in crate::plan::versionen(&vorh).into_iter().rev() {
        if v.ends_with(".zip") {
            out.push(Stand::Zip(v));
        } else {
            out.push(Stand::Ordner(v));
        }
    }
    let mut entf: Vec<String> = eintraege(&ord.join(ENTFERNT)).into_iter().filter(|n| crate::plan::versionsname(n).is_some()).collect();
    entf.sort();
    for e in entf.into_iter().rev() {
        out.push(Stand::Entfernt(e));
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandEintrag {
    pub rel: String,
    pub groesse: u64,
    pub zeit: i64,
    pub ordner: bool,
}

fn stand_wurzel(ord: &Path, s: &Stand) -> PathBuf {
    match s {
        Stand::Spiegel => ord.to_path_buf(),
        Stand::Entfernt(n) => ord.join(ENTFERNT).join(n),
        Stand::Ordner(n) | Stand::Zip(n) => ord.join(n),
    }
}

/// Inhalt eines Stands (Dateien und Ordner).
pub fn stand_inhalt(a: &Auftrag, s: &Stand, fs_: &Fortschritt) -> Result<Vec<StandEintrag>, Fehler> {
    let ord = ziel_pruefen(a)?;
    let wurzel = stand_wurzel(&ord, s);
    fs_.setze_phase(Phase::Lesen);
    if let Stand::Zip(_) = s {
        let mut f = File::open(&wurzel).map_err(|e| Fehler::neu(FehlerArt::Zip, anzeigen(&wurzel), io_text(&e)))?;
        let inh = crate::zipper::verzeichnis(&mut f).map_err(|e| Fehler::neu(FehlerArt::Zip, anzeigen(&wurzel), io_text(&e)))?;
        let mut out: Vec<StandEintrag> = inh
            .into_iter()
            .filter(|e| crate::plan::sicherer_rel(&e.name).is_some())
            .map(|e| StandEintrag {
                rel: e.name.trim_end_matches('/').to_string(),
                groesse: e.groesse,
                zeit: e.unix.unwrap_or(0),
                ordner: e.ordner,
            })
            .collect();
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        fs_.setze_phase(Phase::Fertig);
        return Ok(out);
    }
    if !wurzel.is_dir() {
        return Err(Fehler::neu(FehlerArt::Lesen, anzeigen(&wurzel), "Nicht gefunden"));
    }
    // Im Auftragsordner liegen beim Spiegeln auch _entfernt und evtl. alte Versionen.
    let vorh = eintraege(&ord);
    let mut auslassen: Vec<&str> = Vec::new();
    if *s == Stand::Spiegel {
        auslassen.push(ENTFERNT);
        auslassen.extend(
            vorh.iter()
                .filter(|n| crate::plan::versionsname(n).is_some() || n.ends_with(crate::plan::UNVOLLSTAENDIG))
                .map(|n| n.as_str()),
        );
    }
    let mut fe = |_f: Fehler| {};
    let scan = scannen(&wurzel, &Regeln::default(), &auslassen, true, fs_, &mut fe);
    fs_.setze_phase(Phase::Fertig);
    Ok(scan_zu_eintraegen(scan))
}

fn scan_zu_eintraegen(scan: Scan) -> Vec<StandEintrag> {
    let mut out: Vec<StandEintrag> = scan
        .ordner
        .into_iter()
        .map(|o| StandEintrag { rel: o, groesse: 0, zeit: 0, ordner: true })
        .chain(scan.dateien.into_iter().map(|d| StandEintrag { rel: d.rel, groesse: d.groesse, zeit: d.zeit, ordner: false }))
        .collect();
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// Gehört `rel` zur Auswahl `sel` (gleich oder darunter)?
fn unter(rel: &str, sel: &str) -> bool {
    rel == sel || (rel.len() > sel.len() && rel.starts_with(sel) && rel.as_bytes().get(sel.len()) == Some(&b'/'))
}

/// Auswahl ohne Einträge, die schon in einem ausgewählten Ordner stecken.
pub fn auswahl_bereinigen(auswahl: &[String]) -> Vec<String> {
    let mut v: Vec<String> = auswahl.iter().map(|s| s.trim_end_matches('/').to_string()).filter(|s| !s.is_empty()).collect();
    v.sort();
    v.dedup();
    let kopie = v.clone();
    v.retain(|s| !kopie.iter().any(|o| o != s && unter(s, o)));
    v
}

/// Stellt die Auswahl (Dateien und Ordner eines Stands) in `ziel` wieder her. Jeder gewählte
/// Eintrag landet direkt in `ziel` (ohne die Ordner darüber); gibt es den Namen dort schon,
/// heisst er «Name (2)». Es wird nie etwas überschrieben.
pub fn wiederherstellen(a: &Auftrag, s: &Stand, auswahl: &[String], ziel: &Path, fs_: &Fortschritt) -> Bilanz {
    let mut b = Bilanz::default();
    fs_.setze_phase(Phase::Lesen);
    if !ziel.is_dir() {
        b.fatal(Fehler::neu(FehlerArt::ZielFehlt, anzeigen(ziel), "Ordner nicht gefunden"));
        return b;
    }
    let inhalt = match stand_inhalt(a, s, fs_) {
        Ok(i) => i,
        Err(f) => {
            b.fatal(f);
            return b;
        }
    };
    let Ok(ord) = ziel_pruefen(a) else {
        b.fatal(Fehler::neu(FehlerArt::ZielFehlt, &a.ziel, "Laufwerk nicht angeschlossen oder Ordner nicht vorhanden"));
        return b;
    };
    let wurzel = stand_wurzel(&ord, s);
    let auswahl = auswahl_bereinigen(auswahl);
    // Für jede Auswahl: Elternteil abschneiden und freien Namen im Ziel suchen.
    struct Ziel<'a> {
        sel: &'a str,
        eltern_len: usize,
        neuer_name: String,
    }
    let mut ziele: Vec<Ziel> = Vec::new();
    let mut vergeben: Vec<String> = Vec::new();
    for sel in &auswahl {
        let (eltern_len, name) = match sel.rfind('/') {
            Some(p) => (p + 1, &sel[p + 1..]),
            None => (0, sel.as_str()),
        };
        let neuer = crate::plan::freier_name(name, |n| ziel.join(n).exists() || vergeben.iter().any(|v| v.eq_ignore_ascii_case(n)));
        vergeben.push(neuer.clone());
        ziele.push(Ziel { sel, eltern_len, neuer_name: neuer });
    }
    // Arbeitsliste
    let mut arbeit: Vec<(&StandEintrag, PathBuf)> = Vec::new();
    for e in &inhalt {
        let Some(z) = ziele.iter().find(|z| unter(&e.rel, z.sel)) else { continue };
        if crate::plan::sicherer_rel(&e.rel).is_none() {
            b.fehler(Fehler::neu(FehlerArt::Sonstiges, &e.rel, "Ungültiger Pfad in der Sicherung – übersprungen"));
            continue;
        }
        let rest = e.rel.get(z.eltern_len..).unwrap_or("");
        let rest_ohne_erstes = match rest.find('/') {
            Some(p) => &rest[p..],
            None => "",
        };
        let rel_neu = format!("{}{}", z.neuer_name, rest_ohne_erstes);
        arbeit.push((e, rel_pfad(ziel, &rel_neu)));
    }
    let dateien = arbeit.iter().filter(|(e, _)| !e.ordner).count() as u64;
    let bytes = arbeit.iter().filter(|(e, _)| !e.ordner).fold(0u64, |s, (e, _)| s.saturating_add(e.groesse));
    fs_.zaehler_neu(dateien, bytes);
    fs_.setze_phase(Phase::Wiederherstellen);
    let mut zip: Option<(File, Vec<crate::zipper::ZipEintrag>, std::collections::HashMap<String, usize>)> = None;
    if let Stand::Zip(_) = s {
        match File::open(&wurzel).and_then(|mut f| crate::zipper::verzeichnis(&mut f).map(|v| (f, v))) {
            Ok((f, v)) => {
                let index = v.iter().enumerate().map(|(i, e)| (e.name.clone(), i)).collect();
                zip = Some((f, v, index));
            }
            Err(e) => {
                b.fatal(Fehler::neu(FehlerArt::Zip, anzeigen(&wurzel), io_text(&e)));
                return b;
            }
        }
    }
    for (e, nach) in &arbeit {
        if fs_.abgebrochen() {
            b.abgebrochen = true;
            break;
        }
        fs_.setze_aktuell(&e.rel);
        if e.ordner {
            if let Err(err) = fs::create_dir_all(nach) {
                b.fehler(Fehler::neu(FehlerArt::OrdnerAnlegen, anzeigen(nach), io_text(&err)));
            }
            continue;
        }
        if let Some(eltern) = nach.parent() {
            if let Err(err) = fs::create_dir_all(eltern) {
                b.fehler(Fehler::neu(FehlerArt::OrdnerAnlegen, anzeigen(eltern), io_text(&err)));
                continue;
            }
        }
        let r = match &mut zip {
            Some((f, inh, index)) => {
                let Some(ze) = index.get(&e.rel).and_then(|&i| inh.get(i)) else { continue };
                let crc_soll = ze.crc;
                let zeit = ze.unix;
                match crate::zipper::eintrag_leser(f, ze) {
                    Ok(leser) => {
                        let mut pr = PruefLeser { inner: leser, crc: crate::pruefsumme::Crc32::neu() };
                        let r = neu_schreiben(&mut pr, nach, zeit, fs_);
                        match r {
                            Ok(n) if pr.crc.wert() != crc_soll => {
                                let _ = n;
                                b.fehler(Fehler::neu(FehlerArt::Abweichung, &e.rel, "Daten im ZIP beschädigt (CRC)"));
                                Ok(n)
                            }
                            r => r,
                        }
                    }
                    Err(err) => Err(err),
                }
            }
            None => {
                let von = rel_pfad(&wurzel, &e.rel);
                File::open(&von).and_then(|mut f| neu_schreiben(&mut f, nach, Some(e.zeit), fs_))
            }
        };
        match r {
            Ok(n) => {
                b.neu += 1;
                b.bytes = b.bytes.saturating_add(n);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted && fs_.abgebrochen() => {
                b.abgebrochen = true;
                break;
            }
            Err(err) => b.fehler(Fehler::neu(FehlerArt::Kopieren, &e.rel, io_text(&err))),
        }
        fs_.datei_fertig();
    }
    b.version = s.name().to_string();
    fs_.setze_phase(Phase::Fertig);
    b
}

/// Leser, der nebenbei die CRC-32 rechnet.
struct PruefLeser<R: Read> {
    inner: R,
    crc: crate::pruefsumme::Crc32,
}

impl<R: Read> Read for PruefLeser<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.crc.update(buf.get(..n).unwrap_or(&[]));
        Ok(n)
    }
}

// ------------------------------------------------------------------ Prüfen

/// Prüft eine Stichprobe: liest Dateien aus der Sicherung (bei ZIP mit CRC) und vergleicht
/// sie per SHA-256 mit der Quelle, sofern diese seither unverändert ist (Grösse+Zeit).
pub fn pruefen(a: &Auftrag, u: &Umgebung, fs_: &Fortschritt) -> Bilanz {
    let mut b = Bilanz::default();
    fs_.setze_phase(Phase::Lesen);
    let ord = match ziel_pruefen(a) {
        Ok(o) => o,
        Err(f) => {
            b.fatal(f);
            return b;
        }
    };
    if !ord.is_dir() {
        b.fatal(Fehler::neu(FehlerArt::Lesen, anzeigen(&ord), "Noch keine Sicherung vorhanden"));
        return b;
    }
    // Kandidaten: (Name in der Sicherung, Quelle-Pfad, Grösse, Zeit)
    struct Kand {
        rel: String,
        quelle: Option<PathBuf>,
        groesse: u64,
        zeit: i64,
    }
    let quelle_fuer = |rel: &str| -> Option<PathBuf> {
        let (kopf, rest) = rel.split_once('/')?;
        let q = a.quellen.iter().find(|q| q.name == kopf)?;
        Some(rel_pfad(Path::new(q.pfad.trim()), rest))
    };
    let mut kand: Vec<Kand> = Vec::new();
    let mut zip: Option<(File, Vec<crate::zipper::ZipEintrag>)> = None;
    let wurzel: PathBuf = match a.art {
        Art::Spiegeln => ord.clone(),
        Art::Versionen => {
            let Some(v) = crate::plan::versionen(&eintraege(&ord)).pop() else {
                b.fatal(Fehler::neu(FehlerArt::Lesen, anzeigen(&ord), "Noch keine Version vorhanden"));
                return b;
            };
            b.version = v.clone();
            ord.join(v)
        }
    };
    if b.version.ends_with(".zip") {
        match File::open(&wurzel).and_then(|mut f| crate::zipper::verzeichnis(&mut f).map(|v| (f, v))) {
            Ok((f, inh)) => {
                for e in inh.iter().filter(|e| !e.ordner) {
                    kand.push(Kand { rel: e.name.clone(), quelle: quelle_fuer(&e.name), groesse: e.groesse, zeit: e.unix.unwrap_or(0) });
                }
                zip = Some((f, inh));
            }
            Err(e) => {
                b.fehler(Fehler::neu(FehlerArt::Zip, anzeigen(&wurzel), io_text(&e)));
                return b;
            }
        }
    } else {
        for q in &a.quellen {
            let w = wurzel.join(&q.name);
            if !w.is_dir() {
                continue;
            }
            let mut fe = |f: Fehler| b.fehler(f);
            let scan = scannen(&w, &Regeln::default(), &[], true, fs_, &mut fe);
            for d in scan.dateien {
                let rel = format!("{}/{}", q.name, d.rel);
                kand.push(Kand { quelle: quelle_fuer(&rel), rel, groesse: d.groesse, zeit: d.zeit });
            }
        }
    }
    let mut zufall = crate::plan::Zufall::neu((u.jetzt_unix as u64) ^ a.id.rotate_left(32) ^ 0x9E37_79B9_7F4A_7C15);
    let wahl = zufall.auswahl(kand.len(), a.stichprobe.max(1) as usize);
    let bytes: u64 = wahl.iter().filter_map(|&i| kand.get(i)).fold(0u64, |s, k| s.saturating_add(k.groesse));
    fs_.zaehler_neu(wahl.len() as u64, bytes);
    fs_.setze_phase(Phase::Pruefen);
    let weiter = |n: u64| {
        fs_.bytes_dazu(n);
        !fs_.abgebrochen()
    };
    for &i in &wahl {
        let Some(k) = kand.get(i) else { continue };
        if fs_.abgebrochen() {
            b.abgebrochen = true;
            break;
        }
        fs_.setze_aktuell(&k.rel);
        // Sicherung lesen.
        let gelesen = match &mut zip {
            Some((f, inh)) => match inh.iter().find(|e| e.name == k.rel) {
                Some(e) => {
                    let soll = e.crc;
                    crate::zipper::eintrag_leser(f, e).and_then(|mut r| crate::pruefsumme::lesen(&mut r, weiter)).map(|(sha, crc, n)| {
                        if crc != soll {
                            b.fehler(Fehler::neu(FehlerArt::Abweichung, &k.rel, "Daten im ZIP beschädigt (CRC)"));
                        }
                        (sha, n)
                    })
                }
                None => continue,
            },
            None => {
                let p = rel_pfad(&wurzel, &k.rel);
                File::open(&p).and_then(|mut f| crate::pruefsumme::lesen(&mut f, weiter)).map(|(sha, _, n)| (sha, n))
            }
        };
        let (sha_sich, n) = match gelesen {
            Ok(x) => x,
            Err(e) if e.kind() == io::ErrorKind::Interrupted && fs_.abgebrochen() => {
                b.abgebrochen = true;
                break;
            }
            Err(e) => {
                b.fehler(Fehler::neu(FehlerArt::Lesen, &k.rel, io_text(&e)));
                fs_.datei_fertig();
                continue;
            }
        };
        if n != k.groesse {
            b.fehler(Fehler::neu(FehlerArt::Abweichung, &k.rel, "Grösse stimmt nicht"));
        }
        b.geprueft += 1;
        // Mit der Quelle vergleichen, wenn sie seither unverändert ist.
        if let Some(qp) = &k.quelle {
            if let Ok(m) = fs::metadata(qp) {
                if m.is_file() && crate::plan::gleich(m.len(), meta_zeit(&m), k.groesse, k.zeit) {
                    match File::open(qp).and_then(|mut f| crate::pruefsumme::lesen(&mut f, |_| !fs_.abgebrochen())) {
                        Ok((sha_q, _, _)) => {
                            if sha_q != sha_sich {
                                b.fehler(Fehler::neu(FehlerArt::Abweichung, &k.rel, "Inhalt weicht von der Quelle ab"));
                            } else {
                                b.unveraendert += 1;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted && fs_.abgebrochen() => {
                            b.abgebrochen = true;
                            break;
                        }
                        Err(e) => b.fehler(Fehler::neu(FehlerArt::Lesen, anzeigen(qp), io_text(&e))),
                    }
                }
            }
        }
        fs_.datei_fertig();
    }
    fs_.setze_phase(Phase::Fertig);
    b
}

// ------------------------------------------------------------------ Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auftrag::Zeitplan;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sicherwerk-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn schreib(p: &Path, inhalt: &str, zeit: i64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let f = File::create(p).unwrap();
        (&f).write_all(inhalt.as_bytes()).unwrap();
        f.set_modified(zeit_aus_unix(zeit).unwrap()).unwrap();
    }

    fn lies(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    fn umg(name: &str) -> Umgebung {
        Umgebung { jetzt_name: name.into(), jetzt_unix: 1_700_000_000, gross_klein_egal: false, dos_zeit: crate::zipper::dos_aus_unix_utc }
    }

    /// Alle Dateien mit Inhalt und Zeit – um zu beweisen, dass die Quelle unverändert bleibt.
    fn abbild(d: &Path) -> Vec<(String, String, i64)> {
        let fs_ = Fortschritt::default();
        let mut fe = |_f: Fehler| {};
        let s = scannen(d, &Regeln::default(), &[], false, &fs_, &mut fe);
        s.dateien.iter().map(|e| (e.rel.clone(), lies(&rel_pfad(d, &e.rel)), e.zeit)).collect()
    }

    fn auftrag(q: &Path, ziel: &Path, art: Art) -> Auftrag {
        Auftrag {
            id: 1,
            name: "Test".into(),
            quellen: vec![Quelle { pfad: q.display().to_string(), name: "Daten".into() }],
            ziel: ziel.display().to_string(),
            ausschluesse: vec!["*.tmp".into(), "node_modules".into()],
            art,
            zeitplan: Zeitplan::Keiner,
            ..Default::default()
        }
    }

    #[test]
    fn spiegeln_ganz() {
        let d = tmp("spiegel");
        let q = d.join("quelle");
        let z = d.join("ziel");
        fs::create_dir_all(&z).unwrap();
        schreib(&q.join("a.txt"), "eins", 1_600_000_000);
        schreib(&q.join("Unter/b.txt"), "zwei", 1_600_000_000);
        schreib(&q.join("weg.tmp"), "x", 1_600_000_000);
        schreib(&q.join("node_modules/x.js"), "x", 1_600_000_000);
        fs::create_dir_all(q.join("Leer")).unwrap();
        let a = auftrag(&q, &z, Art::Spiegeln);
        let vorher = abbild(&q);

        let v = vorschau(&a, &umg("2026-01-01_10-00-00"), &Fortschritt::default());
        assert_eq!(v.neu_anzahl, 2);
        assert_eq!(v.entfernen_anzahl, 0);
        assert!(!z.join("Test").exists(), "Vorschau schreibt nichts");

        let b = sichern(&a, &umg("2026-01-01_10-00-00"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Ok, "{:?}", b.fehler);
        assert_eq!(b.neu, 2);
        let s = z.join("Test/Daten");
        assert_eq!(lies(&s.join("a.txt")), "eins");
        assert_eq!(lies(&s.join("Unter/b.txt")), "zwei");
        assert!(s.join("Leer").is_dir());
        assert!(!s.join("weg.tmp").exists());
        assert!(!s.join("node_modules").exists());
        let m = fs::metadata(s.join("a.txt")).unwrap();
        assert_eq!(meta_zeit(&m), 1_600_000_000);

        // Zweiter Lauf ohne Änderung: nichts zu tun.
        let b = sichern(&a, &umg("2026-01-01_11-00-00"), &Fortschritt::default());
        assert_eq!((b.neu, b.geaendert, b.entfernt, b.unveraendert), (0, 0, 0, 2));

        // Ändern, löschen, neu.
        schreib(&q.join("a.txt"), "eins neu", 1_600_000_100);
        fs::remove_file(q.join("Unter/b.txt")).unwrap();
        fs::remove_dir(q.join("Unter")).unwrap();
        schreib(&q.join("c.txt"), "drei", 1_600_000_000);
        let vorher2 = abbild(&q);
        let v = vorschau(&a, &umg("2026-01-02_10-00-00"), &Fortschritt::default());
        assert_eq!((v.neu_anzahl, v.ersetzen_anzahl, v.entfernen_anzahl), (1, 1, 1));
        let b = sichern(&a, &umg("2026-01-02_10-00-00"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Ok, "{:?}", b.fehler);
        assert_eq!((b.neu, b.geaendert, b.entfernt), (1, 1, 1));
        assert_eq!(lies(&s.join("a.txt")), "eins neu");
        assert!(!s.join("Unter").exists());
        assert_eq!(lies(&z.join("Test/_entfernt/2026-01-02_10-00-00/Daten/Unter/b.txt")), "zwei");
        // Quelle unverändert.
        assert_eq!(abbild(&q), vorher2);
        let _ = vorher;

        // Stände und Wiederherstellen aus _entfernt.
        let st = staende(&a).unwrap();
        assert_eq!(st, vec![Stand::Spiegel, Stand::Entfernt("2026-01-02_10-00-00".into())]);
        let inh = stand_inhalt(&a, &Stand::Spiegel, &Fortschritt::default()).unwrap();
        assert!(inh.iter().any(|e| e.rel == "Daten/a.txt"));
        assert!(!inh.iter().any(|e| e.rel.starts_with("_entfernt")));
        let wh = d.join("wieder");
        fs::create_dir_all(&wh).unwrap();
        let b = wiederherstellen(&a, &st[1], &["Daten/Unter".into()], &wh, &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Ok, "{:?}", b.fehler);
        assert_eq!(lies(&wh.join("Unter/b.txt")), "zwei");
        // Nochmals: nichts überschreiben, sondern «Unter (2)».
        let b = wiederherstellen(&a, &st[1], &["Daten/Unter/b.txt".into(), "Daten/Unter".into()], &wh, &Fortschritt::default());
        assert_eq!(b.neu, 1);
        assert_eq!(lies(&wh.join("Unter (2)/b.txt")), "zwei");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn schutz_leere_quelle_und_fehlende_quelle() {
        let d = tmp("schutz");
        let q = d.join("quelle");
        let z = d.join("ziel");
        fs::create_dir_all(&z).unwrap();
        schreib(&q.join("a.txt"), "eins", 1_600_000_000);
        let mut a = auftrag(&q, &z, Art::Spiegeln);
        a.entfernte = Entfernte::Loeschen;
        assert_eq!(sichern(&a, &umg("2026-01-01_10-00-00"), &Fortschritt::default()).ergebnis(), Ergebnis::Ok);
        // Quelle leer (z. B. Netzlaufwerk nicht eingebunden): nichts löschen.
        fs::remove_file(q.join("a.txt")).unwrap();
        let b = sichern(&a, &umg("2026-01-01_11-00-00"), &Fortschritt::default());
        assert_eq!(b.entfernt, 0);
        assert!(z.join("Test/Daten/a.txt").exists());
        // Quelle fehlt ganz.
        fs::remove_dir(&q).unwrap();
        let b = sichern(&a, &umg("2026-01-01_12-00-00"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::MitFehlern);
        assert!(z.join("Test/Daten/a.txt").exists());
        // Ziel fehlt.
        let mut a2 = a.clone();
        a2.ziel = d.join("gibtsnicht").display().to_string();
        let b = sichern(&a2, &umg("2026-01-01_12-00-00"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Fehlgeschlagen);
        assert!(!d.join("gibtsnicht").exists());
        // Ziel in der Quelle: abgelehnt.
        fs::create_dir_all(&q).unwrap();
        let mut a3 = auftrag(&q, &q, Art::Spiegeln);
        a3.name = "Innen".into();
        let b = sichern(&a3, &umg("2026-01-01_12-00-00"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Fehlgeschlagen);
        assert!(!q.join("Innen").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn versionen_ordner() {
        let d = tmp("vers");
        let q = d.join("quelle");
        let z = d.join("ziel");
        fs::create_dir_all(&z).unwrap();
        schreib(&q.join("a.txt"), "eins", 1_600_000_000);
        schreib(&q.join("x/b.txt"), "zwei", 1_600_000_000);
        let mut a = auftrag(&q, &z, Art::Versionen);
        a.behalten = 2;
        let vorher = abbild(&q);
        for (i, n) in ["2026-01-01_10-00-00", "2026-01-02_10-00-00", "2026-01-03_10-00-00"].iter().enumerate() {
            if i == 2 {
                schreib(&q.join("a.txt"), "eins geändert", 1_600_000_500);
            }
            let b = sichern(&a, &umg(n), &Fortschritt::default());
            assert_eq!(b.ergebnis(), Ergebnis::Ok, "{:?}", b.fehler);
            assert_eq!(b.version, *n);
            if i == 1 {
                assert_eq!(b.verknuepft, 2);
                assert_eq!(b.neu, 0);
            }
            if i == 2 {
                assert_eq!(b.versionen_geloescht, 1);
                assert_eq!((b.neu, b.verknuepft), (1, 1));
            }
        }
        let ord = z.join("Test");
        assert!(!ord.join("2026-01-01_10-00-00").exists());
        assert_eq!(lies(&ord.join("2026-01-02_10-00-00/Daten/a.txt")), "eins");
        assert_eq!(lies(&ord.join("2026-01-03_10-00-00/Daten/a.txt")), "eins geändert");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(ord.join("2026-01-03_10-00-00/Daten/x/b.txt")).unwrap().nlink(), 2);
        }
        let _ = vorher;
        // Vorschau
        let v = vorschau(&a, &umg("2026-01-04_10-00-00"), &Fortschritt::default());
        assert_eq!((v.dateien, v.verknuepfen, v.neu_anzahl), (2, 2, 0));
        assert_eq!(v.versionen_weg, vec!["2026-01-02_10-00-00".to_string()]);
        // Abbruch: keine halbe Version bleibt liegen.
        let f = Fortschritt::default();
        f.abbrechen();
        let b = sichern(&a, &umg("2026-01-05_10-00-00"), &f);
        assert_eq!(b.ergebnis(), Ergebnis::Abgebrochen);
        assert_eq!(crate::plan::versionen(&eintraege(&ord)).len(), 2);
        assert!(!eintraege(&ord).iter().any(|n| n.contains("unvollstaendig")));
        // Prüfen: alles gut, dann eine Sicherungsdatei verfälschen (gleiche Grösse und Zeit).
        a.stichprobe = 50;
        let b = pruefen(&a, &umg("x"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Ok, "{:?}", b.fehler);
        assert_eq!(b.geprueft, 2);
        assert_eq!(b.unveraendert, 2);
        let p = ord.join("2026-01-03_10-00-00/Daten/a.txt");
        let zeit = meta_zeit(&fs::metadata(&p).unwrap());
        schreib(&p, "EINS GEÄNDERT", zeit);
        let b = pruefen(&a, &umg("x"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::MitFehlern);
        assert!(b.fehler.iter().any(|f| f.art == FehlerArt::Abweichung));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn versionen_zip() {
        let d = tmp("zip");
        let q = d.join("quelle");
        let z = d.join("ziel");
        fs::create_dir_all(&z).unwrap();
        schreib(&q.join("a.txt"), "eins", 1_600_000_000);
        schreib(&q.join("Fotos/Ferien/b.jpg"), "bild", 1_600_000_000);
        fs::create_dir_all(q.join("Leer")).unwrap();
        let mut a = auftrag(&q, &z, Art::Versionen);
        a.form = VersionForm::Zip;
        a.behalten = 1;
        let b = sichern(&a, &umg("2026-01-01_10-00-00"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Ok, "{:?}", b.fehler);
        assert_eq!(b.version, "2026-01-01_10-00-00.zip");
        let b = sichern(&a, &umg("2026-01-02_10-00-00"), &Fortschritt::default());
        assert_eq!(b.versionen_geloescht, 1);
        let st = staende(&a).unwrap();
        assert_eq!(st, vec![Stand::Zip("2026-01-02_10-00-00.zip".into())]);
        let inh = stand_inhalt(&a, &st[0], &Fortschritt::default()).unwrap();
        assert!(inh.iter().any(|e| e.rel == "Daten/Leer" && e.ordner));
        assert!(inh.iter().any(|e| e.rel == "Daten/Fotos/Ferien/b.jpg" && e.groesse == 4 && e.zeit == 1_600_000_000));
        let wh = d.join("wieder");
        fs::create_dir_all(&wh).unwrap();
        let b = wiederherstellen(&a, &st[0], &["Daten/Fotos".into(), "Daten/a.txt".into()], &wh, &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Ok, "{:?}", b.fehler);
        assert_eq!(b.neu, 2);
        assert_eq!(lies(&wh.join("Fotos/Ferien/b.jpg")), "bild");
        assert_eq!(meta_zeit(&fs::metadata(wh.join("a.txt")).unwrap()), 1_600_000_000);
        a.stichprobe = 10;
        let b = pruefen(&a, &umg("x"), &Fortschritt::default());
        assert_eq!(b.ergebnis(), Ergebnis::Ok, "{:?}", b.fehler);
        assert_eq!(b.geprueft, 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn auswahl() {
        let v = auswahl_bereinigen(&["a/b".into(), "a".into(), "c/d".into(), "c/de".into(), "a/".into()]);
        assert_eq!(v, vec!["a".to_string(), "c/d".to_string(), "c/de".to_string()]);
        assert!(unter("a/b", "a"));
        assert!(!unter("ab", "a"));
    }
}
