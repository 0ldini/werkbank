# Codewerk

QR-Codes erstellen – Teil der Werkbank-Suite. Reines Rust mit **egui/eframe**
(wie die ArtCraft-Programme): läuft nativ auf Windows/Linux/macOS **und** im
Browser (WebAssembly). Mit einer kleinen Kommandozeile im selben Stil.

Inhalte: Text/Link, WLAN-Zugang, E-Mail, Kontakt (vCard) und **Schweizer
QR-Rechnung** (Swiss QR Bill 2.0 mit Schweizer Kreuz). Farben, Grösse, Rand
und Fehlerkorrektur wählbar. Export als **PNG** und **SVG**.

## Bauen und starten

Voraussetzung: Rust (`https://rustup.rs`).

### Nativ (Fenster-App)

    cargo run --release

Die fertige Datei liegt danach unter `target/release/codewerk` (bzw.
`codewerk.exe` auf Windows).

### Windows-.exe von Linux aus (Cross-Build)

    rustup target add x86_64-pc-windows-gnu
    sudo apt install mingw-w64        # Linker
    cargo build --release --target x86_64-pc-windows-gnu

### Browser (WebAssembly)

    rustup target add wasm32-unknown-unknown
    cargo install trunk
    trunk build --release

Danach liegt im Ordner `dist/` eine statische Seite (HTML + .wasm + .js), die
sich auf jedem Webserver ablegen lässt – genau wie die Web-Builds der
ArtCraft-Apps. Zum lokalen Testen: `trunk serve`.

### Kommandozeile

    codewerk --text "https://oldini.ch" --out qr.png
    codewerk --text "WLAN" --out qr.svg --ec H --fg "#1f6feb"

`--help` zeigt alle Optionen. Ohne `--`-Optionen startet die grafische
Oberfläche.

## Aufbau

- `src/qr.rs`   – Kern: Inhalte → QR-Nutzlast, Erzeugung als RGBA/PNG/SVG
  (plattformunabhängig, von Oberfläche und Kommandozeile genutzt).
- `src/app.rs`  – egui-Oberfläche.
- `src/save.rs` – Speichern (nativ per Dialog, im Browser als Download).
- `src/main.rs` – Einstieg: nativ (GUI/Kommandozeile) und Browser (WASM).

## Noch offen (nächste Schritte)

- Schweizer QR-Rechnung: ganzer Zahlteil als A6-PDF (zurzeit nur der QR-Code
  mit Schweizer Kreuz).
- Runde Punkte und eigenes Logo in der Mitte (wie in der Oldinios Toolbox).
- Einbindung in den Werkbank-Launcher (Metadaten, Update-Mechanismus) –
  richtet sich nach dem Launcher.
