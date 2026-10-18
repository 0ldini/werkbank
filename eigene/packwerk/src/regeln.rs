//! Reine Regeln ohne Oberfläche und ohne Fremd-Crates: Archivart erkennen, Pfade aus
//! Archiven prüfen («Zip Slip»), freie Namen finden, Archiv-Bomben erkennen,
//! Kommandozeile lesen.
//!
//! Eigenständig prüfbar: `rustc --edition 2021 --test src/regeln.rs -o /tmp/r && /tmp/r`

#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Archivart
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Art {
    Zip,
    SiebenZ,
    Tar,
    TarGz,
    /// Einzelne, mit gzip gepackte Datei (kein TAR darin).
    Gz,
}

impl Art {
    pub fn name(self) -> &'static str {
        match self {
            Art::Zip => "ZIP",
            Art::SiebenZ => "7z",
            Art::Tar => "TAR",
            Art::TarGz => "TAR.GZ",
            Art::Gz => "GZ",
        }
    }

    pub fn endung(self) -> &'static str {
        match self {
            Art::Zip => "zip",
            Art::SiebenZ => "7z",
            Art::Tar => "tar",
            Art::TarGz => "tar.gz",
            Art::Gz => "gz",
        }
    }

    /// Kann diese Art ein Passwort tragen?
    pub fn kann_passwort(self) -> bool {
        matches!(self, Art::Zip | Art::SiebenZ)
    }
}

/// Endungen, die Packwerk kennt (für «Öffnen mit» und Dateidialoge).
pub const ENDUNGEN: [&str; 5] = ["zip", "7z", "tar", "gz", "tgz"];

/// Art aus dem Dateinamen (Gross-/Kleinschreibung egal).
pub fn art_aus_name(name: &str) -> Option<Art> {
    let n = name.to_lowercase();
    if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        Some(Art::TarGz)
    } else if n.ends_with(".tar") {
        Some(Art::Tar)
    } else if n.ends_with(".zip") {
        Some(Art::Zip)
    } else if n.ends_with(".7z") {
        Some(Art::SiebenZ)
    } else if n.ends_with(".gz") {
        Some(Art::Gz)
    } else {
        None
    }
}

/// Signatur eines TAR-Kopfs («ustar» an Stelle 257).
pub fn ist_tar_kopf(kopf: &[u8]) -> bool {
    kopf.get(257..262) == Some(b"ustar".as_slice())
}

/// Art aus den ersten Bytes der Datei (mindestens 262 Bytes für TAR, sonst reichen 6).
/// Bei gzip wird hier nur `Gz` gemeldet; ob ein TAR darin steckt, entscheidet
/// [`art_bestimmen`] mit dem Namen bzw. der Motor mit einem Blick ins Entpackte.
pub fn art_aus_magie(kopf: &[u8]) -> Option<Art> {
    if kopf.starts_with(b"PK\x03\x04")
        || kopf.starts_with(b"PK\x05\x06")
        || kopf.starts_with(b"PK\x07\x08")
    {
        Some(Art::Zip)
    } else if kopf.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
        Some(Art::SiebenZ)
    } else if kopf.starts_with(&[0x1F, 0x8B]) {
        Some(Art::Gz)
    } else if ist_tar_kopf(kopf) {
        Some(Art::Tar)
    } else {
        None
    }
}

/// Art aus Inhalt und Name: der Inhalt hat Vorrang (falsche Endungen kommen vor),
/// der Name entscheidet bei gzip zwischen TAR.GZ und GZ und hilft bei alten TARs ohne «ustar».
pub fn art_bestimmen(name: &str, kopf: &[u8]) -> Option<Art> {
    let nach_name = art_aus_name(name);
    match art_aus_magie(kopf) {
        Some(Art::Gz) => match nach_name {
            Some(Art::TarGz) => Some(Art::TarGz),
            _ => Some(Art::Gz),
        },
        Some(a) => Some(a),
        None => match nach_name {
            Some(Art::Tar) => Some(Art::Tar),
            _ => None,
        },
    }
}

