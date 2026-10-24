//! Parser für die Autostart-Prüfung (defensiv, nur Lesen): Programmdatei aus einer
//! Befehlszeile, Umgebungsvariablen, Tabulator-Ausgaben von PowerShell, crontab,
//! .desktop-Dateien, launchd-plists und systemd-Units; dazu der Abgleich mit dem
//! letzten bekannten Stand.
//! Nur Standardbibliothek – mit `rustc --edition 2021 --test src/autoparse.rs` testbar.

#![allow(dead_code)]

use std::collections::HashSet;

/// Ersetzt `%NAME%` über `lookup` (Gross/Klein egal); Unbekanntes bleibt stehen.
pub fn env_ersetzen(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(rest.get(..a).unwrap_or(""));
        let nach = rest.get(a + 1..).unwrap_or("");
        match nach.find('%') {
            Some(b) if b > 0 && !nach.get(..b).unwrap_or(" ").contains([' ', '\\', '"']) => {
                let name = nach.get(..b).unwrap_or("");
                match lookup(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = nach.get(b + 1..).unwrap_or("");
            }
            _ => {
                out.push('%');
                rest = nach;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Programmdatei aus einer Befehlszeile: `"C:\a b\x.exe" /min` → `C:\a b\x.exe`,
/// `C:\Program Files\x.exe -a` → `C:\Program Files\x.exe`, `\??\C:\x.sys` → `C:\x.sys`.
pub fn exe_aus_befehl(cmd: &str) -> String {
    let c = cmd.trim();
    let c = c.strip_prefix("\\??\\").unwrap_or(c);
    if let Some(rest) = c.strip_prefix('"') {
        return rest.split('"').next().unwrap_or("").trim().to_string();
    }
    let lower = c.to_lowercase();
    for ext in [".exe", ".com", ".bat", ".cmd", ".vbs", ".ps1", ".scr", ".lnk", ".sys"] {
        let mut start = 0;
        while let Some(pos) = lower.get(start..).and_then(|s| s.find(ext)) {
            let end = start + pos + ext.len();
            // Endung muss am Wortende stehen («x.exe» nicht «x.exe2»).
            if lower[end..].chars().next().is_none_or(|ch| ch.is_whitespace() || ch == ',') {
                return c.get(..end).unwrap_or(c).to_string();
            }
            start = end;
        }
    }
    c.split_whitespace().next().unwrap_or("").to_string()
}

/// Einfache Tabulator-Zeilen, leere Zeilen ausgelassen.
pub fn parse_tsv(out: &str) -> Vec<Vec<String>> {
    out.lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').map(|s| s.trim().to_string()).collect())
        .collect()
}

/// Befehle aus `crontab -l` (ohne Kommentare und Variablen).
pub fn parse_crontab(out: &str) -> Vec<String> {
    let mut v = Vec::new();
    for line in out.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix('@') {
            // @reboot befehl
            if let Some((_, cmd)) = rest.split_once(char::is_whitespace) {
                v.push(cmd.trim().to_string());
            }
            continue;
        }
        let first = l.chars().next().unwrap_or(' ');
        if !(first.is_ascii_digit() || first == '*') {
            continue; // z. B. MAILTO=…
        }
        let mut it = l.split_whitespace();
        for _ in 0..5 {
            it.next();
        }
        let cmd: Vec<&str> = it.collect();
        if !cmd.is_empty() {
            v.push(cmd.join(" "));
        }
    }
    v
}

/// Wert eines Schlüssels in einer INI-artigen Datei (.desktop, systemd-Unit), erster Treffer.
pub fn ini_wert(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim();
        if let Some((k, v)) = l.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// .desktop-Autostart: (Name, Exec, deaktiviert?)
pub fn parse_desktop(text: &str) -> (Option<String>, Option<String>, bool) {
    let aus = ini_wert(text, "Hidden").is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || ini_wert(text, "X-GNOME-Autostart-enabled").is_some_and(|v| v.eq_ignore_ascii_case("false"));
    (ini_wert(text, "Name"), ini_wert(text, "Exec"), aus)
}

/// Inhalt des ersten `<string>` nach `<key>NAME</key>` in einer XML-plist.
fn plist_string_nach(xml: &str, key: &str) -> Option<String> {
    let marke = format!("<key>{key}</key>");
    let pos = xml.find(&marke)? + marke.len();
    let rest = xml.get(pos..)?;
    let a = rest.find("<string>")? + 8;
    let b = rest.get(a..)?.find("</string>")? + a;
    // Nur, wenn dazwischen kein anderer Schlüssel beginnt.
    if rest.get(..a)?.contains("<key>") {
        return None;
    }
    Some(xml_entities(rest.get(a..b)?))
}

fn xml_entities(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// launchd-plist (XML): (Label, Programm, deaktiviert?)
pub fn parse_plist(xml: &str) -> (Option<String>, Option<String>, bool) {
    let label = plist_string_nach(xml, "Label");
    let prog = plist_string_nach(xml, "Program").or_else(|| {
        let pos = xml.find("<key>ProgramArguments</key>")?;
        let rest = xml.get(pos..)?;
        let a = rest.find("<string>")? + 8;
        let b = rest.get(a..)?.find("</string>")? + a;
        Some(xml_entities(rest.get(a..b)?))
    });
    let aus = xml
        .find("<key>Disabled</key>")
        .and_then(|p| xml.get(p + 19..))
        .map(|r| r.trim_start().starts_with("<true/>"))
        .unwrap_or(false);
    (label, prog, aus)
}

/// Aktivierte Units aus `systemctl --user list-unit-files --state=enabled --no-legend`.
pub fn parse_unit_files(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|u| u.contains('.') && !u.ends_with(".target"))
        .map(|u| u.to_string())
        .collect()
}

/// Taskmanager-Schalter (StartupApproved): ungerades erstes Byte = deaktiviert.
pub fn startup_approved_aus(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 1 == 1)
}

/// Markiert, welche Einträge im letzten bekannten Stand fehlen (= neu).
/// Ohne bekannten Stand (`basis` = None) gilt nichts als neu.
pub fn neue(ids: &[String], basis: Option<&[String]>) -> Vec<bool> {
    match basis {
        None => vec![false; ids.len()],
        Some(b) => {
            let set: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
            ids.iter().map(|i| !set.contains(i.as_str())).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(n: &str) -> Option<String> {
        match n.to_lowercase().as_str() {
            "windir" | "systemroot" => Some("C:\\Windows".into()),
            "programfiles" => Some("C:\\Program Files".into()),
            _ => None,
        }
    }

    #[test]
    fn umgebung() {
        assert_eq!(env_ersetzen("%SystemRoot%\\system32\\x.exe", env), "C:\\Windows\\system32\\x.exe");
        assert_eq!(env_ersetzen("%unbekannt%\\a", env), "%unbekannt%\\a");
        assert_eq!(env_ersetzen("100% sicher", env), "100% sicher");
        assert_eq!(env_ersetzen("\"%ProgramFiles%\\A\\a.exe\" -x", env), "\"C:\\Program Files\\A\\a.exe\" -x");
    }

    #[test]
    fn programmdatei() {
        assert_eq!(exe_aus_befehl("\"C:\\Program Files\\A B\\x.exe\" /min"), "C:\\Program Files\\A B\\x.exe");
        assert_eq!(exe_aus_befehl("C:\\Program Files\\Tool\\tool.exe -background"), "C:\\Program Files\\Tool\\tool.exe");
        assert_eq!(exe_aus_befehl("C:\\WINDOWS\\system32\\rundll32.exe x.dll,Start"), "C:\\WINDOWS\\system32\\rundll32.exe");
        assert_eq!(exe_aus_befehl("\\??\\C:\\Windows\\drv.sys"), "C:\\Windows\\drv.sys");
        assert_eq!(exe_aus_befehl("C:\\a.exe2\\b.exe"), "C:\\a.exe2\\b.exe");
        assert_eq!(exe_aus_befehl("/usr/bin/env python3 x.py"), "/usr/bin/env");
        assert_eq!(exe_aus_befehl(""), "");
    }

    #[test]
    fn tsv() {
        let v = parse_tsv("\\Ich\\\tUpdate\tReady\r\n\n\\B\\\tX\tDisabled\n");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], vec!["\\Ich\\", "Update", "Ready"]);
    }

    #[test]
    fn cron() {
        let out = "# Kommentar\nMAILTO=oli\n*/5 * * * * /home/oli/.x/run.sh >/dev/null 2>&1\n@reboot /tmp/m\n\n";
        assert_eq!(parse_crontab(out), vec!["/home/oli/.x/run.sh >/dev/null 2>&1".to_string(), "/tmp/m".to_string()]);
    }

    #[test]
    fn desktop() {
        let d = "[Desktop Entry]\nType=Application\nName=Nextcloud\nExec=/usr/bin/nextcloud --background\nX-GNOME-Autostart-enabled=false\n";
        assert_eq!(parse_desktop(d), (Some("Nextcloud".into()), Some("/usr/bin/nextcloud --background".into()), true));
        assert_eq!(ini_wert("[Service]\nExecStart=/home/o/bin/x\n", "ExecStart").as_deref(), Some("/home/o/bin/x"));
    }

    #[test]
    fn plist() {
        let xml = r#"<?xml version="1.0"?><plist><dict>
<key>Label</key>
<string>com.example.helper</string>
<key>ProgramArguments</key>
<array>
  <string>/Users/oli/Library/Application Support/x/helper</string>
  <string>--run</string>
</array>
<key>RunAtLoad</key><true/>
<key>Disabled</key>
<true/>
</dict></plist>"#;
        let (l, p, aus) = parse_plist(xml);
        assert_eq!(l.as_deref(), Some("com.example.helper"));
        assert_eq!(p.as_deref(), Some("/Users/oli/Library/Application Support/x/helper"));
        assert!(aus);
        let xml2 = "<dict><key>Label</key><string>a&amp;b</string><key>Program</key><string>/opt/a</string></dict>";
        assert_eq!(parse_plist(xml2), (Some("a&b".into()), Some("/opt/a".into()), false));
    }

    #[test]
    fn units() {
        let out = "syncthing.service enabled enabled\ndefault.target enabled enabled\npipewire.socket enabled enabled\n";
        assert_eq!(parse_unit_files(out), vec!["syncthing.service".to_string(), "pipewire.socket".to_string()]);
    }

    #[test]
    fn taskmanager_schalter() {
        assert!(!startup_approved_aus(&[2, 0, 0, 0]));
        assert!(startup_approved_aus(&[3, 0, 0, 0]));
        assert!(!startup_approved_aus(&[]));
    }

    #[test]
    fn neu_markieren() {
        let ids = vec!["a".to_string(), "b".to_string()];
        assert_eq!(neue(&ids, None), vec![false, false]);
        let basis = vec!["a".to_string()];
        assert_eq!(neue(&ids, Some(&basis)), vec![false, true]);
    }
}
