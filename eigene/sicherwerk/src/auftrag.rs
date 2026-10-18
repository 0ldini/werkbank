//! Datentypen: Sicherungsaufträge, Protokolle und die gespeicherte Gesamtdatei, dazu
//! Namensregeln. Nur std – serde kommt über `cfg_attr` dazu, damit sich die Datei einzeln
//! prüfen lässt: `rustc --edition 2021 --cfg ohne_serde --test src/auftrag.rs`.
//!
//! Sichtbare Meldungen sind deutsche `&'static str`; die Oberfläche übersetzt sie mit `t()`.

#![allow(dead_code)]

#[cfg(not(ohne_serde))]
use serde::{Deserialize, Serialize};

/// Spiegeln (nur Neues/Geändertes) oder je Lauf eine neue Version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub enum Art {
    Spiegeln,
    Versionen,
}

/// Was beim Spiegeln mit Dateien im Ziel geschieht, die es in der Quelle nicht mehr gibt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub enum Entfernte {
    /// Im Ziel liegen lassen.
    Behalten,
    /// Nach `_entfernt/<datum>/` verschieben (Standard).
    Verschieben,
    /// Im Ziel löschen (nie in der Quelle).
    Loeschen,
}

/// Eine Version als Ordner (Kopie) oder als ZIP-Datei.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub enum VersionForm {
    Ordner,
    Zip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub enum Zeitplan {
    Keiner,
    Taeglich,
    Woechentlich,
    /// Sobald das Ziellaufwerk da ist (einmal pro Tag).
    BeimAnschliessen,
}

impl Zeitplan {
    pub const ALLE: [Zeitplan; 4] = [Zeitplan::Keiner, Zeitplan::Taeglich, Zeitplan::Woechentlich, Zeitplan::BeimAnschliessen];
    pub fn text(self) -> &'static str {
        match self {
            Zeitplan::Keiner => "Nur von Hand",
            Zeitplan::Taeglich => "Täglich",
            Zeitplan::Woechentlich => "Wöchentlich",
            Zeitplan::BeimAnschliessen => "Beim Anschliessen des Ziellaufwerks",
        }
    }
}

