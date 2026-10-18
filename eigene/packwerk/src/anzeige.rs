//! Reine Anzeige-Logik ohne Oberfläche: Grössen und Zeiten formatieren und aus der flachen
//! Eintragsliste eines Archivs einen Ordnerbaum bauen (auch für Ordner, die im Archiv nur
//! implizit vorkommen, z. B. «a/b/c.txt» ohne eigenen Eintrag «a/»).
//!
//! Eigenständig prüfbar: `rustc --edition 2021 --test src/anzeige.rs -o /tmp/a && /tmp/a`

#![allow(dead_code)]

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Grössen
// ---------------------------------------------------------------------------

/// «0 B», «999 B», «1.5 KB», «12.0 MB», «3.25 GB» (Basis 1024).
pub fn groesse(bytes: u64) -> String {
    const EINHEITEN: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut wert = bytes as f64 / 1024.0;
    let mut i = 0;
    while wert >= 1024.0 && i + 1 < EINHEITEN.len() {
        wert /= 1024.0;
        i += 1;
    }
    if wert < 10.0 {
        format!("{wert:.2} {}", EINHEITEN[i])
    } else if wert < 100.0 {
        format!("{wert:.1} {}", EINHEITEN[i])
    } else {
        format!("{wert:.0} {}", EINHEITEN[i])
    }
}

// ---------------------------------------------------------------------------
// Zeiten
// ---------------------------------------------------------------------------

/// Kalenderzeit ohne Zeitzone (so, wie das Archiv sie liefert).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct Zeit {
    pub jahr: i32,
    pub monat: u8,
    pub tag: u8,
    pub stunde: u8,
    pub minute: u8,
    pub sekunde: u8,
}

impl Zeit {
    /// «2026-10-18 14:03».
    pub fn text(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.jahr, self.monat, self.tag, self.stunde, self.minute
        )
    }

    /// Aus Sekunden seit 1970-01-01 (UTC). Werte ausserhalb von Jahr 1–9999 → `None`.
    pub fn aus_unix(sek: i64) -> Option<Zeit> {
        let tage = sek.div_euclid(86_400);
        let rest = sek.rem_euclid(86_400);
        let (j, m, t) = datum_aus_tagen(tage)?;
        if !(1..=9999).contains(&j) {
            return None;
        }
        Some(Zeit {
            jahr: j as i32,
            monat: m,
            tag: t,
            stunde: (rest / 3600) as u8,
            minute: (rest % 3600 / 60) as u8,
            sekunde: (rest % 60) as u8,
        })
    }

    /// Sekunden seit 1970-01-01, wenn man die Zeit als UTC nimmt.
    pub fn als_unix(&self) -> i64 {
        tage_aus_datum(self.jahr as i64, self.monat as i64, self.tag as i64) * 86_400
            + self.stunde as i64 * 3600
            + self.minute as i64 * 60
            + self.sekunde as i64
    }
}

/// Tage seit 1970-01-01 → (Jahr, Monat, Tag), nach Howard Hinnant.
pub fn datum_aus_tagen(tage: i64) -> Option<(i64, u8, u8)> {
    let z = tage.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    Some((if m <= 2 { y + 1 } else { y }, m, d))
}

