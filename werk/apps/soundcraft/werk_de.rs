//! Werkbank: German interface texts.
//!
//! SoundCraft has no string catalog of its own: menu labels, command names, dialogs and tooltips
//! are English literals. When the Werkbank launcher asks for German (`WERKBANK_LANG`, else
//! `sprache.txt` in the Werkbank settings folder), [`t`] looks each English text up in
//! `werk_de.tsv` (English TAB German, `\n`, `\t` and `\\` escaped) and returns the German text.
//! Otherwise, and for every text missing from the table, it returns the English text unchanged.

use std::collections::HashMap;
use std::sync::OnceLock;

/// English → German, loaded once.
const TABLE: &str = include_str!("werk_de.tsv");

/// «de», «en» or None, as chosen in the Werkbank launcher.
/// Only with a valid Werkbank licence (lizenz.txt, not expired).
fn werkbank_licence() -> bool {
    (|| -> Option<bool> { let base = (if cfg!(windows) { std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Werkbank").join("config") } else if cfg!(target_os = "macos") { std::path::PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Werkbank") } else { std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?.join("werkbank") }); let t = std::fs::read_to_string(base.join("lizenz.txt")).ok()?; let ablauf: u64 = t.lines().next()?.split('|').nth(3)?.trim().parse().ok()?; Some(ablauf > std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()) })().unwrap_or(false)
}

fn werkbank_language() -> Option<String> {
    if !werkbank_licence() {
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
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The German table, or None when the interface stays English.
fn table() -> Option<&'static HashMap<String, String>> {
    static MAP: OnceLock<Option<HashMap<String, String>>> = OnceLock::new();
    MAP.get_or_init(|| {
        if werkbank_language().as_deref() != Some("de") {
            return None;
        }
        let mut map = HashMap::new();
        for line in TABLE.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((en, de)) = line.split_once('\t') else { continue };
            if !en.is_empty() && !de.is_empty() {
                map.insert(unescape(en), unescape(de));
            }
        }
        Some(map)
    })
    .as_ref()
}

/// Whether the interface is German.
pub fn is_german() -> bool {
    table().is_some()
}

/// The German text for `en`, or `en` itself (English interface, or no translation).
pub fn t<'a>(en: &'a str) -> &'a str {
    match table().and_then(|m| m.get(en)) {
        Some(de) => de.as_str(),
        None => en,
    }
}

/// A menu item's text: the whole menu path (`Window > Edit`) first, for the few labels whose
/// translation depends on the menu, then the label alone.
pub fn menu<'a>(path: &str, label: &'a str) -> &'a str {
    match table().and_then(|m| m.get(path)) {
        Some(de) if path.contains(" > ") => de.as_str(),
        _ => t(label),
    }
}

/// A whole menu path (`Window > Edit`) for display, each part translated as in the menu bar and
/// joined with `sep`.
pub fn menu_path(path: &str, sep: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut end = 0;
    for part in path.split(" > ") {
        end += part.len();
        parts.push(menu(path.get(..end).unwrap_or(part), part));
        end += " > ".len();
    }
    parts.join(sep)
}
