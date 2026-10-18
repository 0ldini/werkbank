# Sicherwerk

Ordner regelmässig sichern – Teil der Werkbank. Auf einen USB-Stick, eine zweite
Festplatte oder ein NAS (Netzwerkpfad), von Hand, nach Zeitplan oder sobald das
Ziellaufwerk angeschlossen ist.

Rust + egui, passend zu Codewerk, Wandelwerk, Diawerk und den anderen Werken. Läuft unter
Windows und Linux, auf Deutsch und Englisch, hell und dunkel (wie im Werkbank-Launcher gewählt).

## Grundsatz

**In den Quellordnern wird nie etwas verändert, verschoben oder gelöscht.** Sicherwerk liest
sie nur. Geschrieben wird ausschliesslich im Auftragsordner im Ziel – und beim
Wiederherstellen in einem frei gewählten Ordner, ohne dort etwas zu überschreiben.

## Funktionen

- **Sicherungsaufträge** mit Name, einem oder mehreren Quellordnern, Ziel und Ausschlüssen.
  Im Ziel entsteht ein Ordner mit dem Auftragsnamen; darin hat jede Quelle ihren Unterordner.
  Mehrere Aufträge können denselben Stick nutzen.
- **Zwei Arten**
  - **Spiegeln**: kopiert nur Neues und Geändertes (Vergleich nach Grösse und Änderungszeit).
    Dateien, die es in der Quelle nicht mehr gibt, werden im Ziel wahlweise nach
    `_entfernt/<Datum>/` verschoben (Standard), gelöscht oder behalten.
  - **Versionen**: bei jedem Lauf ein datierter Stand (`2026-10-18_14-30-05`) als **Ordner**
    oder **ZIP-Datei**; aufbewahrt werden die letzten N Versionen. In Ordner-Versionen werden
    unveränderte Dateien auf Wunsch als **harte Verknüpfung** übernommen: jede Version ist
    vollständig, belegt aber nur Platz für Geändertes (NTFS, ext4 usw.; auf FAT/exFAT wird
    automatisch kopiert).
- **Ausschlüsse**: ein Muster pro Zeile. Ohne `/` gilt es für Datei- und Ordnernamen überall
  (`*.tmp`, `node_modules`, `Cache`, `~$*`), mit `/` für Pfade ab dem Quellordner
  (`Projekte/*/build`, `Spiele/**/*.log`). `*`, `?` und `**`; Gross-/Kleinschreibung egal.
- **Vorschau «Was würde passieren»**: zeigt neue, geänderte und zu entfernende Dateien bzw.
  die neue Version, die Datenmenge und welche alten Versionen wegfallen – ohne etwas zu
  schreiben. Vor dem ersten Lauf (und nach Änderungen an Quellen, Ziel oder Art) kommt sie
  automatisch.
- **Fortschritt** mit Dateien, Datenmenge, aktueller Datei und **Abbrechen**. Ein
  abgebrochener Versionslauf hinterlässt keine halbe Version; beim Spiegeln sind fertig
  kopierte Dateien vollständig (geschrieben wird über eine Zwischendatei).
- **Protokoll je Lauf**: Zeit, Auslöser (von Hand, Zeitplan, Laufwerk angeschlossen),
  Ergebnis, Anzahl neu/geändert/entfernt/verknüpft/unverändert, Datenmenge, Dauer, Fehler
  mit Pfad und Grund.
- **Prüfen** (Stichprobe): liest N zufällige Dateien aus der Sicherung (bei ZIP mit
  CRC-Kontrolle) und vergleicht sie per SHA-256 mit der Quelle, sofern diese seither
  unverändert ist.
- **Wiederherstellen** einzelner Dateien oder ganzer Ordner aus dem Spiegel, aus einer
  Version (Ordner oder ZIP) oder aus `_entfernt`: Stand wählen, durchblättern oder suchen,
  anhaken, Zielordner wählen. Vorhandenes wird nie überschrieben – gibt es den Namen schon,
  heisst der neue «Name (2)».

## Zeitplan

- **Täglich** oder **wöchentlich** zu einer Uhrzeit, oder **beim Anschliessen des
  Ziellaufwerks** (einmal pro Tag, sobald das Ziel erreichbar ist: beim Programmstart und
  alle 30 Sekunden, solange das Fenster offen ist).
