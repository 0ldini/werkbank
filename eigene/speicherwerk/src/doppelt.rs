//! Kern für «Doppelte Dateien»: Dateien sammeln, nach Inhalt gruppieren, je Gruppe eine Datei
//! nach einer Regel behalten und vor dem Löschen alles noch einmal prüfen.
//!
//! Doppelt heisst: gleicher Inhalt, egal wie die Dateien heissen. Der Weg dahin ist dreistufig,
//! damit nicht jede Datei ganz gelesen werden muss:
//! 1. gleiche Grösse (aus dem Verzeichnis, ohne zu lesen),
//! 2. gleicher Anfang und gleiches Ende (je 16 KB, SHA-256),
//! 3. gleicher SHA-256 über den ganzen Inhalt.
//!
//! Harte Links (derselbe Dateieintrag unter mehreren Namen, gleiche Datei-ID) sind keine
//! Doppel: sie belegen den Platz nur einmal. Sie werden zu einem Eintrag zusammengefasst.
//! Symbolischen Verknüpfungen wird nicht gefolgt. Nur Standardbibliothek (unter Windows für
//! die Datei-ID zusätzlich `windows-sys`), darum mit `rustc --test` prüfbar:
//! `rustc --edition 2021 --test src/doppelt.rs -o /tmp/doppelt && /tmp/doppelt`

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::time::SystemTime;

/// So tief wird höchstens in Unterordner hinabgestiegen.
pub const MAX_TIEFE: usize = 64;
/// Höchstens so viele Dateien werden gesammelt (Speicher bleibt begrenzt).
pub const MAX_DATEIEN: usize = 3_000_000;
/// Grösse von Anfang und Ende für den schnellen Vorvergleich.
const TEIL: u64 = 16 * 1024;
/// Lesepuffer für den ganzen Inhalt.
const PUFFER: usize = 256 * 1024;

// ------------------------------------------------------------------------------- Fortschritt

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Sammeln,
    Vorpruefen,
    Vergleichen,
    Loeschen,
    Fertig,
}

impl Phase {
    fn zahl(self) -> u8 {
        match self {
            Phase::Sammeln => 0,
            Phase::Vorpruefen => 1,
            Phase::Vergleichen => 2,
            Phase::Loeschen => 3,
            Phase::Fertig => 4,
        }
    }
    fn aus(z: u8) -> Phase {
        match z {
            0 => Phase::Sammeln,
            1 => Phase::Vorpruefen,
            2 => Phase::Vergleichen,
            3 => Phase::Loeschen,
            _ => Phase::Fertig,
        }
    }
}

/// Zähler für die Anzeige; der Hintergrund-Thread schreibt, die Oberfläche liest.
#[derive(Default)]
pub struct Fortschritt {
    phase: AtomicU8,
    /// gefundene Dateien / durchsuchte Ordner (Sammeln)
    pub dateien: AtomicU64,
    pub ordner: AtomicU64,
    /// erledigte und gesamte Schritte der laufenden Phase
    pub erledigt: AtomicU64,
    pub gesamt: AtomicU64,
    /// gelesene und zu lesende Bytes der laufenden Phase
    pub bytes: AtomicU64,
    pub bytes_gesamt: AtomicU64,
    pub abbrechen: AtomicBool,
}

impl Fortschritt {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn phase(&self) -> Phase {
        Phase::aus(self.phase.load(Ordering::Relaxed))
    }
    fn setze_phase(&self, p: Phase, gesamt: u64, bytes_gesamt: u64) {
        self.erledigt.store(0, Ordering::Relaxed);
        self.bytes.store(0, Ordering::Relaxed);
        self.gesamt.store(gesamt, Ordering::Relaxed);
        self.bytes_gesamt.store(bytes_gesamt, Ordering::Relaxed);
        self.phase.store(p.zahl(), Ordering::Relaxed);
    }
    pub fn abgebrochen(&self) -> bool {
        self.abbrechen.load(Ordering::Relaxed)
    }
    fn schritt(&self) {
        self.erledigt.fetch_add(1, Ordering::Relaxed);
    }
}

// ------------------------------------------------------------------------------- Sammeln

#[derive(Clone, Debug)]
pub struct Optionen {
    pub unterordner: bool,
    /// Dateien unter dieser Grösse werden übergangen; leere Dateien immer.
    pub min_groesse: u64,
    pub max_tiefe: usize,
    pub max_dateien: usize,
}

impl Default for Optionen {
    fn default() -> Self {
        Optionen { unterordner: true, min_groesse: 1, max_tiefe: MAX_TIEFE, max_dateien: MAX_DATEIEN }
    }
}

#[derive(Clone, Debug)]
pub struct Datei {
    pub pfad: PathBuf,
    pub groesse: u64,
    pub geaendert: Option<SystemTime>,
}

/// Zählungen aus dem Durchsuchen (für die Fussleiste).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Statistik {
    /// alle gesehenen Dateien (auch zu kleine)
    pub dateien: u64,
    pub ordner: u64,
    /// Ordner, die sich nicht öffnen liessen (übersprungen)
    pub unlesbar: u64,
    /// Dateien, deren Angaben oder Inhalt sich nicht lesen liessen
    pub fehler: u64,
    /// übergangene symbolische Verknüpfungen
    pub verknuepfungen: u64,
    /// Ordner jenseits der Höchsttiefe
    pub zu_tief: u64,
    /// Dateien unter der Mindestgrösse (inkl. leere)
    pub zu_klein: u64,
    pub limit_erreicht: bool,
}

