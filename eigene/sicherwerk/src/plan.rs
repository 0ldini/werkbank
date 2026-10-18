//! Planung ohne Dateizugriff: Spiegel-Vergleich, Versionsnamen, Aufbewahrung, sichere
//! Pfade. Nur std; Test: `rustc --edition 2021 --test src/plan.rs`.

#![allow(dead_code)]

use std::collections::HashMap;

/// Eine Datei: Pfad ab dem Quellordner (mit `/`), Grösse, Änderungszeit (Sekunden seit 1970).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Eintrag {
    pub rel: String,
    pub groesse: u64,
    pub zeit: i64,
}

/// Gilt eine Datei als unverändert? Gleiche Grösse und Änderungszeit, wobei
/// - bis 2 Sekunden Unterschied zählen nicht (FAT32/exFAT auf USB-Sticks speichern grob),
/// - genau eine Stunde (±2 s) zählt auch nicht (FAT speichert Ortszeit: Sommerzeit-Wechsel).
pub fn gleich(a_groesse: u64, a_zeit: i64, b_groesse: u64, b_zeit: i64) -> bool {
    if a_groesse != b_groesse {
        return false;
    }
    let d = a_zeit.saturating_sub(b_zeit).saturating_abs();
    d <= 2 || (3598..=3602).contains(&d)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpiegelPlan {
    /// Indizes in `quelle`: im Ziel noch nicht vorhanden.
    pub neu: Vec<usize>,
    /// Indizes in `quelle`: im Ziel vorhanden, aber anders.
    pub ersetzen: Vec<usize>,
    /// Indizes in `ziel`: in der Quelle nicht mehr vorhanden.
    pub entfernen: Vec<usize>,
    /// Anzahl unveränderter Dateien.
    pub gleich: u64,
    /// Ordner der Quelle, die im Ziel fehlen.
    pub ordner_neu: Vec<String>,
    /// Ordner im Ziel, die es in der Quelle nicht mehr gibt (tiefste zuerst).
    pub ordner_weg: Vec<String>,
}

impl SpiegelPlan {
    pub fn bytes(&self, quelle: &[Eintrag]) -> u64 {
        self.neu
            .iter()
            .chain(self.ersetzen.iter())
            .filter_map(|&i| quelle.get(i))
            .fold(0u64, |s, e| s.saturating_add(e.groesse))
    }
}

fn schluessel(rel: &str, gross_klein_egal: bool) -> String {
    if gross_klein_egal {
        rel.to_lowercase()
    } else {
        rel.to_string()
    }
}

/// Liegt `rel` in einem geschützten Pfad (Ordner, der nicht vollständig gelesen werden
/// konnte, oder Datei, deren Angaben fehlen)? Dann wird im Ziel nichts davon entfernt.
pub fn geschuetzt(rel: &str, schutz: &[String], gross_klein_egal: bool) -> bool {
    let r = schluessel(rel, gross_klein_egal);
    schutz.iter().any(|s| {
        if s.is_empty() {
            return true; // ganzer Quellordner unsicher
        }
        let s = schluessel(s, gross_klein_egal);
        r == s || (r.len() > s.len() && r.starts_with(&s) && r.as_bytes().get(s.len()) == Some(&b'/'))
    })
}

/// Vergleicht Quelle und Ziel.
pub fn spiegeln(
    quelle: &[Eintrag],
    quell_ordner: &[String],
    ziel: &[Eintrag],
    ziel_ordner: &[String],
    schutz: &[String],
    gross_klein_egal: bool,
) -> SpiegelPlan {
    let mut plan = SpiegelPlan::default();
    let ziel_map: HashMap<String, usize> =
        ziel.iter().enumerate().map(|(i, e)| (schluessel(&e.rel, gross_klein_egal), i)).collect();
    let mut gesehen = vec![false; ziel.len()];
    for (i, q) in quelle.iter().enumerate() {
        match ziel_map.get(&schluessel(&q.rel, gross_klein_egal)) {
            Some(&zi) => {
                if let Some(g) = gesehen.get_mut(zi) {
                    *g = true;
                }
                match ziel.get(zi) {
                    Some(z) if gleich(q.groesse, q.zeit, z.groesse, z.zeit) => plan.gleich += 1,
                    _ => plan.ersetzen.push(i),
                }
            }
            None => plan.neu.push(i),
        }
    }
    for (zi, z) in ziel.iter().enumerate() {
        if !gesehen[zi] && !geschuetzt(&z.rel, schutz, gross_klein_egal) {
            plan.entfernen.push(zi);
        }
    }
    let ziel_o: std::collections::HashSet<String> =
        ziel_ordner.iter().map(|o| schluessel(o, gross_klein_egal)).collect();
    let quell_o: std::collections::HashSet<String> =
        quell_ordner.iter().map(|o| schluessel(o, gross_klein_egal)).collect();
    plan.ordner_neu =
        quell_ordner.iter().filter(|o| !ziel_o.contains(&schluessel(o, gross_klein_egal))).cloned().collect();
    let mut weg: Vec<String> = ziel_ordner
        .iter()
        .filter(|o| !quell_o.contains(&schluessel(o, gross_klein_egal)) && !geschuetzt(o, schutz, gross_klein_egal))
        .cloned()
        .collect();
    weg.sort_by(|a, b| b.matches('/').count().cmp(&a.matches('/').count()).then(b.cmp(a)));
    plan.ordner_weg = weg;
    plan
}

// ------------------------------------------------------------------ Versionen

pub const UNVOLLSTAENDIG: &str = ".unvollstaendig";

/// «2026-10-18_14-30-05» (optional «-2», «-3» … bei mehreren Läufen in derselben Sekunde),
/// optional mit «.zip». Liefert (Name ohne .zip, ist_zip).
pub fn versionsname(s: &str) -> Option<(&str, bool)> {
    let (stamm, zip) = match s.strip_suffix(".zip") {
        Some(st) => (st, true),
        None => (s, false),
    };
    let b = stamm.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let muster = b"dddd-dd-dd_dd-dd-dd";
    for (i, m) in muster.iter().enumerate() {
        let c = *b.get(i)?;
        let ok = if *m == b'd' { c.is_ascii_digit() } else { c == *m };
        if !ok {
            return None;
        }
    }
    let rest = stamm.get(19..)?;
    if !rest.is_empty() {
        let n = rest.strip_prefix('-')?;
        if n.is_empty() || n.len() > 4 || !n.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    Some((stamm, zip))
}

/// Sortierschlüssel: Datum, dann die Zusatznummer als Zahl.
fn sortier(stamm: &str) -> (String, u32) {
    let datum = stamm.get(..19).unwrap_or(stamm).to_string();
    let n = stamm.get(20..).and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
    (datum, n)
}

/// Freier Name für eine neue Version: `basis`, sonst `basis-2`, `basis-3` …
pub fn neuer_versionsname(basis: &str, vorhanden: &[String]) -> String {
    let belegt = |n: &str| {
        vorhanden.iter().any(|v| {
            let v = v.strip_suffix(UNVOLLSTAENDIG).unwrap_or(v);
            let v = v.strip_suffix(".zip").unwrap_or(v);
            v == n
        })
    };
    if !belegt(basis) {
        return basis.to_string();
    }
    for i in 2..10_000 {
        let n = format!("{basis}-{i}");
        if !belegt(&n) {
            return n;
        }
    }
    format!("{basis}-x")
}

/// Fertige Versionen aus den Einträgen eines Auftragsordners, älteste zuerst.
pub fn versionen(eintraege: &[String]) -> Vec<String> {
    let mut v: Vec<&String> = eintraege.iter().filter(|e| versionsname(e).is_some()).collect();
    v.sort_by_key(|e| sortier(versionsname(e).map(|x| x.0).unwrap_or(e)));
    v.into_iter().cloned().collect()
}

/// Welche Versionen fallen weg, wenn nur die letzten `behalten` bleiben (mindestens 1)?
pub fn zu_loeschen(eintraege: &[String], behalten: u32) -> Vec<String> {
    let v = versionen(eintraege);
    let behalten = (behalten.max(1)) as usize;
    if v.len() <= behalten {
        return Vec::new();
    }
    v[..v.len() - behalten].to_vec()
}

// ------------------------------------------------------------------ Pfade

/// Prüft einen relativen Pfad (mit `/`) aus einer Sicherung, bevor daraus ein Zielpfad
/// wird: keine leeren Teile, kein `.`/`..`, kein Laufwerk, keine verbotenen Zeichen.
pub fn sicherer_rel(rel: &str) -> Option<Vec<&str>> {
    let rel = rel.trim_end_matches('/');
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\\') {
        return None;
    }
    let teile: Vec<&str> = rel.split('/').collect();
    for t in &teile {
        if t.is_empty() || *t == "." || *t == ".." || t.contains(':') || t.chars().any(|c| c.is_control()) {
            return None;
        }
    }
    Some(teile)
}

/// Freier Dateiname, falls `name` schon belegt ist: «Brief (2).txt», «Brief (3).txt» …
pub fn freier_name(name: &str, belegt: impl Fn(&str) -> bool) -> String {
    if !belegt(name) {
        return name.to_string();
    }
    let (stamm, endung) = match name.rfind('.') {
        Some(p) if p > 0 => (&name[..p], &name[p..]),
        _ => (name, ""),
    };
    for i in 2..100_000 {
        let n = format!("{stamm} ({i}){endung}");
        if !belegt(&n) {
            return n;
        }
    }
    format!("{stamm} (neu){endung}")
}

/// «1.5 GB» usw.
pub fn groesse_text(b: u64) -> String {
    const E: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < E.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", E[i])
    } else {
        format!("{v:.1} {}", E[i])
    }
}

