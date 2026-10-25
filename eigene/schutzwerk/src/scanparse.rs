//! Parser für den Virenscan (defensiv): Ausgaben von Microsoft Defender (`MpCmdRun.exe`,
//! PowerShell `Get-MpComputerStatus`/`Get-MpThreat`) und ClamAV (`clamscan`).
//! Schutzwerk hat keine eigenen Signaturen – es zeigt nur, was diese Programme melden.
//! Nur Standardbibliothek – mit `rustc --edition 2021 --test src/scanparse.rs` testbar.

#![allow(dead_code)]

/// Ein Fund mit den betroffenen Dateien.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fund {
    pub name: String,
    pub dateien: Vec<String>,
}

/// Funde aus der Ausgabe von `MpCmdRun.exe -Scan …`.
pub fn parse_mpcmdrun(out: &str) -> Vec<Fund> {
    let mut v: Vec<Fund> = Vec::new();
    for line in out.lines() {
        let l = line.trim_end_matches('\r');
        let Some((key, val)) = l.split_once(':') else { continue };
        // Pfade enthalten selbst «:» (C:\…) – nur den ersten Doppelpunkt trennen.
        let k = key.trim();
        let val = val.trim();
        if k.eq_ignore_ascii_case("Threat") && !val.is_empty() {
            v.push(Fund { name: val.to_string(), dateien: Vec::new() });
        } else if !k.is_empty() && !k.contains(' ') && !k.eq_ignore_ascii_case("Resources") {
            // «    file                : C:\…» (auch «regkey», «process» …)
            if let Some(f) = v.last_mut() {
                if !val.is_empty() {
                    f.dateien.push(val.to_string());
                }
            }
        }
    }
    v
}

/// Ergebnis eines ClamAV-Laufs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClamErgebnis {
    pub funde: Vec<Fund>,
    pub gescannt: Option<u64>,
    pub infiziert: Option<u64>,
}

/// Ausgabe von `clamscan -r -i <Ordner>`: Zeilen «Pfad: Name FOUND» und die Zusammenfassung.
pub fn parse_clamscan(out: &str) -> ClamErgebnis {
    let mut e = ClamErgebnis::default();
    for line in out.lines() {
        let l = line.trim_end_matches('\r');
        if let Some(rest) = l.strip_suffix(" FOUND") {
            if let Some(pos) = rest.rfind(": ") {
                let pfad = rest.get(..pos).unwrap_or("").to_string();
                let name = rest.get(pos + 2..).unwrap_or("").to_string();
                e.funde.push(Fund { name, dateien: vec![pfad] });
            }
        } else if let Some(n) = l.strip_prefix("Scanned files:") {
            e.gescannt = n.trim().parse().ok();
        } else if let Some(n) = l.strip_prefix("Infected files:") {
            e.infiziert = n.trim().parse().ok();
        }
    }
    e
}