/// Geht `wurzel` durch (ohne Rekursion, mit Tiefengrenze) und liefert alle regulären Dateien ab
/// der Mindestgrösse. None = abgebrochen.
pub fn sammeln(wurzel: &Path, opt: &Optionen, fp: &Fortschritt) -> Option<(Vec<Datei>, Statistik)> {
    let mut st = Statistik::default();
    let mut dateien = Vec::new();
    let mut stapel: Vec<(PathBuf, usize)> = vec![(wurzel.to_path_buf(), 0)];
    let min = opt.min_groesse.max(1);
    'ordner: while let Some((ordner, tiefe)) = stapel.pop() {
        if fp.abgebrochen() {
            return None;
        }
        let rd = match fs::read_dir(&ordner) {
            Ok(rd) => rd,
            Err(_) => {
                st.unlesbar = st.unlesbar.saturating_add(1);
                continue;
            }
        };
        st.ordner = st.ordner.saturating_add(1);
        fp.ordner.store(st.ordner, Ordering::Relaxed);
        for e in rd {
            if fp.abgebrochen() {
                return None;
            }
            let Ok(e) = e else {
                st.fehler = st.fehler.saturating_add(1);
                continue;
            };
            // file_type() folgt Verknüpfungen nicht.
            let Ok(ft) = e.file_type() else {
                st.fehler = st.fehler.saturating_add(1);
                continue;
            };
            if ft.is_symlink() {
                st.verknuepfungen = st.verknuepfungen.saturating_add(1);
                continue;
            }
            if ft.is_dir() {
                if opt.unterordner {
                    if tiefe < opt.max_tiefe {
                        stapel.push((e.path(), tiefe.saturating_add(1)));
                    } else {
                        st.zu_tief = st.zu_tief.saturating_add(1);
                    }
                }
                continue;
            }
            if !ft.is_file() {
                continue; // Geräte, Pipes, Sockets: nie lesen
            }
            st.dateien = st.dateien.saturating_add(1);
            fp.dateien.store(st.dateien, Ordering::Relaxed);
            let Ok(m) = e.metadata() else {
                st.fehler = st.fehler.saturating_add(1);
                continue;
            };
            let groesse = m.len();
            if groesse < min {
                st.zu_klein = st.zu_klein.saturating_add(1);
                continue;
            }
            if dateien.len() >= opt.max_dateien {
                st.limit_erreicht = true;
                break 'ordner;
            }
            dateien.push(Datei { pfad: e.path(), groesse, geaendert: m.modified().ok() });
        }
    }
    Some((dateien, st))
}

// ------------------------------------------------------------------------------- Datei-ID

/// Kennung des Dateieintrags (Laufwerk, Dateinummer): gleich bei harten Links.
#[cfg(unix)]
pub fn datei_id(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = fs::symlink_metadata(p).ok()?;
    Some((m.dev(), m.ino()))
}

/// Kennung des Dateieintrags (Laufwerksnummer, Dateinummer): gleich bei harten Links.
#[cfg(windows)]
pub fn datei_id(p: &Path) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let f = File::open(p).ok()?;
    // SAFETY: Die Struktur besteht nur aus Zahlen; lauter Nullen ist ein gültiger Wert.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: Der Griff gehört zu `f` und bleibt bis nach dem Aufruf offen; `info` ist gross genug.
    let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) };
    if ok == 0 {
        return None;
    }
    let nr = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some((u64::from(info.dwVolumeSerialNumber), nr))
}

#[cfg(not(any(unix, windows)))]
pub fn datei_id(_p: &Path) -> Option<(u64, u64)> {
    None
}

// ------------------------------------------------------------------------------- Gruppen

/// Ein Dateieintrag; `links` sind weitere Namen desselben Eintrags (harte Links).
#[derive(Clone, Debug, PartialEq)]
pub struct Eintrag {
    pub pfad: PathBuf,
    pub links: Vec<PathBuf>,
    pub geaendert: Option<SystemTime>,
}

impl Eintrag {
    pub fn pfade(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.pfad).chain(self.links.iter())
    }
}

/// Dateien mit gleichem Inhalt (mindestens zwei verschiedene Dateieinträge).
#[derive(Clone, Debug, PartialEq)]
pub struct Gruppe {
    pub groesse: u64,
    /// SHA-256 des ganzen Inhalts
    pub hash: [u8; 32],
    pub eintraege: Vec<Eintrag>,
}

impl Gruppe {
    /// Platz, der frei würde, wenn nur eine Datei bliebe.
    pub fn freigebbar(&self) -> u64 {
        let n = self.eintraege.len().saturating_sub(1) as u64;
        self.groesse.saturating_mul(n)
    }
}

