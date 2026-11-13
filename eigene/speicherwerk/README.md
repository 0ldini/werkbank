# Speicherwerk

Speicherplatz-Analyse für die Werkbank. Wählt einen Ordner, durchsucht ihn und
zeigt, was den Platz belegt – als Grössenbaum mit Balken und Prozenten, dazu die
grössten Dateien und die grössten Dateitypen. Im Reiter «Doppelte Dateien» findet es
Dateien mit gleichem Inhalt und verschiebt überzählige Kopien in den Papierkorb.

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

## Doppelte Dateien

Oben auf den Reiter **«Doppelte Dateien»** wechseln.

1. **«Ordner wählen»** – die Suche startet gleich. Rechts unter **«Suche»**:
   **«mit Unterordnern»** (Standard: an) und **«Mindestgrösse»** (Standard: «alle Dateien
   (ausser leere)»). Geänderte Einstellungen gelten ab **«Neu suchen»** (unten rechts).
2. Unten läuft der Fortschritt mit (Sammeln → Anfang und Ende vergleichen → Inhalte
   vergleichen); **«Abbrechen»** stoppt jederzeit. Das Fenster bleibt bedienbar.
3. Die Liste zeigt je Gruppe «n gleiche Dateien zu je …» und den freigebbaren Platz,
   sortiert nach freigebbarem Platz. Je Gruppe ist eine Datei **BEHALTEN**, die übrigen sind
   für den **PAPIERKORB** markiert.
4. Rechts unter **«Behalten»** die Regel wählen: **«kürzester Pfad behalten»** (Standard),
   **«älteste behalten»** oder **«neueste behalten»**. Mit den Häkchen lässt sich die Wahl je
   Datei ändern; die letzte behaltene Datei einer Gruppe lässt sich nicht markieren.
   «Regel neu anwenden» setzt Änderungen zurück, «Nichts markieren» hebt alle Markierungen auf.
5. **«In den Papierkorb …»** (unten rechts) zeigt eine Rückfrage mit Anzahl und Grösse.
   Nach **«In den Papierkorb»** wird jede Datei noch einmal geprüft und erst dann verschoben;
   danach zeigt «Ergebnis», was nicht verschoben wurde und warum.
6. **«CSV speichern …»** (oben rechts) speichert die Liste als
   `Gruppe;Bytes;SHA-256;Aktion;Pfad`.

Wie verglichen wird:

- Doppelt heisst **gleicher Inhalt**, egal wie die Dateien heissen. Zuerst nach Grösse, dann
  Anfang und Ende (je 16 KB), zuletzt der ganze Inhalt mit **SHA-256**. Gleich grosse
  Dateien mit anderem Inhalt werden so sicher getrennt.
- **Leere Dateien** gelten nie als doppelt. **Symbolische Verknüpfungen** (auch zu Ordnern)
  werden nicht verfolgt. Höchstens 64 Ordnerebenen und 3 Millionen Dateien; Ordner ohne
  Zugriff werden übersprungen und gezählt («Ordner ohne Zugriff»).
- **Harte Links** (derselbe Dateieintrag unter mehreren Namen, gleiche Datei-ID) sind keine
  Doppel und belegen keinen zusätzlichen Platz: sie erscheinen als ein Eintrag mit
  «+n harte Links». Wird so ein Eintrag verschoben, kommen alle seine Namen mit.
- **Nie endgültig löschen**: nur in den Papierkorb (Crate `trash`). Unmittelbar davor wird
  geprüft, dass die Datei noch da ist, dieselbe Grösse und denselben SHA-256 hat, nicht
  inzwischen derselbe Dateieintrag wie die behaltene ist – und dass eine behaltene Datei der
  Gruppe noch da ist und denselben Inhalt hat. Sonst wird übersprungen.
- Der Platz wird erst frei, wenn der Papierkorb geleert wird.

## Bauen und starten

```
cargo run --release
```

Beim ersten Mal lädt Cargo die Abhängigkeiten (egui, eframe, rfd) aus dem
Internet (dazu `trash` für den Papierkorb). Danach:

```
cargo build --release
```

Die fertige Datei liegt unter `target/release/speicherwerk` (bzw.
`speicherwerk.exe` unter Windows).

## Aufbau

- `src/scan.rs` – Kern: rekursiver Scan, Grössen-Aggregation, Auswertungen,
  CSV, Formatierung. Nur Standardbibliothek, ohne Oberfläche – darum gut testbar.
- `src/doppelt.rs` – Kern für «Doppelte Dateien»: sammeln, gruppieren, SHA-256, Behalten-Regel,
  Prüfung vor dem Verschieben, CSV. Nur Standardbibliothek (unter Windows `windows-sys` für die
  Datei-ID), mit Tests: `rustc --edition 2021 --test src/doppelt.rs -o /tmp/d && /tmp/d`.
- `src/doppelt_ui.rs` – Oberfläche dazu (virtualisierte Liste, Rückfrage, Ergebnis).
- `src/doppelt_e2e.rs` – Ende-zu-Ende-Test mit echtem Papierkorb (`cargo test`, in CI unter
  Linux und Windows: `.github/workflows/speicherwerk-test.yml`).
- `src/app.rs` – die egui-Oberfläche (Reiter, Baum, Seitenpanel, Thread-Steuerung).
- `src/main.rs` – Fenster und Start.

## Hinweis

Der Scan-Kern (`scan.rs`) ist eigenständig getestet. Die Oberfläche ist gegen
egui 0.29 geschrieben. Falls `cargo` beim Bauen einen Fehler meldet, schick mir
die Meldung – dann passe ich es an.