- **Windows**: Reiter «Zeitplan» → **Eintragen** legt eine Aufgabe in der
  Aufgabenplanung an (`schtasks /Create /XML`, Ordner «Sicherwerk»). Sie startet
  `sicherwerk --auftrag "<Name>"` ohne Fenster, auch im Akkubetrieb; war der Computer zur
  geplanten Zeit aus, wird nachgeholt. Für «beim Anschliessen» gibt es eine gemeinsame
  Aufgabe `Sicherwerk\Anschluss`, die alle 15 Minuten und bei der Anmeldung
  `sicherwerk --anschluss` aufruft. Umbenennen, Ändern und Löschen eines Auftrags führen
  die Aufgabe nach.
- **Linux**: keine automatische Einrichtung. Der Reiter zeigt eine fertige
  **crontab-Zeile** und **systemd-Timer** (`~/.config/systemd/user/…`) zum Kopieren samt
  Befehlen.

Läuft ein Auftrag per Zeitplan, öffnet sich kein Fenster; das Ergebnis steht danach im
Protokoll. Ein Auftrag läuft nie doppelt (Sperrdatei; Fenster und Zeitplan teilen sie).

### Befehlszeile

```
sicherwerk                          Fenster öffnen
sicherwerk --auftrag "<Name>"       Auftrag ohne Fenster ausführen
sicherwerk --anschluss              «beim Anschliessen»-Aufträge mit erreichbarem Ziel ausführen
sicherwerk --hilfe
```

Rückgabewert: 0 = erfolgreich, 1 = mit Fehlern/abgebrochen, 2 = fehlgeschlagen. Unter Linux
steht eine Kurzfassung auf der Standardausgabe (cron schickt sie per Mail).

## Daten

Aufträge und Protokolle stehen in **einer JSON-Datei im Benutzerordner**:

- Windows: `%APPDATA%\Sicherwerk\sicherwerk.json`
- Linux: `~/.config/sicherwerk/sicherwerk.json`

Jede Änderung wird unter Sperre geschrieben (Zwischendatei + Umbenennen), die vorige Fassung
bleibt als `sicherwerk.json.bak`. Eine beschädigte Datei wird nie überschrieben, sondern als
`sicherwerk.json.defekt-<Zeit>` beiseitegelegt. Gespeichert werden höchstens 1000 Protokolle
(älteste fallen weg) und je Lauf höchstens 200 Einzelfehler (gezählt werden alle).

## Sicherheitsnetze

- Ziel nicht da (Stick nicht eingesteckt): Lauf bricht mit «Ziel nicht erreichbar» ab;
  Sicherwerk legt nie einen Ersatzordner an.
- Quellordner fehlt oder ist leer (z. B. NAS nicht eingebunden): im Ziel wird nichts davon
  entfernt.
- Ordner/Dateien, die nicht gelesen werden konnten, werden im Ziel ebenfalls nicht entfernt.
- Liegt das Ziel in einer Quelle (oder umgekehrt), wird der Lauf abgelehnt.
- Alte Versionen werden nur nach einem fehlerfreien Lauf aufgeräumt.
- Symbolische Verknüpfungen und Verbindungspunkte (Junctions) werden übersprungen – keine
  Endlosschleifen, nichts ausserhalb der Quelle.
- FAT/exFAT-Sticks: 2 Sekunden Zeitungenauigkeit und die Sommerzeit-Stunde gelten nicht
  als Änderung.

## Bedienung

1. **+ Neuer Auftrag**, Name eingeben.
2. **Quellordner** hinzufügen (wählen oder Pfad eintippen, auch `\\nas\freigabe`).
3. **Ziel** wählen, **Art** wählen, Ausschlüsse anpassen, **Speichern**.
4. **Jetzt sichern** – beim ersten Mal erscheint die Vorschau, dann **Sichern starten**.
5. Optional unter **Zeitplan** einen Zeitplan wählen, speichern und (Windows) **Eintragen**.
6. **Prüfen** ab und zu laufen lassen; **Wiederherstellen** holt Dateien zurück.

## Grenzen

- Keine Verschlüsselung und keine Kompression (ZIP ist «gespeichert»). Dafür lässt sich jede
  Sicherung ohne Sicherwerk öffnen: Ordner direkt, ZIP mit jedem Entpacker.