/// Fasst harte Links zusammen, vergleicht Anfang/Ende und dann den ganzen Inhalt.
/// Rückgabe: Gruppen (nach freigebbarem Platz absteigend) und Zahl der unlesbaren Dateien;
/// None = abgebrochen.
pub fn gruppieren(dateien: Vec<Datei>, fp: &Fortschritt) -> Option<(Vec<Gruppe>, u64)> {
    let mut fehler = 0u64;
    // 1. gleiche Grösse
    let mut nach_groesse: HashMap<u64, Vec<Datei>> = HashMap::new();
    for d in dateien {
        nach_groesse.entry(d.groesse).or_default().push(d);
    }
    let kandidaten: Vec<(u64, Vec<Datei>)> = nach_groesse.into_iter().filter(|(_, v)| v.len() >= 2).collect();
    let n: u64 = kandidaten.iter().map(|(_, v)| v.len() as u64).sum();
    let b: u64 = kandidaten.iter().fold(0u64, |a, (g, v)| a.saturating_add(g.min(&(2 * TEIL)).saturating_mul(v.len() as u64)));
    fp.setze_phase(Phase::Vorpruefen, n, b);

    // 2. harte Links zusammenfassen, dann Anfang + Ende
    let mut stufe3: Vec<(u64, Vec<Eintrag>)> = Vec::new();
    let mut fertig: Vec<Gruppe> = Vec::new();
    for (groesse, mut v) in kandidaten {
        v.sort_by(|a, c| a.pfad.cmp(&c.pfad));
        let eintraege = zusammenfassen(v);
        if eintraege.len() < 2 {
            fp.erledigt.fetch_add(eintraege.iter().map(|e| 1 + e.links.len() as u64).sum(), Ordering::Relaxed);
            continue;
        }
        let mut nach_teil: HashMap<[u8; 32], Vec<Eintrag>> = HashMap::new();
        let mut voll = false;
        for e in eintraege {
            if fp.abgebrochen() {
                return None;
            }
            fp.erledigt.fetch_add(1 + e.links.len() as u64, Ordering::Relaxed);
            match teil_hash(&e.pfad, groesse) {
                Ok((h, ganz)) => {
                    voll = ganz;
                    fp.bytes.fetch_add(groesse.min(2 * TEIL), Ordering::Relaxed);
                    nach_teil.entry(h).or_default().push(e);
                }
                Err(_) => fehler = fehler.saturating_add(1),
            }
        }
        for (h, v) in nach_teil {
            if v.len() < 2 {
                continue;
            }
            if voll {
                // Kleine Datei: Anfang + Ende war schon der ganze Inhalt.
                fertig.push(Gruppe { groesse, hash: h, eintraege: v });
            } else {
                stufe3.push((groesse, v));
            }
        }
    }

    // 3. ganzer Inhalt
    let n: u64 = stufe3.iter().map(|(_, v)| v.len() as u64).sum();
    let b: u64 = stufe3.iter().fold(0u64, |a, (g, v)| a.saturating_add(g.saturating_mul(v.len() as u64)));
    fp.setze_phase(Phase::Vergleichen, n, b);
    for (groesse, v) in stufe3 {
        let mut nach_hash: HashMap<[u8; 32], Vec<Eintrag>> = HashMap::new();
        for e in v {
            if fp.abgebrochen() {
                return None;
            }
            match voll_hash(&e.pfad, groesse, Some(fp)) {
                Ok(Some(h)) => nach_hash.entry(h).or_default().push(e),
                Ok(None) => return None,
                Err(_) => fehler = fehler.saturating_add(1),
            }
            fp.schritt();
        }
        for (h, v) in nach_hash {
            if v.len() >= 2 {
                fertig.push(Gruppe { groesse, hash: h, eintraege: v });
            }
        }
    }
    for g in &mut fertig {
        g.eintraege.sort_by(|a, c| a.pfad.cmp(&c.pfad));
    }
    sortieren(&mut fertig);
    fp.setze_phase(Phase::Fertig, 0, 0);
    Some((fertig, fehler))
}

/// Gruppen nach freigebbarem Platz absteigend, dann Grösse, dann Pfad.
pub fn sortieren(g: &mut [Gruppe]) {
    g.sort_by(|a, b| {
        b.freigebbar()
            .cmp(&a.freigebbar())
            .then(b.groesse.cmp(&a.groesse))
            .then_with(|| a.eintraege.first().map(|e| &e.pfad).cmp(&b.eintraege.first().map(|e| &e.pfad)))
    });
}

/// Dateien gleicher Grösse: gleiche Datei-ID (harte Links) wird ein Eintrag.
fn zusammenfassen(v: Vec<Datei>) -> Vec<Eintrag> {
    let mut out: Vec<Eintrag> = Vec::new();
    let mut nach_id: HashMap<(u64, u64), usize> = HashMap::new();
    for d in v {
        let id = datei_id(&d.pfad);
        if let Some(i) = id.and_then(|id| nach_id.get(&id).copied()) {
            if let Some(e) = out.get_mut(i) {
                e.links.push(d.pfad);
                continue;
            }
        }
        if let Some(id) = id {
            nach_id.insert(id, out.len());
        }
        out.push(Eintrag { pfad: d.pfad, links: Vec::new(), geaendert: d.geaendert });
    }
    out
}

/// SHA-256 über Anfang und Ende (je `TEIL`). Ist die Datei höchstens 2 × `TEIL` gross, ist es
/// der Hash des ganzen Inhalts (zweiter Wert true).
fn teil_hash(p: &Path, groesse: u64) -> io::Result<([u8; 32], bool)> {
    if groesse <= 2 * TEIL {
        return match voll_hash(p, groesse, None)? {
            Some(h) => Ok((h, true)),
            None => Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen")),
        };
    }
    let mut f = File::open(p)?;
    let mut buf = vec![0u8; TEIL as usize];
    let mut s = Sha256::new();
    f.read_exact(&mut buf)?;
    s.update(&buf);
    f.seek(SeekFrom::Start(groesse.saturating_sub(TEIL)))?;
    f.read_exact(&mut buf)?;
    s.update(&buf);
    Ok((s.finish(), false))
}