/// Wochentag 0 = Montag … 6 = Sonntag.
pub const WOCHENTAGE: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub struct Quelle {
    /// Ordner, der gesichert wird.
    pub pfad: String,
    /// Name des Unterordners im Ziel (eindeutig im Auftrag, bleibt fest).
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
#[cfg_attr(not(ohne_serde), serde(default))]
pub struct Auftrag {
    pub id: u64,
    pub name: String,
    pub quellen: Vec<Quelle>,
    /// Zielordner (USB-Stick, zweite Platte, Netzwerkpfad). Darin legt Sicherwerk einen
    /// Ordner mit dem Auftragsnamen an.
    pub ziel: String,
    /// Ausschlüsse, ein Muster pro Eintrag (siehe regeln.rs).
    pub ausschluesse: Vec<String>,
    pub art: Art,
    pub entfernte: Entfernte,
    pub form: VersionForm,
    /// Letzte N Versionen behalten.
    pub behalten: u32,
    /// Unveränderte Dateien in Versionsordnern als harte Verknüpfung (spart Platz).
    pub verknuepfen: bool,
    /// Anzahl Dateien für «Prüfen».
    pub stichprobe: u32,
    pub zeitplan: Zeitplan,
    pub stunde: u8,
    pub minute: u8,
    /// 0 = Montag … 6 = Sonntag.
    pub wochentag: u8,
    /// Vorschau wurde seit der letzten Änderung von Quellen/Ziel/Art angesehen.
    pub vorschau_gesehen: bool,
    /// In der Windows-Aufgabenplanung eingetragen.
    pub aufgabe_eingerichtet: bool,
}

impl Default for Auftrag {
    fn default() -> Self {
        Auftrag {
            id: 0,
            name: String::new(),
            quellen: Vec::new(),
            ziel: String::new(),
            ausschluesse: ausschluesse_standard(),
            art: Art::Spiegeln,
            entfernte: Entfernte::Verschieben,
            form: VersionForm::Ordner,
            behalten: 10,
            verknuepfen: true,
            stichprobe: 20,
            zeitplan: Zeitplan::Keiner,
            stunde: 18,
            minute: 30,
            wochentag: 4,
            vorschau_gesehen: false,
            aufgabe_eingerichtet: false,
        }
    }
}

impl Auftrag {
    /// Ändert sich etwas davon, braucht es vor dem nächsten Lauf wieder eine Vorschau.
    pub fn gleicher_kern(&self, o: &Auftrag) -> bool {
        self.quellen == o.quellen
            && self.ziel == o.ziel
            && self.art == o.art
            && self.form == o.form
            && self.entfernte == o.entfernte
            && self.ausschluesse == o.ausschluesse
            && self.name == o.name
    }

    pub fn zeitplan_gleich(&self, o: &Auftrag) -> bool {
        self.zeitplan == o.zeitplan
            && self.stunde == o.stunde
            && self.minute == o.minute
            && self.wochentag == o.wochentag
            && self.name == o.name
    }
}

pub fn ausschluesse_standard() -> Vec<String> {
    ["*.tmp", "~$*", "Thumbs.db", ".DS_Store", "$RECYCLE.BIN", "System Volume Information", "node_modules"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Vorschläge zum schnellen Hinzufügen.
pub const AUSSCHLUSS_VORSCHLAEGE: [&str; 9] =
    ["*.tmp", "*.bak", "~$*", "node_modules", "Cache", ".cache", "target", ".git", "*.part"];

// ------------------------------------------------------------------ Protokolle

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub enum Taetigkeit {
    Sichern,
    Pruefen,
    Wiederherstellen,
}

impl Taetigkeit {
    pub fn text(self) -> &'static str {
        match self {
            Taetigkeit::Sichern => "Sichern",
            Taetigkeit::Pruefen => "Prüfen",
            Taetigkeit::Wiederherstellen => "Wiederherstellen",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub enum Ausloeser {
    Hand,
    Zeitplan,
    Anschluss,
}

impl Ausloeser {
    pub fn text(self) -> &'static str {
        match self {
            Ausloeser::Hand => "von Hand",
            Ausloeser::Zeitplan => "Zeitplan",
            Ausloeser::Anschluss => "Laufwerk angeschlossen",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub enum Ergebnis {
    Ok,
    MitFehlern,
    Abgebrochen,
    Fehlgeschlagen,
}

impl Ergebnis {
    pub fn text(self) -> &'static str {
        match self {
            Ergebnis::Ok => "Erfolgreich",
            Ergebnis::MitFehlern => "Mit Fehlern",
            Ergebnis::Abgebrochen => "Abgebrochen",
            Ergebnis::Fehlgeschlagen => "Fehlgeschlagen",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub enum FehlerArt {
    ZielFehlt,
    QuelleFehlt,
    Lesen,
    Kopieren,
    Verschieben,
    Loeschen,
    OrdnerAnlegen,
    Zip,
    Abweichung,
    Sperre,
    Hinweis,
    Sonstiges,
}

impl FehlerArt {
    pub fn text(self) -> &'static str {
        match self {
            FehlerArt::ZielFehlt => "Ziel nicht erreichbar",
            FehlerArt::QuelleFehlt => "Quellordner fehlt",
            FehlerArt::Lesen => "Lesen fehlgeschlagen",
            FehlerArt::Kopieren => "Kopieren fehlgeschlagen",
            FehlerArt::Verschieben => "Verschieben fehlgeschlagen",
            FehlerArt::Loeschen => "Löschen fehlgeschlagen",
            FehlerArt::OrdnerAnlegen => "Ordner anlegen fehlgeschlagen",
            FehlerArt::Zip => "ZIP-Fehler",
            FehlerArt::Abweichung => "Prüfsumme weicht ab",
            FehlerArt::Sperre => "Auftrag läuft bereits",
            FehlerArt::Hinweis => "Hinweis",
            FehlerArt::Sonstiges => "Fehler",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
pub struct Fehler {
    pub art: FehlerArt,
    pub pfad: String,
    pub detail: String,
}

impl Fehler {
    pub fn neu(art: FehlerArt, pfad: impl Into<String>, detail: impl Into<String>) -> Self {
        Fehler { art, pfad: pfad.into(), detail: detail.into() }
    }
}

/// Höchstzahl gespeicherter Einzelfehler je Lauf (gezählt werden alle).
pub const MAX_FEHLER_JE_LAUF: usize = 200;
/// Höchstzahl gespeicherter Protokolle insgesamt (älteste fallen weg).
pub const MAX_PROTOKOLLE: usize = 1000;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
#[cfg_attr(not(ohne_serde), serde(default))]
pub struct Protokoll {
    pub auftrag_id: u64,
    pub auftrag_name: String,
    pub taetigkeit: Taetigkeit,
    pub ausloeser: Ausloeser,
    /// Ortszeit «2026-10-18 14:30:05».
    pub start: String,
    pub start_unix: i64,
    pub dauer_s: u64,
    pub ergebnis: Ergebnis,
    /// Neu kopierte Dateien.
    pub neu: u64,
    /// Ersetzte (geänderte) Dateien.
    pub geaendert: u64,
    /// Im Ziel verschobene/gelöschte Dateien.
    pub entfernt: u64,
    /// Als harte Verknüpfung übernommen.
    pub verknuepft: u64,
    pub unveraendert: u64,
    /// Kopierte Bytes.
    pub bytes: u64,
    /// Name der Version (Ordner/ZIP), falls eine entstand.
    pub version: String,
    pub versionen_geloescht: u64,
    /// Beim Prüfen: verglichene Dateien.
    pub geprueft: u64,
    pub fehler_anzahl: u64,
    pub fehler: Vec<Fehler>,
}

impl Default for Protokoll {
    fn default() -> Self {
        Protokoll {
            auftrag_id: 0,
            auftrag_name: String::new(),
            taetigkeit: Taetigkeit::Sichern,
            ausloeser: Ausloeser::Hand,
            start: String::new(),
            start_unix: 0,
            dauer_s: 0,
            ergebnis: Ergebnis::Ok,
            neu: 0,
            geaendert: 0,
            entfernt: 0,
            verknuepft: 0,
            unveraendert: 0,
            bytes: 0,
            version: String::new(),
            versionen_geloescht: 0,
            geprueft: 0,
            fehler_anzahl: 0,
            fehler: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(ohne_serde), derive(Serialize, Deserialize))]
#[cfg_attr(not(ohne_serde), serde(default))]
pub struct Daten {
    pub format: u32,
    pub naechste_id: u64,
    pub auftraege: Vec<Auftrag>,
    pub protokolle: Vec<Protokoll>,
}

impl Default for Daten {
    fn default() -> Self {
        Daten { format: 1, naechste_id: 1, auftraege: Vec::new(), protokolle: Vec::new() }
    }
}

impl Daten {
    pub fn auftrag(&self, id: u64) -> Option<&Auftrag> {
        self.auftraege.iter().find(|a| a.id == id)
    }

    /// Auftrag über den Namen (Gross-/Kleinschreibung egal) oder «#<id>».
    pub fn finden(&self, name: &str) -> Option<&Auftrag> {
        let name = name.trim();
        if let Some(id) = name.strip_prefix('#').and_then(|s| s.parse::<u64>().ok()) {
            return self.auftrag(id);
        }
        let k = name.to_lowercase();
        self.auftraege.iter().find(|a| a.name.trim().to_lowercase() == k)
    }

    pub fn neue_id(&mut self) -> u64 {
        let max = self.auftraege.iter().map(|a| a.id).max().unwrap_or(0);
        let id = self.naechste_id.max(max.saturating_add(1)).max(1);
        self.naechste_id = id.saturating_add(1);
        id
    }

    pub fn protokoll_anfuegen(&mut self, mut p: Protokoll) {
        if p.fehler.len() > MAX_FEHLER_JE_LAUF {
            p.fehler.truncate(MAX_FEHLER_JE_LAUF);
        }
        self.protokolle.push(p);
        if self.protokolle.len() > MAX_PROTOKOLLE {
            let weg = self.protokolle.len() - MAX_PROTOKOLLE;
            self.protokolle.drain(..weg);
        }
    }

    /// Letztes Protokoll einer Tätigkeit für einen Auftrag.
    pub fn letztes(&self, id: u64, tk: Taetigkeit) -> Option<&Protokoll> {
        self.protokolle.iter().rev().find(|p| p.auftrag_id == id && p.taetigkeit == tk)
    }

    /// Gab es heute (Datum «YYYY-MM-DD») schon eine brauchbare Sicherung?
    pub fn heute_gesichert(&self, id: u64, heute: &str) -> bool {
        self.protokolle.iter().any(|p| {
            p.auftrag_id == id
                && p.taetigkeit == Taetigkeit::Sichern
                && matches!(p.ergebnis, Ergebnis::Ok | Ergebnis::MitFehlern)
                && p.start.starts_with(heute)
        })
    }
}

// ------------------------------------------------------------------ Namen

/// Zeichen, die in Datei-/Ordnernamen unter Windows nicht erlaubt sind.
const VERBOTEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Prüft einen Auftragsnamen. `andere` sind die Namen der übrigen Aufträge.
pub fn name_pruefen(name: &str, andere: &[&str]) -> Result<(), &'static str> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Bitte einen Namen eingeben.");
    }
    if n.chars().count() > 60 {
        return Err("Der Name ist zu lang (höchstens 60 Zeichen).");
    }
    if n.chars().any(|c| VERBOTEN.contains(&c) || c.is_control() || c == '%') {
        return Err("Der Name darf keines dieser Zeichen enthalten: < > : \" / \\ | ? * %");
    }
    if n.starts_with('#') || n.starts_with('-') || n.ends_with('.') {
        return Err("Der Name darf nicht mit # oder - beginnen und nicht mit einem Punkt enden.");
    }
    let k = n.to_lowercase();
    if andere.iter().any(|a| a.trim().to_lowercase() == k) {
        return Err("Es gibt schon einen Auftrag mit diesem Namen.");
    }
    Ok(())
}

/// Macht aus beliebigem Text einen gültigen Ordnernamen (Windows und Linux).
pub fn ordnername(s: &str) -> String {
    let mut out: String =
        s.trim().chars().map(|c| if VERBOTEN.contains(&c) || c.is_control() { '_' } else { c }).collect();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    let up = out.to_uppercase();
    let stamm = up.split('.').next().unwrap_or("");
    let reserviert = ["CON", "PRN", "AUX", "NUL"].contains(&stamm)
        || ((stamm.starts_with("COM") || stamm.starts_with("LPT"))
            && stamm.len() == 4
            && stamm[3..].chars().all(|c| c.is_ascii_digit()));
    if reserviert {
        out.insert(0, '_');
    }
    if out.is_empty() || out == "." || out == ".." {
        out = "Sicherung".to_string();
    }
    out
}

/// Letzter Teil eines Pfads, auch für Windows-Pfade («D:\», «\\nas\fotos») auf jedem System.
pub fn letzter_teil(pfad: &str) -> String {
    let p = pfad.trim().trim_end_matches(['/', '\\']);
    let teil = p.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or("");
    let teil = teil.trim_end_matches(':');
    if teil.is_empty() {
        "Laufwerk".to_string()
    } else {
        teil.to_string()
    }
}

/// Name des Unterordners im Ziel für eine neue Quelle; eindeutig gegenüber `vorhanden`
/// (ohne Gross-/Kleinschreibung) und nie «_entfernt».
pub fn quellname(pfad: &str, vorhanden: &[&str]) -> String {
    let basis = ordnername(&letzter_teil(pfad));
    let belegt = |n: &str| {
        let k = n.to_lowercase();
        k == "_entfernt" || vorhanden.iter().any(|v| v.to_lowercase() == k)
    };
    if !belegt(&basis) {
        return basis;
    }
    for i in 2..10_000u32 {
        let n = format!("{basis} ({i})");
        if !belegt(&n) {
            return n;
        }
    }
    format!("{basis} (x)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namen() {
        assert!(name_pruefen("Fotos", &[]).is_ok());
        assert!(name_pruefen("  ", &[]).is_err());
        assert!(name_pruefen("a/b", &[]).is_err());
        assert!(name_pruefen("Fotos", &["fotos"]).is_err());
        assert!(name_pruefen("50%", &[]).is_err());
        assert!(name_pruefen("Ende.", &[]).is_err());
    }

    #[test]
    fn ordnernamen() {
        assert_eq!(ordnername("Fotos: 2026?"), "Fotos_ 2026_");
        assert_eq!(ordnername("con"), "_con");
        assert_eq!(ordnername("COM1.txt"), "_COM1.txt");
        assert_eq!(ordnername("Daten. "), "Daten");
        assert_eq!(ordnername(""), "Sicherung");
    }

    #[test]
    fn quellnamen() {
        assert_eq!(letzter_teil("C:\\Users\\Oli\\Fotos\\"), "Fotos");
        assert_eq!(letzter_teil("D:\\"), "D");
        assert_eq!(letzter_teil("/"), "Laufwerk");
        assert_eq!(letzter_teil("\\\\nas\\daten"), "daten");
        assert_eq!(letzter_teil("/home/oli/Dokumente"), "Dokumente");
        assert_eq!(quellname("/a/Fotos", &["fotos"]), "Fotos (2)");
        assert_eq!(quellname("/a/_entfernt", &[]), "_entfernt (2)");
        assert_eq!(quellname("/x/Fotos", &["Fotos", "Fotos (2)"]), "Fotos (3)");
    }

    #[test]
    fn daten() {
        let mut d = Daten::default();
        let id = d.neue_id();
        d.auftraege.push(Auftrag { id, name: "Fotos".into(), ..Default::default() });
        assert_eq!(d.neue_id(), id + 1);
        assert!(d.finden("FOTOS").is_some());
        assert!(d.finden(&format!("#{id}")).is_some());
        assert!(d.finden("x").is_none());
        for i in 0..(MAX_PROTOKOLLE + 5) {
            d.protokoll_anfuegen(Protokoll { auftrag_id: id, start_unix: i as i64, ..Default::default() });
        }
        assert_eq!(d.protokolle.len(), MAX_PROTOKOLLE);
        assert_eq!(d.protokolle[0].start_unix, 5);
        let p = Protokoll { auftrag_id: id, start: "2026-10-18 10:00:00".into(), ..Default::default() };
        d.protokoll_anfuegen(p);
        assert!(d.heute_gesichert(id, "2026-10-18"));
        assert!(!d.heute_gesichert(id, "2026-10-19"));
    }
}
