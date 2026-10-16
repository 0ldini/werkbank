# Aufnahmewerk

Bildschirm aufnehmen für die Werkbank. Nimmt Vollbild, ein einzelnes Fenster
oder einen frei gewählten Bereich als MP4 (H.264) auf – mit System-Ton,
Webcam-Overlay, Mauszeiger-/Klick-Hervorhebung, Countdown und Global-Hotkey.

Rust + egui, passend zu Codewerk, Umbenennwerk und Speicherwerk. Nur Windows
(nutzt Windows.Graphics.Capture und Media Foundation).

## Funktionen

**Etappe 1 (dieses Paket):**

- **Aufnahmemodus**: ganzer Monitor (mit Monitorwahl), einzelnes Fenster oder
  frei gewählter Bereich (Zuschnitt).
- **Format**: MP4 mit H.264, Qualität Hoch/Mittel/Datensparend, 30 oder 60 fps.
  Die Bitrate wird aus Auflösung, Bildrate und Qualität berechnet.
- **System-Ton** wird mit aufgenommen (an/aus).
- **Webcam-Overlay**: kleines Kamerabild in einer wählbaren Ecke.
- **Mauszeiger & Klicks**: echter Systemzeiger plus hervorgehobene Klicks.
- **Countdown** (3/5 s) und **Global-Hotkey** `Strg + Umschalt + R` zum
  Starten/Stoppen – auch wenn das Fenster nicht im Vordergrund ist.
- Zielordner wählbar, Liste der letzten Aufnahmen.

**Etappe 2 (folgt):**

- **Mikrofon** zusätzlich zum System-Ton, sauber gemischt in dieselbe MP4
  (der heikle Synchron-/Misch-Teil – wird auf echtem Windows getestet statt
  blind geliefert).
- Mehrmonitor-Versatz für die Klick-Hervorhebung auf Zweitschirmen.

## Bauen und starten (Windows)

```
cargo run --release
```

Beim ersten Mal lädt Cargo die Abhängigkeiten. Fertige Datei danach unter
`target\release\aufnahmewerk.exe`.

Abhängigkeiten: `egui`/`eframe` (Oberfläche), `rfd` (Ordnerauswahl),
`windows-capture` (Aufnahme + H.264-MP4 über Media Foundation), `nokhwa`
(Webcam), `windows` (Win32-Helfer für Hotkey, Mausposition, Fenstergrösse).

## Aufbau

- `src/model.rs` – reine Datenlogik: Einstellungen, Bitratenwahl,
  Zeitstempel-Dateiname, Timecode, Bereich-Clamp. **Eigenständig getestet.**
- `src/frame.rs` – Bildoperationen auf BGRA: Zuschnitt, Webcam-Komposition,
  Skalierung, Klick-Markierung, Rahmen. **Eigenständig getestet.**
- `src/record.rs` – Aufnahme-Motor: Verdrahtung mit `windows-capture`
  (Monitor/Fenster, Cursor, kein Rahmen, System-Ton, H.264-MP4), Bild-Pipeline,
  Webcam-Thread (nokhwa), Fenstergrösse über Win32.
- `src/app.rs` – die egui-Oberfläche (Modi, Ton, Webcam, Countdown, Hotkey,
  Aufnahmeknopf, letzte Aufnahmen).
- `src/main.rs` – Fenster und Start.

## Hinweis zum Stand

`model.rs` und `frame.rs` (die fehleranfällige Bild- und Rechenlogik) sind hier
mit `rustc` gegen echte Fälle geprüft. `record.rs` und `app.rs` sind sorgfältig
gegen die dokumentierte API von egui 0.29 und windows-capture 2.0.1 geschrieben,
konnten in meiner Umgebung aber nicht gebaut werden (kein Windows, kein Zugriff
auf crates.io). Wenn `cargo` beim ersten Bauen meckert – z. B. ein Methodenname
in windows-capture weicht in deiner Version ab – schick mir die Meldung, dann
passe ich es punktgenau an.
