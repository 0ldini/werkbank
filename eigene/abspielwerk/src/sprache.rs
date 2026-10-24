//! Sprache der Oberfläche: Deutsch (Grundsprache) oder Englisch, gewählt im Werkbank-Launcher.
//!
//! Reihenfolge: Umgebungsvariable `WERKBANK_LANG` (setzt der Launcher beim Start eines
//! Programms), dann die Datei `sprache.txt` im Einstellungsordner der Werkbank, sonst Deutsch.
//!
//! Im Code stehen die deutschen Texte; `t("Deutscher Text")` liefert in Englisch die
//! Übersetzung aus `en.tsv` (eine Zeile pro Text: Deutsch, Tabulator, Englisch; `\n` und `\t`
//! als Escape; Zeilen mit `#` sind Kommentare). Fehlt eine Übersetzung, bleibt der deutsche Text.
//! Für Texte mit Werten: `tf("{n} Dateien umwandeln", &[("n", &n)])`.
//!
//! Diese Datei ist in allen eigenen Programmen gleich (Vorlage: eigene/sprache.rs).

#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    De,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::De => "de",
            Lang::En => "en",
        }
    }
}

/// «de», «de-CH», «en_US.UTF-8», «English» … → Sprache.
pub fn parse(s: &str) -> Option<Lang> {
    let s = s.trim().to_lowercase();
    if s.starts_with("de") {
        Some(Lang::De)
    } else if s.starts_with("en") {
        Some(Lang::En)
    } else {
        None
    }
}

/// Datei, in der der Launcher die gewählte Sprache ablegt.
pub fn werkbank_file() -> Option<PathBuf> {
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(base).join("Werkbank").join("config").join("sprache.txt"))
    } else if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/Werkbank/sprache.txt"))
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("werkbank").join("sprache.txt"))
    }
}

fn detect() -> Lang {
    if let Some(l) = std::env::var("WERKBANK_LANG").ok().as_deref().and_then(parse) {
        return l;
    }
    if let Some(l) = werkbank_file().and_then(|p| std::fs::read_to_string(p).ok()).as_deref().and_then(parse) {
        return l;
    }
    Lang::De
}

/// 0 = noch nicht bestimmt, 1 = Deutsch, 2 = Englisch.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::De,
        2 => Lang::En,
        _ => {
            let l = detect();
            set_lang(l);
            l
        }
    }
}

/// Sprache während des Laufs wechseln (z. B. im Launcher).
pub fn set_lang(l: Lang) {
    CURRENT.store(if l == Lang::En { 2 } else { 1 }, Ordering::Relaxed);
}

pub fn is_en() -> bool {
    lang() == Lang::En
}

const EN_TSV: &str = include_str!("en.tsv");

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\t", "\t")
}

fn table() -> &'static HashMap<String, String> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        EN_TSV
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .filter(|(_, en)| !en.trim().is_empty())
            .map(|(de, en)| (unescape(de), unescape(en.trim_end_matches('\r'))))
            .collect()
    })
}

/// Übersetzt einen deutschen Text in die gewählte Sprache.
pub fn t(de: &'static str) -> &'static str {
    if lang() == Lang::De {
        return de;
    }
    table().get(de).map(String::as_str).unwrap_or(de)
}

/// Wie `t`, setzt danach Werte für `{name}` ein.
pub fn tf(de: &'static str, args: &[(&str, &dyn Display)]) -> String {
    let mut s = t(de).to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprache_erkennen() {
        assert_eq!(parse("de-CH"), Some(Lang::De));
        assert_eq!(parse("en_US.UTF-8"), Some(Lang::En));
        assert_eq!(parse("fr"), None);
    }

    #[test]
    fn werte_einsetzen() {
        set_lang(Lang::De);
        assert_eq!(tf("{n} von {m}", &[("n", &3), ("m", &"vier")]), "3 von vier");
    }

    /// Jeder Text in `t("…")`/`tf("…")` der Quelldateien hat eine englische Übersetzung,
    /// und jede Zeile in en.tsv ist gültig. Aufruf im Programmordner:
    /// `rustc --edition 2021 --test src/sprache.rs -o /tmp/sprache && /tmp/sprache`
    #[test]
    fn alles_uebersetzt() {
        for (i, l) in EN_TSV.lines().enumerate() {
            if l.trim().is_empty() || l.trim_start().starts_with('#') {
                continue;
            }
            let Some((de, en)) = l.split_once('\t') else { panic!("en.tsv Zeile {}: kein Tabulator: {l}", i + 1) };
            assert!(!de.is_empty() && !en.trim().is_empty(), "en.tsv Zeile {}: leer", i + 1);
            let ph = |s: &str| {
                let mut v: Vec<String> = s.match_indices('{').filter_map(|(p, _)| s[p..].find('}').map(|e| s[p..p + e + 1].to_string())).collect();
                v.sort();
                v
            };
            assert_eq!(ph(de), ph(en), "en.tsv Zeile {}: Platzhalter verschieden", i + 1);
        }
        let dir = std::path::Path::new(file!()).parent().expect("Ordner");
        let mut missing = Vec::new();
        let mut files = vec![dir.to_path_buf()];
        while let Some(d) = files.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    files.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") || p.file_name().is_some_and(|n| n == "sprache.rs") {
                    continue;
                }
                let src = std::fs::read_to_string(&p).unwrap_or_default();
                for start in ["t(\"", "tf(\""] {
                    for (pos, _) in src.match_indices(start) {
                        // nur echte Aufrufe, nicht z. B. «print(" oder «.split("»
                        let before = src[..pos].chars().last();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                            continue;
                        }
                        let rest = &src[pos + start.len()..];
                        let mut lit = String::new();
                        let mut chars = rest.chars();
                        while let Some(c) = chars.next() {
                            match c {
                                '\\' => match chars.next() {
                                    Some('n') => lit.push_str("\\n"),
                                    Some('t') => lit.push_str("\\t"),
                                    Some(o) => lit.push(o),
                                    None => break,
                                },
                                '"' => break,
                                c => lit.push(c),
                            }
                        }
                        if !table().contains_key(&unescape(&lit)) {
                            missing.push(format!("{}: {lit}", p.display()));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "Ohne Übersetzung in en.tsv:\n{}", missing.join("\n"));
    }
}
