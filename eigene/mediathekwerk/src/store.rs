//! Lokale Speicherung: Einstellungen (TMDB-Schlüssel, Sprache) und die eigene
//! Sammlung als JSON-Dateien im Benutzerordner.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::Entry;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub api_key: String,
    pub lang: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_key: String::new(),
            lang: "de-DE".into(),
        }
    }
}

fn base_dir() -> PathBuf {
    let root = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(root).join("Mediathekwerk")
}

fn config_path() -> PathBuf {
    base_dir().join("config.json")
}
fn library_path() -> PathBuf {
    base_dir().join("library.json")
}

pub fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(c: &Config) {
    let _ = std::fs::create_dir_all(base_dir());
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(config_path(), s);
    }
}

pub fn load_library() -> Vec<Entry> {
    std::fs::read_to_string(library_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_library(lib: &[Entry]) {
    let _ = std::fs::create_dir_all(base_dir());
    if let Ok(s) = serde_json::to_string_pretty(lib) {
        let _ = std::fs::write(library_path(), s);
    }
}
