# Organiwerk

Organigramme und Diagramme zeichnen, im Stil von draw.io, aber klein und schnell. Reines Rust
mit **egui/eframe**, läuft auf Windows, Linux und im Browser. Kein Server, keine Anmeldung;
die Dateien liegen bei dir.

## Was es kann

- **Karten und Formen:** Person/Stelle (Name und Funktion), abgerundetes Rechteck, Rechteck,
  Ellipse, Raute, freier Text und **Gruppen** (Rahmen für Abteilungen; wer die Gruppe
  verschiebt, nimmt den Inhalt mit).
- **Verbinder**, die an den Karten kleben und beim Verschieben mitgehen: rechtwinklig oder
  gerade, mit oder ohne Pfeilspitze, mit Beschriftung.
- **Organigramm anordnen:** ordnet verbundene Karten automatisch als Baum an (Vorgesetzte
  mittig über ihren Untergebenen). Mit Auswahl nur den gewählten Zweig.
- **Schnell aufbauen:** Karte wählen, «Untergeordnet hinzufügen» (Strg+Enter) oder «Gleiche
  Ebene hinzufügen» (Strg+Umschalt+Enter): neue Karte samt Verbinder.
- Zoomen und Verschieben, Raster mit Einrasten, Mehrfachauswahl (Rechteck aufziehen),
  Ausrichten und gleichmässig Verteilen, Farben, Duplizieren, **Rückgängig/Wiederholen**.
- **Speichern/Öffnen** als `.organi`-Datei (Text, JSON), **Export als SVG**. Das
  Dokument wird zusätzlich automatisch im Programm (bzw. im Browser) zwischengespeichert.

## Bedienung

| Aktion | So |
|---|---|
| Neue Karte | Doppelklick auf die Fläche, oder oben bei «Einfügen» |
| Text ändern | Doppelklick auf die Karte; rechts im Feld «Text» (Name, dann Zeilenumbruch, dann Funktion) |
| Verbinden | Werkzeug «Verbinden», von Karte zu Karte ziehen (oder nacheinander antippen) |
| Mehrere wählen | Rechteck aufziehen, oder Umschalt+Klick |
| Fläche verschieben | Mausrad / Zweifingerwischen, Leertaste+Ziehen oder rechte Maustaste |
| Zoom | Strg+Mausrad oder Pinch, oder «+»/«−» |
| Rückgängig / Wiederholen | Strg+Z / Strg+Y |
| Duplizieren, Löschen | Strg+D, Entf |
| Alles wählen, Speichern | Strg+A, Strg+S |
| Pfeiltasten | Auswahl verschieben (mit Umschalt: 10 Einheiten) |

## Noch nicht enthalten

PNG-Export (SVG geht; ein SVG lässt sich in Linienwerk, im Browser oder in Office öffnen
und weiterverwenden), Import/Export von draw.io-Dateien, mehrere Seiten, Ebenen,
Schriftarten und Linienstile, Bilder in Karten, Verbinder an frei wählbaren Seiten,
Android.

## Sprache und Design

Deutsch oder Englisch, hell oder dunkel, wie im Werkbank-Launcher gewählt (`src/sprache.rs`,
`src/thema.rs`); im Browser Deutsch und hell. Die englischen Texte stehen in `src/en.tsv`;
was im Diagramm steht, wird nicht übersetzt (nur der Vorgabetext neuer Formen). Die
Zeichenfläche bleibt auch im dunklen Design hell (wie das Diagramm im SVG).
Prüfen, ob alles übersetzt ist: `rustc --edition 2021 --test src/sprache.rs -o /tmp/sprache && /tmp/sprache`.

## Bauen

Voraussetzung: Rust (`https://rustup.rs`).

    cargo run --release              # Desktop
    trunk build --release            # Browser (rustup target add wasm32-unknown-unknown)

Bei einem Push nach `eigene/organiwerk/**` baut die Aktion «Eigene-Release» alle drei
Versionen und veröffentlicht sie als `organiwerk-v<Version>`.

## Aufbau

`model.rs` enthält alles Rechnerische ohne Oberfläche (Formen, Verbinder-Wege, Treffer,
Baum-Anordnung, Ausrichten, Speichern, SVG) und hat Tests. `json.rs` ist ein kleines
JSON ohne Fremdbibliothek. `app.rs` ist die Oberfläche, `save.rs` Datei-Dialoge bzw.
Download im Browser.