/// Schlüssel-Wert-Zeilen («Schlüssel<TAB>Wert»), z. B. vom Defender-Status.
pub fn parse_kv(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| l.trim_end_matches('\r').split_once('\t'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

pub fn kv<'a>(kv: &'a [(String, String)], key: &str) -> Option<&'a str> {
    kv.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

pub fn ja(v: Option<&str>) -> Option<bool> {
    match v?.to_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Eine von Defender erkannte Bedrohung (Verlauf).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bedrohung {
    pub zeit: String,
    pub name: String,
    /// 0 = unbekannt, 1 niedrig, 2 mittel, 4 hoch, 5 schwer
    pub schwere: u8,
    pub aktiv: bool,
    /// Ergebnis (z. B. «Quarantined», «Removed»), wie Defender es meldet.
    pub status: String,
    pub dateien: Vec<String>,
}

/// Zeilen «Zeit<TAB>Name<TAB>Schwere<TAB>Aktiv<TAB>Status<TAB>Datei | Datei».
pub fn parse_bedrohungen(out: &str) -> Vec<Bedrohung> {
    let mut v = Vec::new();
    for line in out.lines() {
        let c: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
        if c.len() < 5 || c[1].trim().is_empty() {
            continue;
        }
        v.push(Bedrohung {
            zeit: c[0].trim().to_string(),
            name: c[1].trim().to_string(),
            schwere: c[2].trim().parse().unwrap_or(0),
            aktiv: c[3].trim().eq_ignore_ascii_case("true"),
            status: c[4].trim().to_string(),
            dateien: c.get(5).map(|s| s.split(" | ").map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default(),
        });
    }
    v.sort_by(|a, b| b.zeit.cmp(&a.zeit));
    v
}

/// Defender-Statuscode einer Erkennung (ThreatStatusID) als Wort.
pub fn status_wort(id: u32) -> &'static str {
    match id {
        1 => "Erkannt",
        2 | 3 => "Bereinigt",
        4 => "In Quarantäne",
        5 => "Entfernt",
        6 => "Zugelassen",
        102 | 103 => "Bereinigung fehlgeschlagen",
        104 => "Quarantäne fehlgeschlagen",
        105 => "Entfernen fehlgeschlagen",
        106 => "Zulassen fehlgeschlagen",
        107 => "Verlassen",
        108 => "Blockiert",
        _ => "Unbekannt",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpcmdrun_fund() {
        let out = "Scan starting...\r\nScan finished.\r\nScanning C:\\Users\\Oli\\Desktop\\eicar.com found 1 threats.\r\n\r\n\
<===========================LIST OF DETECTED THREATS==========================>\r\n\
----------------------------- Threat information ------------------------------\r\n\
Threat                  : Virus:DOS/EICAR_Test_File\r\n\
Resources               : 1 total\r\n\
    file                : C:\\Users\\Oli\\Desktop\\eicar.com\r\n\
-------------------------------------------------------------------------------\r\n";
        let f = parse_mpcmdrun(out);
        assert_eq!(f, vec![Fund { name: "Virus:DOS/EICAR_Test_File".into(), dateien: vec!["C:\\Users\\Oli\\Desktop\\eicar.com".into()] }]);
    }

    #[test]
    fn mpcmdrun_sauber() {
        let out = "Scan starting...\nScan finished.\nScanning C:\\ found no threats.\n";
        assert!(parse_mpcmdrun(out).is_empty());
    }

    #[test]
    fn clam() {
        let out = "/home/oli/Downloads/eicar.txt: Win.Test.EICAR_HDB-1 FOUND\n\
/home/oli/a: b/c.zip: Eicar FOUND\n\n----------- SCAN SUMMARY -----------\nKnown viruses: 8700000\nScanned files: 1234\nInfected files: 2\n";
        let e = parse_clamscan(out);
        assert_eq!(e.funde.len(), 2);
        assert_eq!(e.funde[0].name, "Win.Test.EICAR_HDB-1");
        assert_eq!(e.funde[1].dateien[0], "/home/oli/a: b/c.zip");
        assert_eq!(e.gescannt, Some(1234));
        assert_eq!(e.infiziert, Some(2));
    }

    #[test]
    fn status_lesen() {
        let out = "RealTimeProtectionEnabled\tTrue\r\nAntivirusSignatureAge\t1\r\nAMRunningMode\tNormal\r\nleer\t\r\n";
        let k = parse_kv(out);
        assert_eq!(ja(kv(&k, "realtimeprotectionenabled")), Some(true));
        assert_eq!(kv(&k, "AntivirusSignatureAge"), Some("1"));
        assert_eq!(kv(&k, "leer"), None);
    }

    #[test]
    fn verlauf() {
        let out = "2026-10-01 09:12\tVirus:DOS/EICAR_Test_File\t5\tFalse\tIn Quarantäne\tfile:_C:\\x\\eicar.com | file:_C:\\y\r\n\
2026-10-20 18:00\tTrojan:Win32/Wacatac\t5\tTrue\tErkannt\t\n\
kaputt\n";
        let v = parse_bedrohungen(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "Trojan:Win32/Wacatac");
        assert!(v[0].aktiv);
        assert_eq!(v[1].dateien.len(), 2);
        assert_eq!(status_wort(4), "In Quarantäne");
    }
}
