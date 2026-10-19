//! Werkbank: deutsche Oberfläche für DeckCraft.
//!
//! DeckCraft hat kein eigenes Übersetzungssystem; die Texte stehen als englische Literale im
//! Code. Die Werkbank macht die sichtbaren Texte beim Bauen über [`t`] übersetzbar und legt die
//! Tabelle `werk_de.tsv` (Englisch TAB Deutsch) daneben. Deutsch gilt nur, wenn der
//! Werkbank-Launcher es vorgibt (Umgebungsvariable `WERKBANK_LANG` oder `sprache.txt` im
//! Einstellungsordner der Werkbank); sonst bleibt alles englisch wie im Original.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Englisch TAB Deutsch, eine Zeile je Text; `\n`, `\t` und `\\` sind maskiert, `#` beginnt einen
/// Kommentar.
const TABLE: &str = include_str!("werk_de.tsv");

/// Die Sprache, die der Werkbank-Launcher vorgibt: «de», «en» oder None.
/// Gilt nur mit gültiger Lizenz der Werkbank (lizenz.txt, Ablauf nicht erreicht).
fn werkbank_lizenz() -> bool {
    (|| -> Option<bool> { let base = (if cfg!(windows) { std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Werkbank").join("config") } else if cfg!(target_os = "macos") { std::path::PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Werkbank") } else { std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?.join("werkbank") }); let t = std::fs::read_to_string(base.join("lizenz.txt")).ok()?; let ablauf: u64 = t.lines().next()?.split('|').nth(3)?.trim().parse().ok()?; Some(ablauf > std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()) })().unwrap_or(false)
}

fn werkbank_lang() -> Option<String> {
    if !werkbank_lizenz() {
        return None;
    }
    std::env::var("WERKBANK_LANG")
        .ok()
        .or_else(|| {
            let base = if cfg!(windows) {
                std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Werkbank").join("config")
            } else if cfg!(target_os = "macos") {
                std::path::PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Werkbank")
            } else {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?
                    .join("werkbank")
            };
            std::fs::read_to_string(base.join("sprache.txt")).ok()
        })
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| v == "de" || v == "en")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The table, or None when the interface stays English. Built once.
fn table() -> Option<&'static HashMap<String, String>> {
    static MAP: OnceLock<Option<HashMap<String, String>>> = OnceLock::new();
    MAP.get_or_init(|| {
        // Unit tests look for the English labels.
        if cfg!(test) || werkbank_lang().as_deref() != Some("de") {
            return None;
        }
        let mut map = HashMap::new();
        for line in TABLE.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((en, de)) = line.split_once('\t')
                && !de.is_empty()
            {
                map.insert(unescape(en), unescape(de));
            }
        }
        Some(map)
    })
    .as_ref()
}

/// True when the Werkbank asks for the German interface.
pub fn is_de() -> bool {
    table().is_some()
}

/// The German text for `en`, or `en` itself (English interface, or no translation).
pub fn t(en: &str) -> &str {
    match table().and_then(|m| m.get(en)) {
        Some(de) => de.as_str(),
        None => en,
    }
}

/// [`t`] for text that is built at run time.
pub fn t_dyn(en: &str) -> String {
    t(en).to_string()
}

/// A translated message with `{}` placeholders, filled in order with `args` (like `format!`
/// with positional `{}` only). `{{` and `}}` stand for braces.
pub fn f(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let template = t(template);
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = args.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '{' if chars.peek() == Some(&'}') => {
                chars.next();
                if let Some(a) = next.next() {
                    out.push_str(&a.to_string());
                }
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            other => out.push(other),
        }
    }
    out
}
