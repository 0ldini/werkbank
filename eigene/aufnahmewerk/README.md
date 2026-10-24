# Aufnahmewerk

Bildschirm aufnehmen für die Werkbank. Nimmt Vollbild, ein einzelnes Fenster
oder einen frei gewählten Bereich als MP4 (H.264) auf – mit System-Ton,
Webcam-Overlay, Mauszeiger-/Klick-Hervorhebung, Countdown und Global-Hotkey.

Dazu **Bildschirmfotos** (ab 1.3.0): ganzer Bildschirm, Bereich oder aktives
Fenster, danach bearbeiten (Pfeil, Rechteck, Stift, Text, Leuchtstift,
Schwärzen, Verpixeln, Zuschneiden) und als PNG/JPG speichern oder kopieren.

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

**Bildschirmfoto (1.3.0):** oben zwischen «Video» und «Bildschirmfoto» umschalten.

- **Modus**: ganzer Bildschirm (Monitor wählbar), **Bereich** (randloses
  Vollbild mit eingefrorenem Bild, Rahmen mit der Maus aufziehen, `Esc` bricht
  ab) oder **aktives Fenster** (das Fenster, das vorne ist, sobald Aufnahmewerk
  ausgeblendet ist – ohne den unsichtbaren Fensterrand).
- **Verzögerung** Aus/3/5/10 s; Aufnahmewerk blendet sich währenddessen aus.
- **Tastenkürzel** `Strg + Umschalt + F` (wie `Strg + Umschalt + R` für Video).
- **Editor** (eigenes Fenster, mehrere gleichzeitig möglich): Pfeil, Rechteck,
  Stift, Text, Markieren (Leuchtstift), Schwärzen, Verpixeln, Zuschneiden;
  Farbe (Farbfelder + eigene Farbe), Strichstärke bzw. Schriftgrösse;
  Rückgängig/Wiederholen (`Strg + Z` / `Strg + Y`).
  Schwärzen und Verpixeln ersetzen die Pixel im gespeicherten Bild – darunter
  ist nichts mehr vorhanden (Verpixeln mit groben Blöcken von mind. 10 px).
- **Speichern** als PNG oder JPG in den Zielordner, Name mit Datum/Uhrzeit
  (`Bildschirmfoto 2026-10-18 14.22.05.png`, bei Gleichstand ` (2)`);
  `Strg + S` nimmt das gewählte Format. **Kopieren** in die Zwischenablage
  (`Strg + C`), **Ordner öffnen**, «Im Ordner zeigen» in der Liste.
- Fragt beim Schliessen nach, wenn das Bild weder gespeichert noch kopiert ist.

**Bildschirmfoto 1.5.0:**

- Neuer Modus **Fenster**: ein Fenster aus der Liste wählen (dieselbe wie
  bei Video) – es wird direkt über Windows.Graphics.Capture fotografiert, auch
  wenn es verdeckt ist; minimierte Fenster werden vorher wiederhergestellt,
  der unsichtbare Fensterrand wird abgeschnitten.
- Eigener **Ordner für Bildschirmfotos**, Standard `Bilder\Bildschirmfotos`
  (getrennt vom Videoordner), wählbar in der Kopfleiste oder in der Karte.
- **Gleich in die Zwischenablage kopieren** nach dem Foto (Schalter).
- Tastenkürzel `Strg + Umschalt + F` steht jetzt auch in den Einstellungen.
- Dateiname wie `Bildschirmfoto 2026-10-24 23.30.12.png`.

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
`windows-capture` (Aufnahme + H.264-MP4 über Media Foundation, auch das
Einzelbild fürs Bildschirmfoto), `nokhwa` (Webcam), `windows` (Win32-Helfer für
Hotkey, Mausposition, Fenstergrösse, Monitore, aktives Fenster), `image`
(PNG/JPG schreiben), `ab_glyph` (Text ins Bild rastern, mit der eingebauten
egui-Schrift), `arboard` (Zwischenablage).

## Aufbau

- `src/model.rs` – reine Datenlogik: Einstellungen, Bitratenwahl,
  Zeitstempel-Dateiname, Timecode, Bereich-Clamp. **Eigenständig getestet.**
- `src/frame.rs` – Bildoperationen auf BGRA: Zuschnitt, Webcam-Komposition,
  Skalierung, Klick-Markierung, Rahmen. **Eigenständig getestet.**
- `src/schnapp.rs` – reine Bildschirmfoto-Logik auf RGBA-Puffern:
  Rechteck-Normalisierung, Zuschneiden, Schwärzen, Verpixeln, Pfeil/Rechteck/
  Stift/Leuchtstift zeichnen, Text-Deckung einmischen, Dateinamen,
  Rückgängig/Wiederholen, Fenster→Monitor-Zuordnung. **Eigenständig getestet:**
  `rustc --edition 2021 --test src/schnapp.rs -o schnapp-test && ./schnapp-test`
- `src/shot.rs` – Bildschirmfoto: Einzelbild über `windows-capture`, eigene
  Fenster kurz ausblenden, Monitor-Rechtecke und aktives Fenster (Win32/DWM),
  Speichern, Zwischenablage, Explorer.
- `src/editor.rs` – das Editor-Fenster (egui-Viewport) mit Werkzeugen.
- `src/record.rs` – Aufnahme-Motor: Verdrahtung mit `windows-capture`
  (Monitor/Fenster, Cursor, kein Rahmen, System-Ton, H.264-MP4), Bild-Pipeline,
  Webcam-Thread (nokhwa), Fenstergrösse über Win32.
- `src/app.rs` – die egui-Oberfläche (Modi, Ton, Webcam, Countdown, Hotkey,
  Aufnahmeknopf, letzte Aufnahmen).
- `src/main.rs` – Fenster und Start.

## Hinweis zum Stand

`model.rs`, `frame.rs` und `schnapp.rs` (die fehleranfällige Bild- und
Rechenlogik) sind hier mit `rustc` gegen echte Fälle geprüft. `shot.rs` und
`editor.rs` sind wie `record.rs` ungebaut (siehe unten). `record.rs` und `app.rs` sind sorgfältig
gegen die dokumentierte API von egui 0.29 und windows-capture 2.0.1 geschrieben,
konnten in meiner Umgebung aber nicht gebaut werden (kein Windows, kein Zugriff
auf crates.io). Wenn `cargo` beim ersten Bauen meckert – z. B. ein Methodenname
in windows-capture weicht in deiner Version ab – schick mir die Meldung, dann
passe ich es punktgenau an.