/// SHA-256 des ganzen Inhalts. Fehler, wenn die Datei nicht mehr `groesse` Bytes hat.
/// Ok(None) = abgebrochen.
pub fn voll_hash(p: &Path, groesse: u64, fp: Option<&Fortschritt>) -> io::Result<Option<[u8; 32]>> {
    let mut f = File::open(p)?;
    let mut buf = vec![0u8; PUFFER];
    let mut s = Sha256::new();
    let mut gelesen = 0u64;
    loop {
        if fp.is_some_and(|fp| fp.abgebrochen()) {
            return Ok(None);
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let Some(teil) = buf.get(..n) else { break };
        s.update(teil);
        gelesen = gelesen.saturating_add(n as u64);
        if gelesen > groesse {
            break;
        }
        if let Some(fp) = fp {
            fp.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    if gelesen != groesse {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Grösse hat sich geändert"));
    }
    Ok(Some(s.finish()))
}

/// Ganze Suche: sammeln und gruppieren. None = abgebrochen.
pub fn suchen(wurzel: &Path, opt: &Optionen, fp: &Fortschritt) -> Option<(Vec<Gruppe>, Statistik)> {
    fp.setze_phase(Phase::Sammeln, 0, 0);
    let (dateien, mut st) = sammeln(wurzel, opt, fp)?;
    let (gruppen, fehler) = gruppieren(dateien, fp)?;
    st.fehler = st.fehler.saturating_add(fehler);
    Some((gruppen, st))
}

// ------------------------------------------------------------------------------- Behalten

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Regel {
    Aelteste,
    Neueste,
    KuerzesterPfad,
}

fn pfadlaenge(p: &Path) -> usize {
    p.to_string_lossy().chars().count()
}

/// Welche Datei die Regel behält. Gleichstand: kürzerer Pfad bzw. ältere Datei, dann alphabetisch.
/// Dateien ohne bekanntes Datum kommen bei «älteste»/«neueste» zuletzt.
pub fn behalten_index(g: &Gruppe, r: Regel) -> usize {
    let mut beste = 0usize;
    for (i, e) in g.eintraege.iter().enumerate().skip(1) {
        let Some(b) = g.eintraege.get(beste) else { break };
        if besser(e, b, r) {
            beste = i;
        }
    }
    beste
}

fn besser(a: &Eintrag, b: &Eintrag, r: Regel) -> bool {
    use std::cmp::Ordering as O;
    // Datum: bekannt vor unbekannt; dann je nach Richtung.
    let datum = |alt_zuerst: bool| match (a.geaendert, b.geaendert) {
        (Some(x), Some(y)) => {
            if alt_zuerst {
                x.cmp(&y)
            } else {
                y.cmp(&x)
            }
        }
        (Some(_), None) => O::Less,
        (None, Some(_)) => O::Greater,
        (None, None) => O::Equal,
    };
    let laenge = pfadlaenge(&a.pfad).cmp(&pfadlaenge(&b.pfad));
    let ord = match r {
        Regel::Aelteste => datum(true).then(laenge),
        Regel::Neueste => datum(false).then(laenge),
        Regel::KuerzesterPfad => laenge.then(datum(true)),
    };
    ord.then_with(|| a.pfad.cmp(&b.pfad)) == O::Less
}

/// Auswahl nach Regel: true = in den Papierkorb, genau eine Datei bleibt.
pub fn auswahl_nach_regel(g: &Gruppe, r: Regel) -> Vec<bool> {
    let k = behalten_index(g, r);
    (0..g.eintraege.len()).map(|i| i != k).collect()
}

/// Darf Datei `i` zum Löschen markiert werden? Nur, wenn danach noch eine bleibt.
pub fn darf_loeschen(loeschen: &[bool], i: usize) -> bool {
    loeschen.iter().enumerate().any(|(j, &l)| j != i && !l)
}

/// Schaltet die Markierung von Datei `i` um; verweigert, wenn sonst keine Datei bliebe.
pub fn umschalten(loeschen: &mut [bool], i: usize) -> bool {
    let erlaubt = darf_loeschen(loeschen, i);
    match loeschen.get_mut(i) {
        Some(l) if *l => {
            *l = false;
            true
        }
        Some(l) if erlaubt => {
            *l = true;
            true
        }
        _ => false,
    }
}

// ------------------------------------------------------------------------------- Löschen

/// Warum eine Datei nicht in den Papierkorb kam.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grund {
    /// Datei gibt es nicht mehr.
    Fehlt,
    /// Grösse oder Inhalt anders als beim Suchen.
    Geaendert,
    /// Keine behaltene Datei mit gleichem Inhalt mehr da.
    KeinPartner,
    /// Ist inzwischen derselbe Dateieintrag wie die behaltene Datei.
    GleicheDatei,
    Lesefehler(String),
    Papierkorb(String),
    Abgebrochen,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Loeschergebnis {
    pub pfad: PathBuf,
    /// frei gewordener Platz (bei harten Links nur beim ersten Namen gezählt)
    pub bytes: u64,
    pub erfolg: Result<(), Grund>,
}

/// Eine Gruppe mit der Auswahl (true = in den Papierkorb).
#[derive(Clone, Debug)]
pub struct Auftrag {
    pub gruppe: Gruppe,
    pub loeschen: Vec<bool>,
}

/// Prüft, ob `p` noch eine normale Datei mit `groesse` Bytes ist; liefert ihre ID.
fn noch_da(p: &Path, groesse: u64) -> Result<Option<(u64, u64)>, Grund> {
    let m = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Grund::Fehlt),
        Err(e) => return Err(Grund::Lesefehler(e.to_string())),
    };
    if !m.file_type().is_file() || m.len() != groesse {
        return Err(Grund::Geaendert);
    }
    Ok(datei_id(p))
}

/// Prüft `p` ganz: noch da, gleiche Grösse, gleicher SHA-256.
fn gleich(p: &Path, g: &Gruppe, fp: &Fortschritt) -> Result<Option<(u64, u64)>, Grund> {
    let id = noch_da(p, g.groesse)?;
    match voll_hash(p, g.groesse, Some(fp)) {
        Ok(Some(h)) if h == g.hash => Ok(id),
        Ok(Some(_)) => Err(Grund::Geaendert),
        Ok(None) => Err(Grund::Abgebrochen),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(Grund::Geaendert),
        Err(e) => Err(Grund::Lesefehler(e.to_string())),
    }
}

/// Anzahl Pfade und Bytes, die `auftraege` in den Papierkorb verschieben würden.
pub fn umfang(auftraege: &[Auftrag]) -> (u64, u64) {
    let mut n = 0u64;
    let mut b = 0u64;
    for a in auftraege {
        for (e, l) in a.gruppe.eintraege.iter().zip(&a.loeschen) {
            if *l {
                n = n.saturating_add(1 + e.links.len() as u64);
                b = b.saturating_add(a.gruppe.groesse);
            }
        }
    }
    (n, b)
}

/// Verschiebt die markierten Dateien mit `papierkorb` – nie endgültig löschen. Vorher wird je
/// Gruppe eine behaltene Datei mit gleichem Inhalt gesucht und jede zu löschende Datei noch
/// einmal ganz geprüft (gleiche Grösse, gleicher SHA-256, nicht derselbe Dateieintrag).
pub fn loeschen(auftraege: &[Auftrag], fp: &Fortschritt, papierkorb: &mut dyn FnMut(&Path) -> Result<(), String>) -> Vec<Loeschergebnis> {
    let (n, _) = umfang(auftraege);
    let b = auftraege.iter().fold(0u64, |a, x| a.saturating_add(x.gruppe.groesse.saturating_mul(x.gruppe.eintraege.len() as u64)));
    fp.setze_phase(Phase::Loeschen, n, b);
    let mut out = Vec::new();
    for a in auftraege {
        let g = &a.gruppe;
        let markiert: Vec<&Eintrag> = g.eintraege.iter().zip(&a.loeschen).filter(|(_, l)| **l).map(|(e, _)| e).collect();
        if markiert.is_empty() {
            continue;
        }
        let alle = |grund: Grund, out: &mut Vec<Loeschergebnis>| {
            for e in &markiert {
                for p in e.pfade() {
                    out.push(Loeschergebnis { pfad: p.clone(), bytes: 0, erfolg: Err(grund.clone()) });
                    fp.schritt();
                }
            }
        };
        if fp.abgebrochen() {
            alle(Grund::Abgebrochen, &mut out);
            continue;
        }
        // Eine behaltene Datei muss noch da und gleich sein (sonst ginge der Inhalt verloren).
        let behalten: Vec<&Eintrag> = g.eintraege.iter().zip(a.loeschen.iter().chain(std::iter::repeat(&false))).filter(|(_, l)| !**l).map(|(e, _)| e).collect();
        let mut partner: Option<Option<(u64, u64)>> = None;
        'suche: for e in &behalten {
            for p in e.pfade() {
                if let Ok(id) = gleich(p, g, fp) {
                    partner = Some(id);
                    break 'suche;
                }
            }
        }
        let Some(partner_id) = partner else {
            let grund = if fp.abgebrochen() { Grund::Abgebrochen } else { Grund::KeinPartner };
            alle(grund, &mut out);
            continue;
        };
        for e in &markiert {
            // geprüfte ID dieses Eintrags: weitere harte Links mit derselben ID brauchen keinen Hash
            let mut geprueft: Option<(u64, u64)> = None;
            let mut erster = true;
            for p in e.pfade() {
                fp.schritt();
                let pruefung = if fp.abgebrochen() {
                    Err(Grund::Abgebrochen)
                } else {
                    match noch_da(p, g.groesse) {
                        Err(gr) => Err(gr),
                        Ok(id) if id.is_some() && id == partner_id => Err(Grund::GleicheDatei),
                        Ok(id) if id.is_some() && id == geprueft => Ok(()),
                        Ok(_) => match gleich(p, g, fp) {
                            Ok(id) if id.is_some() && id == partner_id => Err(Grund::GleicheDatei),
                            Ok(id) => {
                                geprueft = id;
                                Ok(())
                            }
                            Err(gr) => Err(gr),
                        },
                    }
                };
                let erfolg = pruefung.and_then(|()| papierkorb(p).map_err(Grund::Papierkorb));
                let bytes = if erfolg.is_ok() && erster { g.groesse } else { 0 };
                if erfolg.is_ok() {
                    erster = false;
                }
                out.push(Loeschergebnis { pfad: p.clone(), bytes, erfolg });
            }
        }
    }
    fp.setze_phase(Phase::Fertig, 0, 0);
    out
}

