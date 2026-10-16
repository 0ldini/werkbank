# Mediathekwerk

Filme & Serien verwalten – Teil der Werkbank. Suche über **TMDB**
(themoviedb.org), lege dir eine eigene Sammlung an, setze je Titel einen Status
(geplant / am Schauen / gesehen) und deine eigene Bewertung.

Rust + egui, passend zu Codewerk, Umbenennwerk, Speicherwerk, Aufnahmewerk,
Wandelwerk und Schutzwerk.

## Funktionen

- **Suche** nach Filmen und Serien gemeinsam (TMDB search/multi), mit Poster,
  Jahr, TMDB-Bewertung und Beschreibung.
- **Eigene Sammlung** als Poster-Raster, filterbar nach Alle / Filme / Serien /
  Geplant / Am Schauen / Gesehen.
- **Detail** je Titel: Status setzen, eigene Sternebewertung, in die Sammlung
  aufnehmen oder wieder entfernen.
- Sammlung und Einstellungen werden **lokal** gespeichert
  (`%APPDATA%\Mediathekwerk\`).

## TMDB-Schlüssel

Du brauchst einen **kostenlosen API-Schlüssel** von themoviedb.org:
Konto anlegen → Einstellungen → API → „API Read Access"/„API Key (v3 auth)".
Den Schlüssel trägst du einmal in Mediathekwerk unter **Einstellungen** ein; er
bleibt nur auf deinem Rechner.

## Bauen und starten

```
cargo run --release
```

Fertige Datei danach unter `target\release\mediathekwerk.exe`.

Abhängigkeiten: `egui`/`eframe` (Oberfläche), `egui_extras` (lädt die Poster
direkt per URL), `ureq` (TMDB-Abfragen), `serde`/`serde_json` (Sammlung
speichern).

## Aufbau

- `src/model.rs` – Datenmodell, TMDB-URL-Bau, Query-Encoding, Filter/Sortierung.
  **Eigenständig getestet.**
- `src/tmdb.rs` – Suche und Details über die TMDB-API.
- `src/store.rs` – Einstellungen und Sammlung als JSON lokal speichern.
- `src/app.rs` – die egui-Oberfläche (Suche, Raster, Detail, Einstellungen).
- `src/main.rs` – Fenster und Start.

## Hinweis zum Stand

Die URL-/Encoding-/Filterlogik (`model.rs`) ist hier mit `rustc` gegen echte
Fälle geprüft. Oberfläche, TMDB-Anbindung und Speicherung sind gegen die
dokumentierten APIs von egui 0.29, egui_extras, ureq und serde geschrieben,
liessen sich in meiner Umgebung aber nicht bauen (kein Zugriff auf crates.io).
Falls `cargo` beim ersten Bauen meckert, schick mir die Meldung.
