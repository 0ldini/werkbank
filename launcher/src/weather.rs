//! Wetter von Open-Meteo (open-meteo.com): kostenlos, ohne Schlüssel, ohne Konto.
//! Abgefragt werden nur Koordinaten des eingestellten Ortes, keine Daten über den Benutzer.

use crate::core::{self, Place};
use anyhow::{anyhow, Result};
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Sky {
    Sun,
    Moon,
    Partly,
    Cloud,
    Fog,
    Rain,
    Snow,
    Thunder,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Day {
    /// Kurzname des Wochentags («Mo»).
    pub weekday: String,
    pub sky: Sky,
    pub max: f32,
    pub min: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Weather {
    pub place: String,
    pub temp: f32,
    pub text: String,
    pub sky: Sky,
    /// WMO-Wettercode (für die Beschreibung in der gewählten Sprache).
    #[serde(default)]
    pub code: Option<i64>,
    pub wind: f32,
    pub today_max: f32,
    pub today_min: f32,
    pub rain_prob: f32,
    /// Die nächsten Tage (ohne heute).
    pub days: Vec<Day>,
    /// Abrufzeit (Unix-Sekunden).
    pub fetched: u64,
}

/// WMO-Wettercode → Beschreibung und Symbol.
pub fn describe(code: i64, day: bool) -> (&'static str, Sky) {
    let clear = if day { Sky::Sun } else { Sky::Moon };
    match code {
        0 => ("Klar", clear),
        1 => ("Überwiegend klar", clear),
        2 => ("Teilweise bewölkt", Sky::Partly),
        3 => ("Bedeckt", Sky::Cloud),
        45 | 48 => ("Nebel", Sky::Fog),
        51 | 53 | 55 => ("Nieselregen", Sky::Rain),
        56 | 57 => ("Gefrierender Niesel", Sky::Rain),
        61 => ("Leichter Regen", Sky::Rain),
        63 => ("Regen", Sky::Rain),
        65 => ("Starker Regen", Sky::Rain),
        66 | 67 => ("Gefrierender Regen", Sky::Rain),
        71 => ("Leichter Schneefall", Sky::Snow),
        73 => ("Schneefall", Sky::Snow),
        75 => ("Starker Schneefall", Sky::Snow),
        77 => ("Schneegriesel", Sky::Snow),
        80 | 81 => ("Regenschauer", Sky::Rain),
        82 => ("Heftige Regenschauer", Sky::Rain),
        85 | 86 => ("Schneeschauer", Sky::Snow),
        95 => ("Gewitter", Sky::Thunder),
        96 | 99 => ("Gewitter mit Hagel", Sky::Thunder),
        _ => ("Wetter unbekannt", Sky::Cloud),
    }
}

pub fn weekday_short(date: &str) -> String {
    use chrono::Datelike;
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"][d.weekday().num_days_from_monday() as usize].to_string())
        .unwrap_or_default()
}

fn f(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

/// Liest die Antwort von api.open-meteo.com/v1/forecast.
pub fn parse(place: &str, body: &str, now: u64) -> Result<Weather> {
    let v: Value = serde_json::from_str(body)?;
    let cur = &v["current"];
    let daily = &v["daily"];
    let day = cur["is_day"].as_i64().unwrap_or(1) == 1;
    let (text, sky) = describe(cur["weather_code"].as_i64().unwrap_or(-1), day);
    let arr = |k: &str| daily[k].as_array().cloned().unwrap_or_default();
    let (times, codes, maxs, mins, rain) = (
        arr("time"),
        arr("weather_code"),
        arr("temperature_2m_max"),
        arr("temperature_2m_min"),
        arr("precipitation_probability_max"),
    );
    let mut days = Vec::new();
    for i in 1..times.len().min(6) {
        days.push(Day {
            weekday: weekday_short(times[i].as_str().unwrap_or("")),
            sky: describe(codes.get(i).and_then(Value::as_i64).unwrap_or(-1), true).1,
            max: maxs.get(i).and_then(f).unwrap_or(f32::NAN),
            min: mins.get(i).and_then(f).unwrap_or(f32::NAN),
        });
    }
    Ok(Weather {
        place: place.to_string(),
        temp: f(&cur["temperature_2m"]).ok_or_else(|| anyhow!("keine Temperatur in der Antwort"))?,
        text: text.to_string(),
        sky,
        code: cur["weather_code"].as_i64(),
        wind: f(&cur["wind_speed_10m"]).unwrap_or(0.0),
        today_max: maxs.first().and_then(f).unwrap_or(f32::NAN),
        today_min: mins.first().and_then(f).unwrap_or(f32::NAN),
        rain_prob: rain.first().and_then(f).unwrap_or(0.0),
        days,
        fetched: now,
    })
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn fetch(place: &Place) -> Result<Weather> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,weather_code,wind_speed_10m,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max\
         &timezone=auto&forecast_days=6",
        place.lat, place.lon
    );
    let body = core::http_text(&url, 1 << 20)?;
    let w = parse(&place.name, &body, now_secs())?;
    let _ = std::fs::write(cache_path(), serde_json::to_string(&w)?);
    Ok(w)
}

fn cache_path() -> std::path::PathBuf {
    core::data_dir().join("wetter.json")
}

/// Zuletzt abgerufenes Wetter (für den Start ohne Netz).
pub fn cached() -> Option<Weather> {
    serde_json::from_str(&std::fs::read_to_string(cache_path()).ok()?).ok()
}

/// Ortssuche über geocoding-api.open-meteo.com.
pub fn search_place(name: &str) -> Result<Vec<Place>> {
    let q: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' { c.to_string() } else { "%20".into() }).collect();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let body = core::http_text(
        &format!("https://geocoding-api.open-meteo.com/v1/search?name={q}&count=6&language=de&format=json"),
        1 << 20,
    )?;
    let v: Value = serde_json::from_str(&body)?;
    let mut out = Vec::new();
    for r in v["results"].as_array().cloned().unwrap_or_default() {
        let (Some(lat), Some(lon)) = (r["latitude"].as_f64(), r["longitude"].as_f64()) else { continue };
        let name = r["name"].as_str().unwrap_or("").to_string();
        let region = [r["admin1"].as_str(), r["country"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(", ");
        out.push(Place { name, region: Some(region), lat, lon });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Symbole (gezeichnet, damit keine Schrift fehlen kann)

const CLOUD: Color32 = Color32::from_rgb(0xD6, 0xD9, 0xE0);
const RAIN: Color32 = Color32::from_rgb(0x7F, 0xB4, 0xE6);

fn cloud(p: &Painter, c: Pos2, s: f32, fill: Color32) {
    p.circle_filled(Pos2::new(c.x - 0.17 * s, c.y + 0.04 * s), 0.15 * s, fill);
    p.circle_filled(Pos2::new(c.x + 0.02 * s, c.y - 0.06 * s), 0.21 * s, fill);
    p.circle_filled(Pos2::new(c.x + 0.21 * s, c.y + 0.05 * s), 0.13 * s, fill);
    p.rect_filled(Rect::from_min_max(Pos2::new(c.x - 0.17 * s, c.y + 0.02 * s), Pos2::new(c.x + 0.21 * s, c.y + 0.19 * s)), 0.0, fill);
}

fn sun(p: &Painter, c: Pos2, s: f32, color: Color32) {
    let st = Stroke::new((0.06 * s).max(1.5), color);
    p.circle_stroke(c, 0.17 * s, st);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (dx, dy) = (a.cos(), a.sin());
        p.line_segment([Pos2::new(c.x + dx * 0.27 * s, c.y + dy * 0.27 * s), Pos2::new(c.x + dx * 0.36 * s, c.y + dy * 0.36 * s)], st);
    }
}

/// Zeichnet ein Wettersymbol in ein Quadrat der Kantenlänge `s` um `c`.
pub fn draw(p: &Painter, c: Pos2, s: f32, sky: Sky, accent: Color32, bg: Color32) {
    match sky {
        Sky::Sun => sun(p, c, s, accent),
        Sky::Moon => {
            p.circle_filled(c, 0.24 * s, accent);
            p.circle_filled(Pos2::new(c.x + 0.12 * s, c.y - 0.09 * s), 0.21 * s, bg);
        }
        Sky::Partly => {
            sun(p, Pos2::new(c.x - 0.12 * s, c.y - 0.12 * s), 0.8 * s, accent);
            cloud(p, Pos2::new(c.x + 0.06 * s, c.y + 0.08 * s), 0.85 * s, CLOUD);
        }
        Sky::Cloud => cloud(p, c, s, CLOUD),
        Sky::Fog => {
            let st = Stroke::new((0.06 * s).max(1.5), CLOUD);
            for (i, w) in [0.34, 0.26, 0.34].iter().enumerate() {
                let y = c.y - 0.16 * s + i as f32 * 0.16 * s;
                p.line_segment([Pos2::new(c.x - w * s, y), Pos2::new(c.x + w * s, y)], st);
            }
        }
        Sky::Rain | Sky::Snow | Sky::Thunder => {
            let top = Pos2::new(c.x, c.y - 0.1 * s);
            cloud(p, top, s, CLOUD);
            match sky {
                Sky::Rain => {
                    let st = Stroke::new((0.05 * s).max(1.5), RAIN);
                    for dx in [-0.16, 0.0, 0.16] {
                        let x = c.x + dx * s;
                        p.line_segment([Pos2::new(x, c.y + 0.16 * s), Pos2::new(x - 0.05 * s, c.y + 0.3 * s)], st);
                    }
                }
                Sky::Snow => {
                    for dx in [-0.16, 0.0, 0.16] {
                        p.circle_filled(Pos2::new(c.x + dx * s, c.y + 0.24 * s), 0.035 * s, Color32::WHITE);
                    }
                }
                _ => {
                    let pts = vec![
                        Pos2::new(c.x + 0.02 * s, c.y + 0.12 * s),
                        Pos2::new(c.x - 0.08 * s, c.y + 0.28 * s),
                        Pos2::new(c.x + 0.02 * s, c.y + 0.27 * s),
                        Pos2::new(c.x - 0.04 * s, c.y + 0.42 * s),
                    ];
                    p.add(Shape::line(pts, Stroke::new((0.05 * s).max(1.5), accent)));
                }
            }
        }
    }
}

/// Grad-Anzeige ohne «-0».
pub fn deg(v: f32) -> String {
    if v.is_nan() {
        return "–".into();
    }
    let r = v.round();
    format!("{}°", if r == 0.0 { 0.0 } else { r })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antwort_lesen() {
        let body = r#"{"current":{"temperature_2m":11.3,"weather_code":2,"wind_speed_10m":8.1,"is_day":1},
            "daily":{"time":["2026-10-18","2026-10-19","2026-10-20"],"weather_code":[2,0,61],
            "temperature_2m_max":[14.2,15.0,9.1],"temperature_2m_min":[5.6,4.0,3.2],
            "precipitation_probability_max":[20,5,80]}}"#;
        let w = parse("Sarnen", body, 1).unwrap();
        assert_eq!(w.text, "Teilweise bewölkt");
        assert_eq!(deg(w.temp), "11°");
        assert_eq!(w.days.len(), 2);
        assert_eq!(w.days[0].weekday, "Mo");
        assert_eq!(w.days[1].sky, Sky::Rain);
        assert_eq!(deg(-0.3), "0°");
    }
}
