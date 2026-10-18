# Wandelwerk

Ein Konverter für Ton, Bild und Video – Teil der Werkbank. Dateien gemischt
hineinwerfen, Zielformat wählen, bei Bild und Video optional das
Seitenverhältnis ändern (1:1, 16:9, 9:16, 4:3, 3:2, 21:9), umwandeln.

Rust + egui, passend zu Codewerk, Umbenennwerk, Speicherwerk und Aufnahmewerk.

## Funktionen

- **Alles in einem**: Ton, Bild und Video in derselben Liste. Der Typ wird je
  Datei an der Endung erkannt (grün = Ton, blau = Bild, violett = Video).
- **Zielformate**
  - Ton: MP3, M4A, WAV, FLAC, OGG, OPUS
  - Bild: JPG, PNG, WebP, BMP, GIF, TIFF
  - Video: MP4, MKV, WebM, MOV, GIF
  - **Nur Ton aus einem Video**: bei Videos (z. B. MKV) die Tonspur als MP3,
    M4A, WAV, FLAC, OGG oder OPUS speichern. Hat der Film mehrere Tonspuren
    (z. B. Deutsch und Englisch), lässt sich die gewünschte wählen.
- **Seitenverhältnis** (Bild/Video): Original, 1:1, 16:9, 9:16, 4:3, 3:2, 21:9 –
  mit Live-Vorschau. **Anpassung je Datei**: Zuschneiden (füllt, schneidet
  Ränder ab), Ränder (ganzes Bild mit Balken) oder Strecken.
- **Qualität**: Hoch / Mittel / Klein (steuert Bitrate bzw. CRF bzw. JPEG-Güte).
- **Stapel**: viele Dateien auf einmal, Fortschritt je Datei; „Für alle
  übernehmen" setzt die Einstellung der gewählten Datei auf die ganze Liste.
- Zielordner frei wählbar, Namenskollisionen werden mit „ (2)" umgangen.

## Motor

- **Bilder** laufen rein in Rust über die `image`-Crate – kein Zusatzprogramm
  nötig.
- **Ton & Video** laufen über **ffmpeg**. Lege `ffmpeg.exe` neben
  `wandelwerk.exe` (oder sorge dafür, dass ffmpeg auf dem PATH liegt). Unten
  links zeigt das Programm, ob ffmpeg gefunden wurde. In der Werkbank kann
  ffmpeg einmal mitgeliefert werden.

## Bauen und starten

```
cargo run --release
```

Fertige Datei danach unter `target/release/wandelwerk` (bzw.
`wandelwerk.exe` unter Windows).

## Aufbau

- `src/model.rs` – reine Logik: Typ-Erkennung, Zielformate,
  Seitenverhältnis-Geometrie, Qualität, Dateiname, **ffmpeg-Argumentbau**.
  **Eigenständig getestet.**
- `src/convert.rs` – Motor: Bilder (image-Crate), Ton/Video (ffmpeg) mit
  Fortschritt; die Fortschritts-Parser sind **eigenständig getestet**.
- `src/app.rs` – die egui-Oberfläche (Liste, Ausgabe, Vorschau, Fortschritt).
- `src/main.rs` – Fenster und Start.

## Hinweis zum Stand

Die Rechen- und Argumentlogik (`model.rs`) und die Fortschritts-Parser sind hier
mit `rustc` gegen echte Fälle geprüft. Oberfläche (`app.rs`) und
Motor-Verdrahtung sind gegen die dokumentierten APIs von egui 0.29 und der
image-Crate 0.25 geschrieben, liessen sich in meiner Umgebung aber nicht bauen
(kein Zugriff auf crates.io). Wenn `cargo` beim ersten Bauen meckert, schick mir
die Meldung – dann passe ich es an.


## ffmpeg eingebaut

Die Werkbank-Ausgabe von Wandelwerk bringt ffmpeg mit: Es steckt in der Programmdatei und wird
beim ersten Ton- oder Video-Auftrag einmalig nach `%LOCALAPPDATA%\Wandelwerk` entpackt. Nichts
muss installiert werden. Liegt eine eigene `ffmpeg.exe` neben dem Programm, hat diese Vorrang.

ffmpeg ist ein eigenständiges Programm des FFmpeg-Projekts (https://ffmpeg.org), hier als
GPL-Build von https://github.com/BtbN/FFmpeg-Builds. Es steht unter der GNU GPL Version 3
(siehe `LICENSE-ffmpeg.txt`); den Quellcode gibt es unter https://ffmpeg.org/download.html und
https://github.com/BtbN/FFmpeg-Builds.