/// Entfernt verschobene Pfade aus den Gruppen; Gruppen mit weniger als zwei Einträgen fallen weg.
pub fn nach_loeschen(gruppen: &mut Vec<Gruppe>, weg: &HashSet<PathBuf>) {
    for g in gruppen.iter_mut() {
        for e in g.eintraege.iter_mut() {
            if weg.contains(&e.pfad) {
                let mut rest = e.links.iter().filter(|p| !weg.contains(*p)).cloned();
                match rest.next() {
                    Some(p) => {
                        e.pfad = p;
                        e.links = rest.collect();
                    }
                    None => e.links.clear(),
                }
            } else {
                e.links.retain(|p| !weg.contains(p));
            }
        }
        g.eintraege.retain(|e| !weg.contains(&e.pfad));
    }
    gruppen.retain(|g| g.eintraege.len() >= 2);
    sortieren(gruppen);
}

// ------------------------------------------------------------------------------- CSV, Anzeige

fn csv_feld(s: &str) -> String {
    if s.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// CSV der Gruppen: `Gruppe;Bytes;SHA-256;Aktion;Pfad`. `woerter` = [Kopfzeile, behalten,
/// Papierkorb, harter Link].
pub fn csv(gruppen: &[Gruppe], auswahl: &[Vec<bool>], woerter: [&str; 4]) -> String {
    let [kopf, behalten, weg, link] = woerter;
    let mut out = format!("{kopf}\n");
    for (i, g) in gruppen.iter().enumerate() {
        let h = hex(&g.hash);
        for (j, e) in g.eintraege.iter().enumerate() {
            let l = auswahl.get(i).and_then(|a| a.get(j)).copied().unwrap_or(false);
            let aktion = if l { weg } else { behalten };
            out.push_str(&format!("{};{};{};{};{}\n", i + 1, g.groesse, h, csv_feld(aktion), csv_feld(&e.pfad.to_string_lossy())));
            for p in &e.links {
                out.push_str(&format!("{};{};{};{};{}\n", i + 1, g.groesse, h, csv_feld(link), csv_feld(&p.to_string_lossy())));
            }
        }
    }
    out
}

/// Datum als TT.MM.JJJJ (UTC).
pub fn datum(t: Option<SystemTime>) -> String {
    let Some(s) = t.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()) else {
        return String::from("–");
    };
    let tage = (s / 86_400) as i64;
    // Tage seit 1970 → Kalenderdatum (Howard Hinnant, civil_from_days)
    let z = tage + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d:02}.{m:02}.{y}")
}

// ------------------------------------------------------------------------------- SHA-256