- Geöffnete, gesperrte Dateien (z. B. eine laufende Outlook-PST) können unter Windows nicht
  gelesen werden – sie erscheinen als Fehler im Protokoll (kein Volumenschattendienst).
- Kein Prüfen des freien Platzes vor dem Lauf; ist das Ziel voll, melden die Kopierfehler es.
- Dateirechte/Besitzer, ACLs, alternative Datenströme und Ordner-Zeitstempel werden nicht
  übernommen – nur Inhalt und Änderungszeit der Dateien.
- Linux: Dateinamen, die kein gültiges UTF-8 sind, werden übersprungen (mit Hinweis).
- Harte Verknüpfungen: höchstens etwa 1000 Versionen auf NTFS (dann wird kopiert).
- `_entfernt` wird nicht automatisch geleert.
- Die Windows-Aufgabe läuft nur, solange der Benutzer angemeldet ist; verbundene
  Netzlaufwerke (Buchstaben) sieht sie oft nicht – lieber `\\server\freigabe` verwenden.
- Ändert man Quellen, Ziel oder Art, fragt Sicherwerk wieder nach der Vorschau; die
  Zeitplan-Läufe laufen trotzdem.

## Aufbau

- `src/auftrag.rs` – Aufträge, Protokolle, Namensregeln (serde nur über `cfg_attr`).
- `src/regeln.rs` – Ausschlussmuster (Glob).
- `src/plan.rs` – Spiegel-Vergleich, Versionsnamen, Aufbewahrung, sichere Pfade.
- `src/pruefsumme.rs` – SHA-256 und CRC-32.
- `src/zipper.rs` – ZIP schreiben/lesen (gespeichert, ZIP64, UTF-8-Namen).
- `src/zeitplan.rs` – Aufgaben-XML für Windows, crontab-Zeile, systemd-Einheiten.
- `src/sperre.rs` – Sperrdateien.
- `src/motor.rs` – Lesen, Spiegeln, Versionen, Prüfen, Wiederherstellen.
- `src/store.rs`, `src/lauf.rs`, `src/aufgabe.rs` – Datei, Lauf mit Protokoll, Aufgabenplanung.
- `src/app.rs` – Oberfläche; `src/main.rs` – Fenster und Befehlszeile.

## Tests

Die Logik lässt sich ohne Cargo mit `rustc` prüfen (im Programmordner):

```
rustc --edition 2021 --test src/regeln.rs -o /tmp/t && /tmp/t
rustc --edition 2021 --test src/plan.rs -o /tmp/t && /tmp/t
rustc --edition 2021 --test src/pruefsumme.rs -o /tmp/t && /tmp/t
rustc --edition 2021 --test src/zipper.rs -o /tmp/t && /tmp/t
rustc --edition 2021 --test src/zeitplan.rs -o /tmp/t && /tmp/t
rustc --edition 2021 --test src/sperre.rs -o /tmp/t && /tmp/t
rustc --edition 2021 --cfg ohne_serde --test src/auftrag.rs -o /tmp/t && /tmp/t
rustc --edition 2021 --cfg ohne_serde --test src/motor.rs -o /tmp/t && /tmp/t
rustc --edition 2021 --test src/sprache.rs -o /tmp/t && /tmp/t
```

`motor.rs` bindet dabei alle Logikmodule ein und testet mit echten Ordnern im
Temp-Verzeichnis: Spiegeln (neu/geändert/entfernt, `_entfernt`, Quelle bleibt unverändert),
Schutz bei leerer/fehlender Quelle und fehlendem Ziel, Ordner-Versionen mit harten
Verknüpfungen und Aufbewahrung, Abbruch ohne halbe Version, ZIP-Versionen, Prüfen mit
verfälschter Sicherung, Wiederherstellen ohne Überschreiben. Die ZIP-Dateien wurden
zusätzlich mit Pythons `zipfile` und `unzip -t` gegengeprüft (auch ZIP64).

## Bauen

```
cargo run --release
```

## Lizenz

Eigenes Programm der Werkbank. Verwendete Bibliotheken (egui/eframe, rfd, serde, serde_json,
chrono) stehen unter MIT- bzw. Apache-2.0-Lizenz.