/// (Jahr, Monat, Tag) → Tage seit 1970-01-01.
pub fn tage_aus_datum(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Windows-Dateizeit (100-ns-Schritte seit 1601) → Sekunden seit 1970.
pub fn unix_aus_nt(nt: u64) -> i64 {
    (nt / 10_000_000) as i64 - 11_644_473_600
}

// ---------------------------------------------------------------------------
// Baum
// ---------------------------------------------------------------------------

/// Ein Eintrag, wie der Baum ihn braucht.
#[derive(Clone, Debug, Default)]
pub struct Blatt {
    pub teile: Vec<String>,
    pub ist_ordner: bool,
    pub groesse: u64,
    pub gepackt: Option<u64>,
    pub zeit: Option<Zeit>,
}

#[derive(Clone, Debug)]
pub struct Knoten {
    pub name: String,
    pub ist_ordner: bool,
    /// Index in der Eintragsliste des Archivs (`None` bei impliziten Ordnern).
    pub eintrag: Option<usize>,
    /// Bei Ordnern: Summe aller Dateien darunter.
    pub groesse: u64,
    pub gepackt: Option<u64>,
    pub zeit: Option<Zeit>,
    pub eltern: Option<usize>,
    pub kinder: Vec<usize>,
    /// Zahl der Dateien darunter (bei Dateien 1).
    pub dateien: u64,
}

#[derive(Clone, Debug)]
pub struct Baum {
    /// Knoten 0 ist die Wurzel.
    pub knoten: Vec<Knoten>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sortierung {
    Name,
    Groesse,
    Gepackt,
    Zeit,
}

impl Baum {
    pub fn neu(blaetter: &[Blatt]) -> Baum {
        let wurzel = Knoten {
            name: String::new(),
            ist_ordner: true,
            eintrag: None,
            groesse: 0,
            gepackt: None,
            zeit: None,
            eltern: None,
            kinder: Vec::new(),
            dateien: 0,
        };
        let mut knoten = vec![wurzel];
        // (Eltern, Name) → Knoten, nur für Ordner.
        let mut ordner: HashMap<(usize, String), usize> = HashMap::new();
        for (i, b) in blaetter.iter().enumerate() {
            if b.teile.is_empty() {
                continue;
            }
            let mut eltern = 0usize;
            let letzter = b.teile.len() - 1;
            for (k, teil) in b.teile.iter().enumerate() {
                let ist_letzter = k == letzter;
                if ist_letzter && !b.ist_ordner {
                    let idx = knoten.len();
                    knoten.push(Knoten {
                        name: teil.clone(),
                        ist_ordner: false,
                        eintrag: Some(i),
                        groesse: b.groesse,
                        gepackt: b.gepackt,
                        zeit: b.zeit,
                        eltern: Some(eltern),
                        kinder: Vec::new(),
                        dateien: 1,
                    });
                    knoten[eltern].kinder.push(idx);
                    break;
                }
                let key = (eltern, teil.clone());
                let idx = match ordner.get(&key) {
                    Some(&idx) => idx,
                    None => {
                        let idx = knoten.len();
                        knoten.push(Knoten {
                            name: teil.clone(),
                            ist_ordner: true,
                            eintrag: None,
                            groesse: 0,
                            gepackt: None,
                            zeit: None,
                            eltern: Some(eltern),
                            kinder: Vec::new(),
                            dateien: 0,
                        });
                        knoten[eltern].kinder.push(idx);
                        ordner.insert(key, idx);
                        idx
                    }
                };
                if ist_letzter {
                    knoten[idx].eintrag = Some(i);
                    knoten[idx].zeit = b.zeit;
                }
                eltern = idx;
            }
        }
        let mut baum = Baum { knoten };
        baum.summen(0);
        baum
    }

    /// Summiert Grössen und Dateizahlen von unten nach oben (ohne Rekursion).
    fn summen(&mut self, _wurzel: usize) {
        // Knoten werden immer nach ihren Eltern angelegt: rückwärts laufen genügt.
        for i in (1..self.knoten.len()).rev() {
            let (g, p, d, e) = {
                let k = &self.knoten[i];
                (k.groesse, k.gepackt, k.dateien, k.eltern)
            };
            if let Some(e) = e {
                let el = &mut self.knoten[e];
                el.groesse = el.groesse.saturating_add(g);
                el.dateien = el.dateien.saturating_add(d);
                if let Some(p) = p {
                    el.gepackt = Some(el.gepackt.unwrap_or(0).saturating_add(p));
                }
            }
        }
    }

    /// Kinder eines Ordners, sortiert (Ordner zuerst).
    pub fn kinder_sortiert(&self, ordner: usize, nach: Sortierung, absteigend: bool) -> Vec<usize> {
        let Some(k) = self.knoten.get(ordner) else {
            return Vec::new();
        };
        let mut v = k.kinder.clone();
        v.sort_by(|&a, &b| {
            let (ka, kb) = (&self.knoten[a], &self.knoten[b]);
            let ord = kb.ist_ordner.cmp(&ka.ist_ordner);
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
            let o = match nach {
                Sortierung::Name => natuerlich(&ka.name, &kb.name),
                Sortierung::Groesse => ka.groesse.cmp(&kb.groesse),
                Sortierung::Gepackt => ka.gepackt.cmp(&kb.gepackt),
                Sortierung::Zeit => ka.zeit.cmp(&kb.zeit),
            }
            .then_with(|| natuerlich(&ka.name, &kb.name));
            if absteigend {
                o.reverse()
            } else {
                o
            }
        });
        v
    }

    /// Alle Eintragsindizes unter einem Knoten (inklusive des Knotens selbst).
    pub fn eintraege_unter(&self, start: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stapel = vec![start];
        while let Some(i) = stapel.pop() {
            let Some(k) = self.knoten.get(i) else {
                continue;
            };
            if let Some(e) = k.eintrag {
                out.push(e);
            }
            stapel.extend(k.kinder.iter().copied());
        }
        out.sort_unstable();
        out
    }

    /// Pfad von der Wurzel bis zum Knoten (ohne Wurzel), z. B. ["a", "b"].
    pub fn pfad(&self, knoten: usize) -> Vec<String> {
        let mut teile = Vec::new();
        let mut i = knoten;
        let mut schritte = 0;
        while let Some(k) = self.knoten.get(i) {
            let Some(e) = k.eltern else { break };
            teile.push(k.name.clone());
            i = e;
            schritte += 1;
            if schritte > self.knoten.len() {
                break;
            }
        }
        teile.reverse();
        teile
    }
}

/// Natürlicher Vergleich ohne Gross/Klein: «bild2» vor «bild10».
pub fn natuerlich(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n1 = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n1.push(c);
                    x.next();
                }
                let mut n2 = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    n2.push(d);
                    y.next();
                }
                let t1 = n1.trim_start_matches('0');
                let t2 = n2.trim_start_matches('0');
                let o = t1.len().cmp(&t2.len()).then_with(|| t1.cmp(t2));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groessen() {
        assert_eq!(groesse(0), "0 B");
        assert_eq!(groesse(1023), "1023 B");
        assert_eq!(groesse(1024), "1.00 KB");
        assert_eq!(groesse(1536), "1.50 KB");
        assert_eq!(groesse(50 * 1024 * 1024), "50.0 MB");
        assert_eq!(groesse(300 * 1024 * 1024), "300 MB");
        assert_eq!(groesse(u64::MAX), "16384 PB");
    }

    #[test]
    fn zeiten() {
        assert_eq!(Zeit::aus_unix(0).unwrap().text(), "1970-01-01 00:00");
        // 2026-10-18 14:03:05 UTC
        let z = Zeit::aus_unix(1_792_332_185).unwrap();
        assert_eq!(z.text(), "2026-10-18 14:03");
        assert_eq!(z.sekunde, 5);
        assert_eq!(z.als_unix(), 1_792_332_185);
        // Schalttag
        assert_eq!(
            Zeit::aus_unix(951_782_400).unwrap().text(),
            "2000-02-29 00:00"
        );
        assert_eq!(Zeit::aus_unix(-86_400).unwrap().text(), "1969-12-31 00:00");
        assert_eq!(Zeit::aus_unix(i64::MAX), None);
        assert_eq!(Zeit::aus_unix(i64::MIN), None);
        assert_eq!(unix_aus_nt(116_444_736_000_000_000), 0);
        for t in [0i64, 59, 86_399, 1_000_000_000, 4_102_444_800] {
            assert_eq!(Zeit::aus_unix(t).unwrap().als_unix(), t);
        }
    }

    fn blatt(name: &str, ordner: bool, g: u64) -> Blatt {
        Blatt {
            teile: name
                .split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            ist_ordner: ordner,
            groesse: g,
            gepackt: Some(g / 2),
            zeit: None,
        }
    }

    #[test]
    fn baum_bauen() {
        let b = vec![
            blatt("a/b/c.txt", false, 10),
            blatt("a/", true, 0),
            blatt("a/d.txt", false, 5),
            blatt("z.txt", false, 1),
            blatt("a/b/", true, 0),
        ];
        let baum = Baum::neu(&b);
        let wurzel = baum.kinder_sortiert(0, Sortierung::Name, false);
        assert_eq!(wurzel.len(), 2);
        let a = wurzel[0];
        assert_eq!(baum.knoten[a].name, "a");
        assert!(baum.knoten[a].ist_ordner);
        assert_eq!(baum.knoten[a].eintrag, Some(1));
        assert_eq!(baum.knoten[a].groesse, 15);
        assert_eq!(baum.knoten[a].gepackt, Some(7));
        assert_eq!(baum.knoten[a].dateien, 2);
        assert_eq!(baum.knoten[0].groesse, 16);
        let unter_a = baum.kinder_sortiert(a, Sortierung::Name, false);
        assert_eq!(baum.knoten[unter_a[0]].name, "b");
        assert_eq!(baum.knoten[unter_a[1]].name, "d.txt");
        assert_eq!(baum.eintraege_unter(a), vec![0, 1, 2, 4]);
        assert_eq!(baum.pfad(unter_a[0]), vec!["a", "b"]);
        assert_eq!(baum.pfad(0), Vec::<String>::new());
        let nach_groesse = baum.kinder_sortiert(a, Sortierung::Groesse, true);
        assert_eq!(baum.knoten[nach_groesse[0]].name, "b");
    }

    #[test]
    fn baum_leer_und_kaputt() {
        let baum = Baum::neu(&[]);
        assert_eq!(baum.knoten.len(), 1);
        assert!(baum.kinder_sortiert(99, Sortierung::Name, false).is_empty());
        assert!(baum.eintraege_unter(99).is_empty());
        let baum = Baum::neu(&[
            blatt("", false, 1),
            blatt("x", false, 2),
            blatt("x", false, 3),
        ]);
        // Doppelte Dateinamen bleiben beide sichtbar.
        assert_eq!(baum.knoten[0].kinder.len(), 2);
    }

    #[test]
    fn natuerlich_sortieren() {
        let mut v = vec!["bild10", "Bild2", "bild1", "a"];
        v.sort_by(|a, b| natuerlich(a, b));
        assert_eq!(v, vec!["a", "bild1", "Bild2", "bild10"]);
        assert_eq!(natuerlich("x01", "x1"), "x01".cmp("x1"));
    }
}