/// SHA-256 (FIPS 180-4), nur Standardbibliothek.
pub struct Sha256 {
    h: [u32; 8],
    block: [u8; 64],
    fuell: usize,
    laenge: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
    0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            block: [0; 64],
            fuell: 0,
            laenge: 0,
        }
    }

    fn runde(h: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            if let (Some(wi), [a, b, cc, d]) = (w.get_mut(i), c) {
                *wi = u32::from_be_bytes([*a, *b, *cc, *d]);
            }
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }

    pub fn update(&mut self, mut daten: &[u8]) {
        self.laenge = self.laenge.wrapping_add(daten.len() as u64);
        while !daten.is_empty() {
            let frei = 64 - self.fuell;
            let n = frei.min(daten.len());
            let (vorne, rest) = daten.split_at(n);
            if let Some(ziel) = self.block.get_mut(self.fuell..self.fuell + n) {
                ziel.copy_from_slice(vorne);
            }
            self.fuell += n;
            daten = rest;
            if self.fuell == 64 {
                Self::runde(&mut self.h, &self.block);
                self.fuell = 0;
            }
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.laenge.wrapping_mul(8);
        self.update(&[0x80]);
        while self.fuell != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 32];
        for (c, v) in out.chunks_exact_mut(4).zip(self.h) {
            c.copy_from_slice(&v.to_be_bytes());
        }
        out
    }
}