/// Name ohne Archiv-Endung, z. B. «Fotos.tar.gz» → «Fotos». Leer, wenn nichts übrig bleibt.
pub fn name_ohne_endung(dateiname: &str) -> String {
    let lower = dateiname.to_lowercase();
    for e in [".tar.gz", ".tgz", ".tar", ".zip", ".7z", ".gz"] {
        if lower.ends_with(e) && dateiname.len() >= e.len() {
            let cut = dateiname.len() - e.len();
            if dateiname.is_char_boundary(cut) {
                return dateiname[..cut].trim().to_string();
            }
        }
    }
    dateiname.trim().to_string()
}

/// Dateiname für eine entpackte .gz-Datei: «bericht.txt.gz» → «bericht.txt».
pub fn gz_zielname(dateiname: &str) -> String {
    let n = name_ohne_endung(dateiname);
    if n.is_empty() {
        "entpackt".to_string()
    } else {
        n
    }
}

// ---------------------------------------------------------------------------
// Pfade aus Archiven («Zip Slip»)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PfadFehler {
    /// Kein Name übrig (z. B. «./»).
    Leer,
    /// Beginnt mit «/», «\», «C:» oder «\\server».
    Absolut,
    /// Enthält «..».
    Elternverweis,
    /// Unter Windows ungültiger oder reservierter Name, oder Steuerzeichen.
    Ungueltig(String),
}

const WIN_RESERVIERT: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

fn teil_pruefen(teil: &str, windows: bool) -> Result<(), PfadFehler> {
    if teil.chars().any(|c| c == '\0' || (c as u32) < 0x20) {
        return Err(PfadFehler::Ungueltig(teil.to_string()));
    }
    if windows {
        if teil
            .chars()
            .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        {
            return Err(PfadFehler::Ungueltig(teil.to_string()));
        }
        if teil.ends_with('.') || teil.ends_with(' ') {
            return Err(PfadFehler::Ungueltig(teil.to_string()));
        }
        let stamm = teil
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_lowercase();
        if WIN_RESERVIERT.contains(&stamm.as_str()) {
            return Err(PfadFehler::Ungueltig(teil.to_string()));
        }
    }
    Ok(())
}

/// Prüft einen Eintragsnamen aus einem Archiv und liefert seine Bestandteile.
/// «\» gilt als Trenner, «.» und leere Teile fallen weg. Abgelehnt werden absolute Pfade,
/// Laufwerksangaben, «..» (auch wenn es sich wieder aufheben würde) und – unter Windows –
/// ungültige Namen. Das Ergebnis lässt sich gefahrlos an den Zielordner hängen.
pub fn sichere_teile(name: &str, windows: bool) -> Result<Vec<String>, PfadFehler> {
    let n = name.replace('\\', "/");
    if n.starts_with('/') {
        return Err(PfadFehler::Absolut);
    }
    let mut teile = Vec::new();
    for (i, teil) in n.split('/').enumerate() {
        if teil.is_empty() || teil == "." {
            continue;
        }
        if teil == ".." {
            return Err(PfadFehler::Elternverweis);
        }
        // «C:» oder «C:foo» am Anfang = Laufwerk (auf jedem System ablehnen).
        if i == 0 {
            let b = teil.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                return Err(PfadFehler::Absolut);
            }
        }
        teil_pruefen(teil, windows)?;
        teile.push(teil.to_string());
    }
    if teile.is_empty() {
        return Err(PfadFehler::Leer);
    }
    Ok(teile)
}

/// Bestandteile für die Anzeige im Baum (ohne Prüfung): «\» → «/», leere Teile und «.» weg.
pub fn anzeige_teile(name: &str) -> Vec<String> {
    name.replace('\\', "/")
        .split('/')
        .filter(|t| !t.is_empty() && *t != ".")
        .map(str::to_string)
        .collect()
}

