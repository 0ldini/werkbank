//! TMDB-Anbindung: Suche und Details über die öffentliche API (ureq + JSON).
//! Gibt fertige `Entry`-Werte zurück. Netzzugriff – läuft in der App in einem
//! Hintergrund-Thread.

use serde_json::Value;

use crate::model::{self, Entry, Kind, Status};

/// Sucht Filme und Serien gemeinsam (search/multi).
pub fn search(api_key: &str, query: &str, lang: &str) -> Result<Vec<Entry>, String> {
    if api_key.trim().is_empty() {
        return Err("Kein TMDB-Schlüssel hinterlegt (Einstellungen).".into());
    }
    let url = model::search_url(api_key, query, lang);
    let body = get_text(&url)?;
    let v: Value = serde_json::from_str(&body).map_err(|e| format!("Antwort unlesbar: {e}"))?;
    if let Some(msg) = v.get("status_message").and_then(|m| m.as_str()) {
        return Err(format!("TMDB: {msg}"));
    }
    let results = v.get("results").and_then(|r| r.as_array()).ok_or("Keine Ergebnisse.")?;
    let mut out = Vec::new();
    for r in results {
        let mt = r.get("media_type").and_then(|m| m.as_str()).unwrap_or("");
        let kind = match mt {
            "movie" => Kind::Movie,
            "tv" => Kind::Tv,
            _ => continue,
        };
        out.push(entry_from_json(r, kind));
    }
    Ok(out)
}

/// Holt Details (Laufzeit bei Film, Staffelzahl bei Serie) und ergänzt `e`.
pub fn enrich(api_key: &str, lang: &str, e: &mut Entry) -> Result<(), String> {
    let url = model::details_url(api_key, e.kind, e.id, lang);
    let body = get_text(&url)?;
    let v: Value = serde_json::from_str(&body).map_err(|e| format!("Antwort unlesbar: {e}"))?;
    match e.kind {
        Kind::Movie => {
            e.runtime = v.get("runtime").and_then(|r| r.as_u64()).map(|n| n as u32);
        }
        Kind::Tv => {
            e.seasons = v.get("number_of_seasons").and_then(|r| r.as_u64()).map(|n| n as u32);
        }
    }
    Ok(())
}

fn entry_from_json(r: &Value, kind: Kind) -> Entry {
    let id = r.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
    let (title_key, date_key) = match kind {
        Kind::Movie => ("title", "release_date"),
        Kind::Tv => ("name", "first_air_date"),
    };
    let title = r.get(title_key).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let date = r.get(date_key).and_then(|x| x.as_str()).unwrap_or("");
    let rating = r.get("vote_average").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    let poster = r
        .get("poster_path")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let overview = r.get("overview").and_then(|x| x.as_str()).unwrap_or("").to_string();
    Entry {
        id,
        kind,
        title,
        year: model::year_from_date(date),
        rating,
        poster,
        overview,
        status: Status::Geplant,
        my_rating: 0,
        seasons: None,
        runtime: None,
    }
}

fn get_text(url: &str) -> Result<String, String> {
    match ureq::get(url).call() {
        Ok(resp) => resp.into_string().map_err(|e| format!("Lesefehler: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let msg = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {}", first_line(&msg)))
        }
        Err(e) => Err(format!("Netzwerkfehler: {e}")),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(160).collect()
}