// ------------------------------------------------------------------------------- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sha(d: &[u8]) -> String {
        let mut s = Sha256::new();
        s.update(d);
        hex(&s.finish())
    }

    #[test]
    fn sha256_pruefwerte() {
        assert_eq!(sha(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // eine Million «a», in ungeraden Stücken
        let mut s = Sha256::new();
        let a = vec![b'a'; 1000];
        for i in 0..1000 {
            let (x, y) = a.split_at(i % 97);
            s.update(x);
            s.update(y);
        }
        assert_eq!(hex(&s.finish()), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(name: &str) -> Tmp {
            let p = std::env::temp_dir().join(format!("speicherwerk-doppelt-{}-{}", name, std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).expect("Testordner");
            Tmp(p)
        }
        fn datei(&self, rel: &str, inhalt: &[u8]) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d).expect("Ordner");
            }
            fs::write(&p, inhalt).expect("schreiben");
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn gross(n: usize, x: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(31) ^ x).collect()
    }

    fn namen(g: &Gruppe, wurzel: &Path) -> Vec<String> {
        g.eintraege.iter().map(|e| e.pfad.strip_prefix(wurzel).unwrap_or(&e.pfad).to_string_lossy().replace('\\', "/")).collect()
    }

    #[test]
    fn findet_doppelte_nach_inhalt() {
        let t = Tmp::new("inhalt");
        // gleicher Name, anderer Inhalt
        t.datei("a/foto.jpg", b"eins");
        t.datei("b/foto.jpg", b"zwei");
        // andere Namen, gleicher Inhalt, verschachtelt
        t.datei("x.txt", b"hallo welt");
        t.datei("tief/er/noch/y.dat", b"hallo welt");
        t.datei("z.bin", b"hallo welt");
        // leere Dateien sind keine Doppel
        t.datei("leer1", b"");
        t.datei("leer2", b"");
        // grosse Dateien: gleicher Anfang und gleiches Ende, Mitte verschieden
        let g1 = gross(100_000, 7);
        let mut g2 = g1.clone();
        g2[50_000] ^= 0xff;
        t.datei("gross1.bin", &g1);
        t.datei("gross2.bin", &g2);
        t.datei("gross3.bin", &g1);

        let fp = Fortschritt::new();
        let (g, st) = suchen(&t.0, &Optionen::default(), &fp).expect("nicht abgebrochen");
        assert_eq!(st.zu_klein, 2, "leere Dateien");
        assert_eq!(st.dateien, 10);
        assert_eq!(g.len(), 2, "{g:?}");
        // nach freigebbarem Platz sortiert: grosse Gruppe zuerst
        assert_eq!(namen(&g[0], &t.0), vec!["gross1.bin", "gross3.bin"]);
        assert_eq!(g[0].freigebbar(), 100_000);
        assert_eq!(namen(&g[1], &t.0), vec!["tief/er/noch/y.dat", "x.txt", "z.bin"]);
        assert_eq!(g[1].freigebbar(), 20);
        assert_eq!(hex(&g[1].hash), sha(b"hallo welt"));
        assert_eq!(fp.phase(), Phase::Fertig);
    }

    #[test]
    fn ohne_unterordner_und_mindestgroesse() {
        let t = Tmp::new("flach");
        t.datei("a.txt", b"12345");
        t.datei("b.txt", b"12345");
        t.datei("sub/c.txt", b"12345");
        t.datei("k1", b"ab");
        t.datei("k2", b"ab");
        let fp = Fortschritt::new();
        let opt = Optionen { unterordner: false, min_groesse: 3, ..Optionen::default() };
        let (g, st) = suchen(&t.0, &opt, &fp).expect("ok");
        assert_eq!(g.len(), 1);
        assert_eq!(namen(&g[0], &t.0), vec!["a.txt", "b.txt"]);
        assert_eq!(st.zu_klein, 2);
        // Tiefengrenze 0 mit Unterordnern: Unterordner wird gezählt, nicht betreten
        let opt = Optionen { max_tiefe: 0, ..Optionen::default() };
        let (g, st) = suchen(&t.0, &opt, &fp).expect("ok");
        assert_eq!(g[0].eintraege.len(), 2);
        assert_eq!(st.zu_tief, 1);
        // Limit
        let opt = Optionen { max_dateien: 1, ..Optionen::default() };
        let (_, st) = suchen(&t.0, &opt, &fp).expect("ok");
        assert!(st.limit_erreicht);
    }

    #[test]
    fn abbrechen() {
        let t = Tmp::new("abbruch");
        t.datei("a", b"x");
        let fp = Fortschritt::new();
        fp.abbrechen.store(true, Ordering::Relaxed);
        assert!(suchen(&t.0, &Optionen::default(), &fp).is_none());
    }

    #[test]
    fn unlesbarer_ordner_wird_gezaehlt() {
        let t = Tmp::new("unlesbar");
        let fp = Fortschritt::new();
        let (g, st) = suchen(&t.0.join("gibt-es-nicht"), &Optionen::default(), &fp).expect("ok");
        assert!(g.is_empty());
        assert_eq!(st.unlesbar, 1);
    }

    #[cfg(unix)]
    #[test]
    fn harte_links_und_verknuepfungen_sind_keine_doppel() {
        let t = Tmp::new("links");
        let a = t.datei("a.txt", b"gleicher inhalt");
        fs::hard_link(&a, t.0.join("a-link.txt")).expect("harter Link");
        std::os::unix::fs::symlink(&a, t.0.join("a-sym.txt")).expect("symlink");
        fs::create_dir_all(t.0.join("echt")).expect("ordner");
        t.datei("echt/drin.txt", b"anderer inhalt");
        std::os::unix::fs::symlink(t.0.join("echt"), t.0.join("schleife")).expect("ordner-symlink");
        let fp = Fortschritt::new();
        let (g, st) = suchen(&t.0, &Optionen::default(), &fp).expect("ok");
        assert!(g.is_empty(), "nur harte Links: keine Doppel {g:?}");
        assert_eq!(st.verknuepfungen, 2);

        // dazu eine echte Kopie: eine Gruppe mit zwei Einträgen, der harte Link hängt am ersten
        let k = t.datei("kopie.txt", b"gleicher inhalt");
        let (g, _) = suchen(&t.0, &Optionen::default(), &fp).expect("ok");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].eintraege.len(), 2);
        assert_eq!(g[0].freigebbar(), 15);
        let mit_link = g[0].eintraege.iter().find(|e| !e.links.is_empty()).expect("Eintrag mit Link");
        assert_eq!(mit_link.links.len(), 1);

        // Kopie nach dem Suchen durch einen harten Link auf das Original ersetzt: nicht löschen
        let mut auswahl: Vec<bool> = g[0].eintraege.iter().map(|e| e.pfad == k).collect();
        fs::remove_file(&k).expect("weg");
        fs::hard_link(&a, &k).expect("link");
        let mut korb = Vec::new();
        let r = loeschen(&[Auftrag { gruppe: g[0].clone(), loeschen: auswahl.clone() }], &fp, &mut |p| {
            korb.push(p.to_path_buf());
            Ok(())
        });
        assert!(korb.is_empty());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].erfolg, Err(Grund::GleicheDatei));

        // Eintrag mit hartem Link löschen: beide Namen, Platz einmal gezählt
        fs::remove_file(&k).expect("weg");
        fs::write(&k, b"gleicher inhalt").expect("kopie");
        let (g, _) = suchen(&t.0, &Optionen::default(), &fp).expect("ok");
        auswahl = g[0].eintraege.iter().map(|e| !e.links.is_empty()).collect();
        let r = loeschen(&[Auftrag { gruppe: g[0].clone(), loeschen: auswahl }], &fp, &mut |p| {
            korb.push(p.to_path_buf());
            Ok(())
        });
        assert_eq!(korb.len(), 2, "{r:?}");
        assert_eq!(r.iter().map(|x| x.bytes).sum::<u64>(), 15);
    }

    fn eintrag(p: &str, alter: u64) -> Eintrag {
        Eintrag { pfad: PathBuf::from(p), links: vec![], geaendert: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(alter)) }
    }

    #[test]
    fn behalten_regeln() {
        let g = Gruppe {
            groesse: 10,
            hash: [0; 32],
            eintraege: vec![eintrag("/b/lang/kopie.jpg", 100), eintrag("/a/x.jpg", 300), eintrag("/a/neu/sehr/lang.jpg", 900)],
        };
        assert_eq!(behalten_index(&g, Regel::Aelteste), 0);
        assert_eq!(behalten_index(&g, Regel::Neueste), 2);
        assert_eq!(behalten_index(&g, Regel::KuerzesterPfad), 1);
        assert_eq!(auswahl_nach_regel(&g, Regel::KuerzesterPfad), vec![true, false, true]);
        // Gleichstand beim Datum (Kopie mit erhaltenem Datum): kürzerer Pfad gewinnt
        let g = Gruppe { groesse: 1, hash: [0; 32], eintraege: vec![eintrag("/a/foto - Kopie.jpg", 5), eintrag("/a/foto.jpg", 5)] };
        assert_eq!(behalten_index(&g, Regel::Aelteste), 1);
        assert_eq!(behalten_index(&g, Regel::Neueste), 1);
        // unbekanntes Datum zuletzt
        let mut e = eintrag("/z", 0);
        e.geaendert = None;
        let g = Gruppe { groesse: 1, hash: [0; 32], eintraege: vec![e, eintrag("/sehr/lang/aber/datiert", 50)] };
        assert_eq!(behalten_index(&g, Regel::Aelteste), 1);
        assert_eq!(behalten_index(&g, Regel::Neueste), 1);
    }

    #[test]
    fn eine_datei_bleibt_immer() {
        let mut l = vec![true, false, true];
        assert!(!umschalten(&mut l, 1), "letzte behaltene");
        assert_eq!(l, vec![true, false, true]);
        assert!(umschalten(&mut l, 0));
        assert_eq!(l, vec![false, false, true]);
        assert!(umschalten(&mut l, 1));
        assert_eq!(l, vec![false, true, true]);
        assert!(!umschalten(&mut l, 0));
        assert!(!umschalten(&mut l, 9), "ausserhalb");
        assert!(!darf_loeschen(&[false], 0));
    }

    #[test]
    fn loeschen_prueft_vorher() {
        let t = Tmp::new("loeschen");
        let inhalt = gross(40_000, 3);
        t.datei("orig.bin", &inhalt);
        t.datei("kopie1.bin", &inhalt);
        t.datei("kopie2.bin", &inhalt);
        t.datei("kopie3.bin", &inhalt);
        t.datei("weg.bin", &inhalt);
        t.datei("p/a.txt", b"zweite gruppe");
        t.datei("p/b.txt", b"zweite gruppe");
        let fp = Fortschritt::new();
        let (mut g, _) = suchen(&t.0, &Optionen::default(), &fp).expect("ok");
        assert_eq!(g.len(), 2);
        let auswahl: Vec<Vec<bool>> = g.iter().map(|g| auswahl_nach_regel(g, Regel::KuerzesterPfad)).collect();
        // gruppe 0: kopie1 kopie2 kopie3 orig weg  → kürzester: weg.bin (7 Zeichen) vs orig.bin
        assert_eq!(namen(&g[0], &t.0)[behalten_index(&g[0], Regel::KuerzesterPfad)], "weg.bin");
        // nach dem Suchen geändert: kopie2 anderer Inhalt gleicher Grösse, kopie3 kürzer, orig fehlt
        let mut anders = inhalt.clone();
        anders[20_000] ^= 1;
        t.datei("kopie2.bin", &anders);
        t.datei("kopie3.bin", b"kurz");
        fs::remove_file(t.0.join("orig.bin")).expect("weg");
        let auftraege: Vec<Auftrag> = g.iter().cloned().zip(auswahl.clone()).map(|(gruppe, loeschen)| Auftrag { gruppe, loeschen }).collect();
        assert_eq!(umfang(&auftraege), (5, 4 * 40_000 + 13));
        let mut korb = Vec::new();
        let r = loeschen(&auftraege, &fp, &mut |p| {
            korb.push(p.to_path_buf());
            if p.ends_with("b.txt") {
                Err("gesperrt".into())
            } else {
                Ok(())
            }
        });
        let ergebnis = |n: &str| r.iter().find(|x| x.pfad.ends_with(n)).map(|x| x.erfolg.clone());
        assert_eq!(ergebnis("kopie1.bin"), Some(Ok(())));
        assert_eq!(ergebnis("kopie2.bin"), Some(Err(Grund::Geaendert)));
        assert_eq!(ergebnis("kopie3.bin"), Some(Err(Grund::Geaendert)));
        assert_eq!(ergebnis("orig.bin"), Some(Err(Grund::Fehlt)));
        assert_eq!(ergebnis("b.txt"), Some(Err(Grund::Papierkorb("gesperrt".into()))));
        assert!(ergebnis("a.txt").is_none());
        assert!(ergebnis("weg.bin").is_none(), "behaltene Datei nie anfassen");
        assert_eq!(r.iter().map(|x| x.bytes).sum::<u64>(), 40_000);
        // nur kopie1 und b.txt wurden dem Papierkorb übergeben
        assert_eq!(korb.len(), 2);

        // Gruppen nachführen
        let weg: HashSet<PathBuf> = r.iter().filter(|x| x.erfolg.is_ok()).map(|x| x.pfad.clone()).collect();
        nach_loeschen(&mut g, &weg);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].eintraege.len(), 4);
    }

    #[test]
    fn ohne_behaltenen_partner_nichts_loeschen() {
        let t = Tmp::new("partner");
        t.datei("a", b"inhalt!");
        t.datei("b", b"inhalt!");
        t.datei("c", b"inhalt!");
        let fp = Fortschritt::new();
        let (g, _) = suchen(&t.0, &Optionen::default(), &fp).expect("ok");
        let auswahl = auswahl_nach_regel(&g[0], Regel::KuerzesterPfad); // behält «a»
        // behaltene Datei inzwischen geändert
        t.datei("a", b"anders!");
        let mut korb = 0;
        let r = loeschen(&[Auftrag { gruppe: g[0].clone(), loeschen: auswahl }], &fp, &mut |_| {
            korb += 1;
            Ok(())
        });
        assert_eq!(korb, 0);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.erfolg == Err(Grund::KeinPartner)));
        // alles markiert (darf nie vorkommen): ebenfalls nichts löschen
        let r = loeschen(&[Auftrag { gruppe: g[0].clone(), loeschen: vec![true; 3] }], &fp, &mut |_| {
            korb += 1;
            Ok(())
        });
        assert_eq!(korb, 0);
        assert!(r.iter().all(|x| x.erfolg == Err(Grund::KeinPartner)));
    }

    #[test]
    fn nachfuehren_mit_links() {
        let mut e = eintrag("/a", 1);
        e.links = vec![PathBuf::from("/a2"), PathBuf::from("/a3")];
        let mut g = vec![Gruppe { groesse: 5, hash: [0; 32], eintraege: vec![e, eintrag("/b", 1), eintrag("/c", 1)] }];
        let weg: HashSet<PathBuf> = ["/a", "/a3", "/c"].iter().map(PathBuf::from).collect();
        nach_loeschen(&mut g, &weg);
        assert_eq!(g[0].eintraege.len(), 2);
        assert_eq!(g[0].eintraege[0].pfad, PathBuf::from("/a2"));
        assert!(g[0].eintraege[0].links.is_empty());
        let weg: HashSet<PathBuf> = ["/a2"].iter().map(PathBuf::from).collect();
        nach_loeschen(&mut g, &weg);
        assert!(g.is_empty());
    }

    #[test]
    fn csv_und_datum() {
        let mut e = eintrag("/x;y/\"z\".txt", 0);
        e.links = vec![PathBuf::from("/l")];
        let g = vec![Gruppe { groesse: 3, hash: [0xab; 32], eintraege: vec![e, eintrag("/b", 0)] }];
        let c = csv(&g, &[vec![false, true]], ["Gruppe;Bytes;SHA-256;Aktion;Pfad", "behalten", "Papierkorb", "harter Link"]);
        let z: Vec<&str> = c.lines().collect();
        assert_eq!(z.len(), 4);
        assert!(z[1].ends_with(";behalten;\"/x;y/\"\"z\"\".txt\""), "{}", z[1]);
        assert!(z[2].ends_with(";harter Link;/l"));
        assert!(z[3].starts_with("1;3;abab") && z[3].ends_with(";Papierkorb;/b"));
        assert_eq!(datum(Some(SystemTime::UNIX_EPOCH)), "01.01.1970");
        assert_eq!(datum(Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_709_208_000))), "29.02.2024");
        assert_eq!(datum(None), "–");
    }
}
