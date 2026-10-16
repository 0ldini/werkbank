# Speicherwerk

Speicherplatz-Analyse für die Werkbank. Wählt einen Ordner, durchsucht ihn und
zeigt, was den Platz belegt – als Grössenbaum mit Balken und Prozenten, dazu die
grössten Dateien und die grössten Dateitypen.

Rust + egui, passend zu den übrigen Werkzeugen (Codewerk, Umbenennwerk).

## Funktionen

- **Grössenbaum**: Ordner und Dateien nach Grösse sortiert, mit Balken und
  Prozentanteil. Ordner lassen sich auf- und zuklappen.
- **Grösste Dateien**: die 15 grössten Einzeldateien im gewählten Ordner.
- **Dateitypen**: Speicherverbrauch je Endung (.mp4, .psd, …), absteigend.
- **Hintergrund-Scan**: das Durchsuchen läuft in einem eigenen Thread, das
  Fenster bleibt bedienbar. Fortschritt (Dateien/Ordner) unten, mit Abbrechen.
- **CSV-Export**: der ganze Baum als `Pfad;Bytes;Typ` zum Weiterverarbeiten.
- Symbolischen Verknüpfungen wird nicht gefolgt (keine Endlosschleifen).
- Grössen in 1024er-Einheiten, Schweizer Schreibweise (Komma, Tausender-Apostroph).

## Bauen und starten

```
cargo run --release
```

Beim ersten Mal lädt Cargo die Abhängigkeiten (egui, eframe, rfd) aus dem
Internet. Danach:

```
cargo build --release
```

Die fertige Datei liegt unter `target/release/speicherwerk` (bzw.
`speicherwerk.exe` unter Windows).

## Aufbau

- `src/scan.rs` – Kern: rekursiver Scan, Grössen-Aggregation, Auswertungen,
  CSV, Formatierung. Nur Standardbibliothek, ohne Oberfläche – darum gut testbar.
- `src/app.rs` – die egui-Oberfläche (Baum, Seitenpanel, Thread-Steuerung).
- `src/main.rs` – Fenster und Start.

## Hinweis

Der Scan-Kern (`scan.rs`) ist eigenständig getestet. Die Oberfläche ist gegen
egui 0.29 geschrieben. Falls `cargo` beim Bauen einen Fehler meldet, schick mir
die Meldung – dann passe ich es an.