/// Zielpfad für einen Eintrag: Zielordner + geprüfte Teile, wobei die ersten `weglassen`
/// Teile (der Ordner, in dem man in der Ansicht gerade steht) entfallen. `None`, wenn nichts
/// übrig bleibt (der Ordner selbst).
pub fn zielpfad(ziel: &Path, teile: &[String], weglassen: usize) -> Option<PathBuf> {
    let rest = teile.get(weglassen..)?;
    if rest.is_empty() {
        return None;
    }
    let mut p = ziel.to_path_buf();
    for t in rest {
        p.push(t);
    }
    Some(p)
}

/// Zweite Sicherung: liegt `kandidat` rein lexikalisch innerhalb von `ziel`?
pub fn liegt_innerhalb(ziel: &Path, kandidat: &Path) -> bool {
    use std::path::Component;
    let Ok(rest) = kandidat.strip_prefix(ziel) else {
        return false;
    };
    rest.components().all(|c| matches!(c, Component::Normal(_)))
        && rest.components().next().is_some()
}

// ---------------------------------------------------------------------------
// Freie Namen
// ---------------------------------------------------------------------------

/// Teilt «bild.tar.gz» in («bild», «.tar.gz»), «foto.jpg» in («foto», «.jpg»),
/// «.profile» und «ordner» in (Name, «»).
pub fn stamm_und_endung(name: &str) -> (String, String) {
    let lower = name.to_lowercase();
    if lower.ends_with(".tar.gz") && name.len() > 7 {
        let cut = name.len() - 7;
        if name.is_char_boundary(cut) {
            return (name[..cut].to_string(), name[cut..].to_string());
        }
    }
    match name.rfind('.') {
        Some(p) if p > 0 => (name[..p].to_string(), name[p..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

/// Erster freie Name nach dem Muster «name (2).ext», «name (3).ext» … (oder `pfad` selbst,
/// wenn er frei ist). `belegt` prüft, ob ein Pfad schon existiert.
pub fn freier_pfad(pfad: &Path, belegt: impl Fn(&Path) -> bool) -> PathBuf {
    if !belegt(pfad) {
        return pfad.to_path_buf();
    }
    let name = pfad
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stamm, endung) = stamm_und_endung(&name);
    let eltern = pfad.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 2u32..100_000 {
        let p = eltern.join(format!("{stamm} ({n}){endung}"));
        if !belegt(&p) {
            return p;
        }
    }
    eltern.join(format!("{stamm} ({}){endung}", std::process::id()))
}

/// Macht Namen in einer Liste eindeutig («a.txt», «a (2).txt» …), ohne Gross/Klein zu
/// unterscheiden (Windows). Für die obersten Einträge eines neuen Archivs.
pub fn eindeutige_namen(namen: &[String]) -> Vec<String> {
    let mut vergeben: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(namen.len());
    for n in namen {
        let mut kandidat = n.clone();
        let (stamm, endung) = stamm_und_endung(n);
        let mut i = 2;
        while vergeben.iter().any(|v| v.eq_ignore_ascii_case(&kandidat)) {
            kandidat = format!("{stamm} ({i}){endung}");
            i += 1;
        }
        vergeben.push(kandidat.clone());
        out.push(kandidat);
    }
    out
}

// ---------------------------------------------------------------------------
// Archiv-Bomben
// ---------------------------------------------------------------------------

pub const GROSS_GRENZE: u64 = 8 * 1024 * 1024 * 1024; // 8 GiB
pub const VERHAELTNIS_GRENZE: u64 = 100;
pub const VERHAELTNIS_MINDESTGROESSE: u64 = 256 * 1024 * 1024; // 256 MiB
pub const ANZAHL_GRENZE: usize = 200_000;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Warnung {
    /// Entpackt sehr gross (Bytes).
    SehrGross(u64),
    /// Ungewöhnlich stark gepackt (Faktor).
    HoheKompression(u64),
    /// Sehr viele Einträge.
    VieleEintraege(usize),
}

/// Warnungen vor dem Entpacken: `gesamt` = Summe der entpackten Grössen,
/// `archiv` = Grösse der Archivdatei, `anzahl` = Zahl der Einträge.
pub fn bomben_warnungen(gesamt: u64, archiv: u64, anzahl: usize) -> Vec<Warnung> {
    let mut w = Vec::new();
    if gesamt > GROSS_GRENZE {
        w.push(Warnung::SehrGross(gesamt));
    }
    if gesamt > VERHAELTNIS_MINDESTGROESSE {
        let faktor = gesamt / archiv.max(1);
        if faktor > VERHAELTNIS_GRENZE {
            w.push(Warnung::HoheKompression(faktor));
        }
    }
    if anzahl > ANZAHL_GRENZE {
        w.push(Warnung::VieleEintraege(anzahl));
    }
    w
}

// ---------------------------------------------------------------------------
// Kommandozeile
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Start {
    /// Ohne Argumente.
    Leer,
    /// `packwerk <archiv>` – Archiv anzeigen (auch für «Öffnen mit»).
    Oeffnen(PathBuf),
    /// `packwerk --entpacken <archiv> [ziel]`.
    Entpacken {
        archiv: PathBuf,
        ziel: Option<PathBuf>,
    },
    /// `packwerk datei1 ordner2 …` (keine Archive) – neues Archiv mit diesen Einträgen.
    Packen(Vec<PathBuf>),
    /// `--hilfe`.
    Hilfe,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StartFehler {
    /// `--entpacken` ohne Archiv.
    ArchivFehlt,
    /// Zu viele Angaben nach `--entpacken`.
    ZuVieleAngaben,
    /// Unbekannte Option.
    UnbekannteOption(String),
}

/// Liest die Argumente (ohne Programmnamen).
pub fn start_lesen(args: Vec<OsString>) -> Result<Start, StartFehler> {
    let mut entpacken = false;
    let mut hilfe = false;
    let mut pfade: Vec<PathBuf> = Vec::new();
    let mut nur_pfade = false;
    for a in args {
        let s = a.to_string_lossy();
        if !nur_pfade && s.starts_with('-') && s.len() > 1 {
            match s.as_ref() {
                "--" => nur_pfade = true,
                "--entpacken" | "--extract" | "-x" | "-e" => entpacken = true,
                "--hilfe" | "--help" | "-h" | "-?" => hilfe = true,
                _ => return Err(StartFehler::UnbekannteOption(s.into_owned())),
            }
            continue;
        }
        pfade.push(PathBuf::from(a));
    }
    if hilfe {
        return Ok(Start::Hilfe);
    }
    if entpacken {
        let mut it = pfade.into_iter();
        let archiv = it.next().ok_or(StartFehler::ArchivFehlt)?;
        let ziel = it.next();
        if it.next().is_some() {
            return Err(StartFehler::ZuVieleAngaben);
        }
        return Ok(Start::Entpacken { archiv, ziel });
    }
    match pfade.len() {
        0 => Ok(Start::Leer),
        1 => {
            let name = pfade[0]
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if art_aus_name(&name).is_some() {
                Ok(Start::Oeffnen(pfade.remove(0)))
            } else {
                Ok(Start::Packen(pfade))
            }
        }
        _ => Ok(Start::Packen(pfade)),
    }
}

/// Standard-Zielordner beim Entpacken: neben dem Archiv, benannt nach dem Archiv
/// («Ferien.zip» → «Ferien»). `ersatz` gilt, wenn vom Namen nichts übrig bleibt.
pub fn standard_ziel(archiv: &Path, ersatz: &str) -> PathBuf {
    let eltern = archiv.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = archiv
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut stamm = name_ohne_endung(&name);
    if stamm.is_empty() || stamm == "." || stamm == ".." {
        stamm = ersatz.to_string();
    }
    eltern.join(stamm)
}

/// Standardname für ein neues Archiv aus den Quellen: eine Quelle → ihr Name,
/// mehrere → Name des gemeinsamen Ordners, sonst `ersatz`.
pub fn archivname_vorschlag(quellen: &[PathBuf], ersatz: &str) -> String {
    let name_von = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let roh = match quellen {
        [] => None,
        [eine] => name_von(eine).map(|n| {
            if eine.is_dir() {
                n
            } else {
                stamm_und_endung(&n).0
            }
        }),
        mehrere => {
            let eltern = mehrere[0].parent();
            if mehrere.iter().all(|q| q.parent() == eltern) {
                eltern.and_then(name_von)
            } else {
                None
            }
        }
    };
    match roh {
        Some(n) if !n.trim().is_empty() => n,
        _ => ersatz.to_string(),
    }
}

/// Teile eines Quellpfads unterhalb der Wurzel zu einem Archivnamen mit «/» verbinden.
pub fn archiv_eintragsname(oben: &str, unterteile: &[String]) -> String {
    let mut s = oben.to_string();
    for t in unterteile {
        s.push('/');
        s.push_str(t);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_erkennen() {
        assert_eq!(art_aus_name("A.TAR.GZ"), Some(Art::TarGz));
        assert_eq!(art_aus_name("a.tgz"), Some(Art::TarGz));
        assert_eq!(art_aus_name("a.gz"), Some(Art::Gz));
        assert_eq!(art_aus_name("a.7Z"), Some(Art::SiebenZ));
        assert_eq!(art_aus_name("a.rar"), None);
        assert_eq!(art_aus_magie(b"PK\x03\x04rest"), Some(Art::Zip));
        assert_eq!(art_aus_magie(b"PK\x05\x06"), Some(Art::Zip));
        assert_eq!(
            art_aus_magie(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C, 0, 4]),
            Some(Art::SiebenZ)
        );
        let mut tar = vec![0u8; 512];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(art_aus_magie(&tar), Some(Art::Tar));
        assert_eq!(art_aus_magie(b"hallo"), None);
        assert_eq!(art_aus_magie(&[]), None);
    }

    #[test]
    fn art_mit_name_und_inhalt() {
        // Inhalt geht vor: eine ZIP-Datei mit falscher Endung bleibt ZIP.
        assert_eq!(art_bestimmen("x.7z", b"PK\x03\x04"), Some(Art::Zip));
        assert_eq!(art_bestimmen("x.tgz", &[0x1F, 0x8B, 8]), Some(Art::TarGz));
        assert_eq!(art_bestimmen("x.log.gz", &[0x1F, 0x8B, 8]), Some(Art::Gz));
        // Altes TAR ohne «ustar»: Name entscheidet.
        assert_eq!(art_bestimmen("alt.tar", &[0u8; 600]), Some(Art::Tar));
        assert_eq!(art_bestimmen("text.txt", b"hallo"), None);
    }

    #[test]
    fn endungen_weg() {
        assert_eq!(name_ohne_endung("Ferien 2026.tar.gz"), "Ferien 2026");
        assert_eq!(name_ohne_endung("a.ZIP"), "a");
        assert_eq!(name_ohne_endung("bericht.txt.gz"), "bericht.txt");
        assert_eq!(name_ohne_endung(".zip"), "");
        assert_eq!(gz_zielname(".gz"), "entpackt");
        assert_eq!(name_ohne_endung("ohne"), "ohne");
        assert_eq!(name_ohne_endung("Grüße.7z"), "Grüße");
    }

    #[test]
    fn zip_slip_wird_abgelehnt() {
        assert_eq!(
            sichere_teile("../../etc/passwd", false),
            Err(PfadFehler::Elternverweis)
        );
        assert_eq!(
            sichere_teile("a/../../b", false),
            Err(PfadFehler::Elternverweis)
        );
        assert_eq!(
            sichere_teile("a/../b", false),
            Err(PfadFehler::Elternverweis)
        );
        assert_eq!(
            sichere_teile("..\\..\\Windows\\x.dll", false),
            Err(PfadFehler::Elternverweis)
        );
        assert_eq!(
            sichere_teile("/etc/passwd", false),
            Err(PfadFehler::Absolut)
        );
        assert_eq!(
            sichere_teile("\\\\server\\share\\x", false),
            Err(PfadFehler::Absolut)
        );
        assert_eq!(
            sichere_teile("C:\\Windows\\x", false),
            Err(PfadFehler::Absolut)
        );
        assert_eq!(sichere_teile("c:x", true), Err(PfadFehler::Absolut));
        assert_eq!(sichere_teile("./", false), Err(PfadFehler::Leer));
        assert_eq!(sichere_teile("", false), Err(PfadFehler::Leer));
        assert!(matches!(
            sichere_teile("a\0b", false),
            Err(PfadFehler::Ungueltig(_))
        ));
    }

    #[test]
    fn gute_pfade() {
        assert_eq!(
            sichere_teile("a/b/c.txt", false).unwrap(),
            vec!["a", "b", "c.txt"]
        );
        assert_eq!(sichere_teile("./a//b/", false).unwrap(), vec!["a", "b"]);
        assert_eq!(
            sichere_teile("ordner\\datei.txt", true).unwrap(),
            vec!["ordner", "datei.txt"]
        );
        assert_eq!(
            sichere_teile("..hidden/x", false).unwrap(),
            vec!["..hidden", "x"]
        );
        assert_eq!(sichere_teile("Ä ö/ü…", true).unwrap(), vec!["Ä ö", "ü…"]);
        // Doppelpunkt ist unter Linux erlaubt, unter Windows nicht (Datenströme).
        assert!(sichere_teile("a/b:c", false).is_ok());
        assert!(sichere_teile("a/b:c", true).is_err());
    }

    #[test]
    fn windows_namen() {
        assert!(sichere_teile("CON", true).is_err());
        assert!(sichere_teile("ordner/nul.txt", true).is_err());
        assert!(sichere_teile("ordner/Com1 .log", true).is_err());
        assert!(sichere_teile("console.txt", true).is_ok());
        assert!(sichere_teile("punkt.", true).is_err());
        assert!(sichere_teile("punkt.", false).is_ok());
        assert!(sichere_teile("a?b", true).is_err());
    }

    #[test]
    fn zielpfade() {
        let z = Path::new("/ziel");
        let teile: Vec<String> = vec!["a".into(), "b".into(), "c.txt".into()];
        assert_eq!(
            zielpfad(z, &teile, 0),
            Some(PathBuf::from("/ziel/a/b/c.txt"))
        );
        assert_eq!(zielpfad(z, &teile, 2), Some(PathBuf::from("/ziel/c.txt")));
        assert_eq!(zielpfad(z, &teile, 3), None);
        assert_eq!(zielpfad(z, &teile, 9), None);
        assert!(liegt_innerhalb(z, Path::new("/ziel/a/b")));
        assert!(!liegt_innerhalb(z, Path::new("/ziel")));
        assert!(!liegt_innerhalb(z, Path::new("/ziel/../x")));
        assert!(!liegt_innerhalb(z, Path::new("/zielx/a")));
        assert!(!liegt_innerhalb(z, Path::new("/anders")));
    }

    #[test]
    fn freie_namen() {
        let belegt = ["/z/a.txt", "/z/a (2).txt", "/z/b.tar.gz", "/z/ordner"];
        let f = |p: &Path| belegt.iter().any(|b| Path::new(b) == p);
        assert_eq!(
            freier_pfad(Path::new("/z/neu.txt"), f),
            PathBuf::from("/z/neu.txt")
        );
        assert_eq!(
            freier_pfad(Path::new("/z/a.txt"), f),
            PathBuf::from("/z/a (3).txt")
        );
        assert_eq!(
            freier_pfad(Path::new("/z/b.tar.gz"), f),
            PathBuf::from("/z/b (2).tar.gz")
        );
        assert_eq!(
            freier_pfad(Path::new("/z/ordner"), f),
            PathBuf::from("/z/ordner (2)")
        );
        assert_eq!(
            stamm_und_endung(".profile"),
            (".profile".to_string(), String::new())
        );
    }

    #[test]
    fn eindeutig() {
        let n: Vec<String> = ["a.txt", "A.txt", "b", "a.txt"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            eindeutige_namen(&n),
            vec!["a.txt", "A (2).txt", "b", "a (3).txt"]
        );
    }

    #[test]
    fn bomben() {
        assert!(bomben_warnungen(10_000, 5_000, 3).is_empty());
        // 1 GiB aus 1 MiB: Faktor 1024.
        let w = bomben_warnungen(1 << 30, 1 << 20, 1);
        assert_eq!(w, vec![Warnung::HoheKompression(1024)]);
        // Kleine, stark gepackte Datei (z. B. leere Bilder) ist kein Problem.
        assert!(bomben_warnungen(100 << 20, 1 << 10, 1).is_empty());
        let w = bomben_warnungen(20 << 30, 15 << 30, 300_000);
        assert_eq!(
            w,
            vec![
                Warnung::SehrGross(20 << 30),
                Warnung::VieleEintraege(300_000)
            ]
        );
        // Archivgrösse 0 darf nicht durch null teilen.
        assert_eq!(
            bomben_warnungen(1 << 30, 0, 1),
            vec![Warnung::HoheKompression(1 << 30)]
        );
    }

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn kommandozeile() {
        assert_eq!(start_lesen(os(&[])), Ok(Start::Leer));
        assert_eq!(
            start_lesen(os(&["a.zip"])),
            Ok(Start::Oeffnen("a.zip".into()))
        );
        assert_eq!(
            start_lesen(os(&["/x/b.TGZ"])),
            Ok(Start::Oeffnen("/x/b.TGZ".into()))
        );
        assert_eq!(
            start_lesen(os(&["--entpacken", "a.7z"])),
            Ok(Start::Entpacken {
                archiv: "a.7z".into(),
                ziel: None
            })
        );
        assert_eq!(
            start_lesen(os(&["a.7z", "--entpacken", "/ziel"])),
            Ok(Start::Entpacken {
                archiv: "a.7z".into(),
                ziel: Some("/ziel".into())
            })
        );
        assert_eq!(
            start_lesen(os(&["--entpacken"])),
            Err(StartFehler::ArchivFehlt)
        );
        assert_eq!(
            start_lesen(os(&["-x", "a", "b", "c"])),
            Err(StartFehler::ZuVieleAngaben)
        );
        assert_eq!(
            start_lesen(os(&["--quatsch"])),
            Err(StartFehler::UnbekannteOption("--quatsch".into()))
        );
        assert_eq!(
            start_lesen(os(&["bild.jpg"])),
            Ok(Start::Packen(vec!["bild.jpg".into()]))
        );
        assert_eq!(
            start_lesen(os(&["a.zip", "b.zip"])),
            Ok(Start::Packen(vec!["a.zip".into(), "b.zip".into()]))
        );
        assert_eq!(
            start_lesen(os(&["--", "-komisch.zip"])),
            Ok(Start::Oeffnen("-komisch.zip".into()))
        );
        assert_eq!(start_lesen(os(&["-h"])), Ok(Start::Hilfe));
        assert_eq!(start_lesen(os(&["-"])), Ok(Start::Packen(vec!["-".into()])));
    }

    #[test]
    fn vorschlaege() {
        assert_eq!(
            standard_ziel(Path::new("/d/Ferien.zip"), "Archiv"),
            PathBuf::from("/d/Ferien")
        );
        assert_eq!(
            standard_ziel(Path::new("/d/.zip"), "Archiv"),
            PathBuf::from("/d/Archiv")
        );
        assert_eq!(
            standard_ziel(Path::new("x.tar.gz"), "Archiv"),
            PathBuf::from("x")
        );
        assert_eq!(archivname_vorschlag(&[], "Archiv"), "Archiv");
        assert_eq!(
            archivname_vorschlag(&["/nicht/da/bericht.txt".into()], "Archiv"),
            "bericht"
        );
        assert_eq!(
            archivname_vorschlag(
                &[
                    "/home/oli/Fotos/a.jpg".into(),
                    "/home/oli/Fotos/b.jpg".into()
                ],
                "Archiv"
            ),
            "Fotos"
        );
        assert_eq!(
            archivname_vorschlag(&["/a/x".into(), "/b/y".into()], "Archiv"),
            "Archiv"
        );
        assert_eq!(
            archiv_eintragsname("Fotos", &["2026".into(), "a.jpg".into()]),
            "Fotos/2026/a.jpg"
        );
        assert_eq!(anzeige_teile("./a\\b//c/"), vec!["a", "b", "c"]);
    }
}
