//! Datenmodell und reine Logik: Medientyp, Status, Bibliothekseintrag, Filter,
//! TMDB-URL-Bau mit Query-Encoding und Jahr aus einem Datum. Die URL-/Filter-/
//! Encoding-Logik ist ohne Netz und ohne Oberfläche testbar.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Movie,
    Tv,
}

impl Kind {
    pub fn path(self) -> &'static str {
        match self {
            Kind::Movie => "movie",
            Kind::Tv => "tv",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Movie => "Film",
            Kind::Tv => "Serie",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Geplant,
    AmSchauen,
    Gesehen,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Geplant => "Geplant",
            Status::AmSchauen => "Am Schauen",
            Status::Gesehen => "Gesehen",
        }
    }
    pub fn all() -> [Status; 3] {
        [Status::Geplant, Status::AmSchauen, Status::Gesehen]
    }
}

/// Ein Eintrag in der eigenen Sammlung.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub kind: Kind,
    pub title: String,
    pub year: Option<u32>,
    pub rating: f32,
    pub poster: Option<String>,
    pub overview: String,
    pub status: Status,
    pub my_rating: u8, // 0..=5
    #[serde(default)]
    pub seasons: Option<u32>,
    #[serde(default)]
    pub runtime: Option<u32>, // Minuten (Film)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Alle,
    Filme,
    Serien,
    Geplant,
    AmSchauen,
    Gesehen,
}

impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Filter::Alle => "Alle",
            Filter::Filme => "Filme",
            Filter::Serien => "Serien",
            Filter::Geplant => "Geplant",
            Filter::AmSchauen => "Am Schauen",
            Filter::Gesehen => "Gesehen",
        }
    }
    pub fn all() -> [Filter; 6] {
        [
            Filter::Alle,
            Filter::Filme,
            Filter::Serien,
            Filter::Geplant,
            Filter::AmSchauen,
            Filter::Gesehen,
        ]
    }
}

/// Prozent-Kodierung eines Suchbegriffs für die URL.
pub fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

const API: &str = "https://api.themoviedb.org/3";
const IMG: &str = "https://image.tmdb.org/t/p";

/// Such-URL (Filme + Serien gemeinsam über search/multi).
pub fn search_url(api_key: &str, query: &str, lang: &str) -> String {
    format!(
        "{API}/search/multi?api_key={key}&query={q}&language={lang}&include_adult=false&page=1",
        key = api_key,
        q = encode_query(query),
        lang = lang,
    )
}

/// Detail-URL für einen Titel.
pub fn details_url(api_key: &str, kind: Kind, id: u64, lang: &str) -> String {
    format!("{API}/{}/{}?api_key={}&language={}", kind.path(), id, api_key, lang)
}

/// Poster-URL in gewünschter Grösse (z. B. "w342").
pub fn image_url(size: &str, path: &str) -> String {
    format!("{IMG}/{size}{path}")
}

/// Jahr aus einem TMDB-Datum "JJJJ-MM-TT".
pub fn year_from_date(d: &str) -> Option<u32> {
    if d.len() >= 4 {
        d[..4].parse::<u32>().ok()
    } else {
        None
    }
}

/// Passt ein Eintrag zum Filter?
pub fn matches(e: &Entry, f: Filter) -> bool {
    match f {
        Filter::Alle => true,
        Filter::Filme => e.kind == Kind::Movie,
        Filter::Serien => e.kind == Kind::Tv,
        Filter::Geplant => e.status == Status::Geplant,
        Filter::AmSchauen => e.status == Status::AmSchauen,
        Filter::Gesehen => e.status == Status::Gesehen,
    }
}

/// Indizes der passenden Einträge, alphabetisch nach Titel sortiert.
pub fn filter_sort(entries: &[Entry], f: Filter, search: &str) -> Vec<usize> {
    let s = search.to_lowercase();
    let mut idx: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| matches(e, f) && (s.is_empty() || e.title.to_lowercase().contains(&s)))
        .map(|(i, _)| i)
        .collect();
    idx.sort_by(|&a, &b| {
        entries[a]
            .title
            .to_lowercase()
            .cmp(&entries[b].title.to_lowercase())
    });
    idx
}