/// «1 h 05 min», «3 min 12 s», «8 s».
pub fn dauer_text(s: u64) -> String {
    if s >= 3600 {
        format!("{} h {:02} min", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

/// «2026-10-18_14-30-05(-2)(.zip)» → «2026-10-18 14:30:05 (2)»; anderes bleibt, wie es ist.
pub fn datum_anzeige(name: &str) -> String {
    let Some((stamm, _)) = versionsname(name) else { return name.to_string() };
    let datum = stamm.get(..10).unwrap_or("");
    let zeit = stamm.get(11..19).unwrap_or("").replace('-', ":");
    match stamm.get(20..) {
        Some(n) if !n.is_empty() => format!("{datum} {zeit} ({n})"),
        _ => format!("{datum} {zeit}"),
    }
}

/// Einfacher Zufall für Stichproben (xorshift).
pub struct Zufall(u64);

impl Zufall {
    pub fn neu(saat: u64) -> Self {
        Zufall(saat | 1)
    }
    pub fn naechste(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// `k` verschiedene Indizes aus `0..n`.
    pub fn auswahl(&mut self, n: usize, k: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..n).collect();
        let k = k.min(n);
        for i in 0..k {
            let span = (n - i) as u64;
            let j = i + (self.naechste() % span) as usize;
            v.swap(i, j);
        }
        v.truncate(k);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(rel: &str, g: u64, z: i64) -> Eintrag {
        Eintrag { rel: rel.into(), groesse: g, zeit: z }
    }

    #[test]
    fn vergleich() {
        assert!(gleich(5, 100, 5, 102));
        assert!(!gleich(5, 100, 5, 103));
        assert!(gleich(5, 100, 5, 100 + 3600));
        assert!(gleich(5, 100 + 3601, 5, 100));
        assert!(!gleich(5, 100, 6, 100));
        assert!(gleich(1, i64::MIN, 1, i64::MIN));
        assert!(!gleich(1, i64::MIN, 1, i64::MAX));
    }

    #[test]
    fn spiegel() {
        let q = vec![e("a.txt", 1, 10), e("b/c.txt", 2, 20), e("d.txt", 3, 30), e("E.txt", 4, 40)];
        let z = vec![e("a.txt", 1, 11), e("b/c.txt", 2, 99), e("x.txt", 5, 1), e("alt/y.txt", 1, 1), e("e.txt", 4, 40)];
        let qo = vec!["b".to_string(), "neu".to_string()];
        let zo = vec!["b".to_string(), "alt".to_string(), "alt/tief".to_string()];
        let p = spiegeln(&q, &qo, &z, &zo, &[], false);
        assert_eq!(p.neu, vec![2, 3]);
        assert_eq!(p.ersetzen, vec![1]);
        assert_eq!(p.entfernen, vec![2, 3, 4]);
        assert_eq!(p.gleich, 1);
        assert_eq!(p.ordner_neu, vec!["neu".to_string()]);
        assert_eq!(p.ordner_weg, vec!["alt/tief".to_string(), "alt".to_string()]);
        assert_eq!(p.bytes(&q), 3 + 4 + 2);
        // Ohne Gross-/Kleinschreibung (Windows): E.txt == e.txt.
        let p = spiegeln(&q, &qo, &z, &zo, &[], true);
        assert_eq!(p.neu, vec![2]);
        assert_eq!(p.gleich, 2);
        assert_eq!(p.entfernen, vec![2, 3]);
    }

    #[test]
    fn schutz() {
        let q = vec![e("a.txt", 1, 1)];
        let z = vec![e("a.txt", 1, 1), e("kaputt/x.txt", 1, 1), e("kaputtx.txt", 1, 1), e("f.txt", 1, 1)];
        let s = vec!["kaputt".to_string(), "f.txt".to_string()];
        let p = spiegeln(&q, &[], &z, &["kaputt".into()], &s, false);
        assert_eq!(p.entfernen, vec![2]);
        assert!(p.ordner_weg.is_empty());
        // Leerer Schutz = ganzer Ordner geschützt.
        let p = spiegeln(&[], &[], &z, &[], &[String::new()], false);
        assert!(p.entfernen.is_empty());
    }

    #[test]
    fn versionsnamen() {
        assert_eq!(versionsname("2026-10-18_14-30-05"), Some(("2026-10-18_14-30-05", false)));
        assert_eq!(versionsname("2026-10-18_14-30-05.zip"), Some(("2026-10-18_14-30-05", true)));
        assert_eq!(versionsname("2026-10-18_14-30-05-2"), Some(("2026-10-18_14-30-05-2", false)));
        assert_eq!(versionsname("2026-10-18_14-30-05.unvollstaendig"), None);
        assert_eq!(versionsname("2026-10-18_14-30-05-"), None);
        assert_eq!(versionsname("_entfernt"), None);
        assert_eq!(versionsname("Fotos"), None);
        assert_eq!(versionsname("2026-10-18 14-30-05"), None);
        assert_eq!(versionsname("ä026-10-18_14-30-05"), None);
        let v: Vec<String> = ["2026-10-18_14-30-05", "2026-10-18_14-30-05.zip.unvollstaendig"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(neuer_versionsname("2026-10-18_14-30-05", &v), "2026-10-18_14-30-05-2");
        assert_eq!(neuer_versionsname("2026-10-19_00-00-00", &v), "2026-10-19_00-00-00");
    }

    #[test]
    fn aufbewahrung() {
        let e: Vec<String> = [
            "2026-10-03_10-00-00.zip",
            "Fotos",
            "2026-10-01_10-00-00",
            "2026-10-02_10-00-00-10",
            "2026-10-02_10-00-00-2",
            "2026-10-02_10-00-00",
            "2026-10-04_10-00-00.unvollstaendig",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            versionen(&e),
            vec!["2026-10-01_10-00-00", "2026-10-02_10-00-00", "2026-10-02_10-00-00-2", "2026-10-02_10-00-00-10", "2026-10-03_10-00-00.zip"]
        );
        assert_eq!(zu_loeschen(&e, 3), vec!["2026-10-01_10-00-00", "2026-10-02_10-00-00"]);
        assert_eq!(zu_loeschen(&e, 0).len(), 4);
        assert!(zu_loeschen(&e, 50).is_empty());
    }

    #[test]
    fn pfade() {
        assert_eq!(sicherer_rel("Fotos/a.jpg"), Some(vec!["Fotos", "a.jpg"]));
        assert_eq!(sicherer_rel("Fotos/Urlaub/"), Some(vec!["Fotos", "Urlaub"]));
        assert_eq!(sicherer_rel("../x"), None);
        assert_eq!(sicherer_rel("/etc/x"), None);
        assert_eq!(sicherer_rel("a//b"), None);
        assert_eq!(sicherer_rel("C:/x"), None);
        assert_eq!(sicherer_rel("a\\..\\b"), None);
        assert_eq!(sicherer_rel(""), None);
        let belegt = |n: &str| n == "Brief.txt" || n == "Brief (2).txt" || n == "Ordner";
        assert_eq!(freier_name("Brief.txt", belegt), "Brief (3).txt");
        assert_eq!(freier_name("Ordner", belegt), "Ordner (2)");
        assert_eq!(freier_name(".bashrc", |n| n == ".bashrc"), ".bashrc (2)");
        assert_eq!(freier_name("neu.txt", belegt), "neu.txt");
    }

    #[test]
    fn texte() {
        assert_eq!(groesse_text(512), "512 B");
        assert_eq!(groesse_text(1536), "1.5 KB");
        assert_eq!(groesse_text(500 * 1024 * 1024), "500 MB");
        assert_eq!(dauer_text(5), "5 s");
        assert_eq!(dauer_text(65), "1 min 05 s");
        assert_eq!(dauer_text(3720), "1 h 02 min");
        assert_eq!(datum_anzeige("2026-10-18_14-30-05.zip"), "2026-10-18 14:30:05");
        assert_eq!(datum_anzeige("2026-10-18_14-30-05-2"), "2026-10-18 14:30:05 (2)");
        assert_eq!(datum_anzeige("Fotos"), "Fotos");
    }

    #[test]
    fn zufall() {
        let mut z = Zufall::neu(42);
        let a = z.auswahl(100, 10);
        assert_eq!(a.len(), 10);
        let mut b = a.clone();
        b.sort();
        b.dedup();
        assert_eq!(b.len(), 10);
        assert_eq!(z.auswahl(3, 10).len(), 3);
        assert!(z.auswahl(0, 5).is_empty());
    }
}
