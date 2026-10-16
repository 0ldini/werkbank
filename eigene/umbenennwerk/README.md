# Umbenennwerk

Dateien stapelweise umbenennen – Teil der Werkbank-Suite. Reines Rust mit
**egui/eframe** (wie die ArtCraft-Programme), läuft nativ auf
Windows/Linux/macOS.

Links die Regeln, rechts die Live-Vorschau **Alt → Neu** mit rot markierten
Namenskonflikten. Umbenennen läuft zweistufig (erst temporäre Namen, dann ans
Ziel), damit sich Namen auch tauschen lassen; danach ist **Rückgängig**
möglich.

Regeln: Namen ersetzen oder Suchen & Ersetzen, fortlaufende Nummerierung
(Start/Schritt/Stellen/Trennzeichen/Position), Präfix/Suffix,
Gross-/Kleinschreibung (Original/GROSS/klein/Titel), Datum einfügen
(Dateidatum, Tokens JJJJ MM TT hh mm ss), Bereinigen (Leerzeichen→Unterstrich,
Sonderzeichen entfernen, Erweiterung klein).

## Bauen und starten

Voraussetzung: Rust (https://rustup.rs).

    cargo run --release

Windows-.exe von Linux aus:

    rustup target add x86_64-pc-windows-gnu
    sudo apt install mingw-w64
    cargo build --release --target x86_64-pc-windows-gnu

## Aufbau

- `src/rename.rs`  – Kern: aus altem Namen + Regeln den neuen bilden
  (plattformunabhängig, mit Tests abgedeckt).
- `src/datefmt.rs` – Unix-Zeit → Datumstext (ohne externe Bibliothek).
- `src/app.rs`     – egui-Oberfläche inkl. Umbenennen (zweistufig) und
  Rückgängig.
- `src/main.rs`    – Einstieg (Fenster-App).

## Noch offen (nächste Schritte)

- Datum in lokaler Zeitzone (zurzeit UTC) und EXIF-Aufnahmedatum.
- Musik-Tags (Titel/Interpret) als Namensbausteine.
- Ordner rekursiv einlesen, Spalten sortieren.
- Einbindung in den Werkbank-Launcher (richtet sich nach dem Launcher).
