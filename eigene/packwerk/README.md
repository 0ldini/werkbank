# Packwerk

Archive packen und entpacken – schlicht, wie 7-Zip oder WinRAR, aber ohne
Schnickschnack. Teil der Werkbank.

Rust + egui, passend zu Wandelwerk, Umbenennwerk, Speicherwerk und den anderen
eigenen Programmen. Läuft unter Windows und Linux.

## Funktionen

- **Öffnen und ansehen**: ZIP, 7z, TAR, TAR.GZ/TGZ und GZ. Der Inhalt erscheint
  als Ordner-Ansicht (Doppelklick öffnet einen Ordner, ⬆ bzw. Rücktaste geht
  zurück) mit Name, Grösse, gepackter Grösse und Änderungsdatum; Spalten lassen
  sich durch Klick auf den Kopf sortieren. Die Art wird am Inhalt erkannt, nicht
  nur an der Endung.
- **Entpacken**: alles oder nur die markierten Einträge (Klick, Strg+Klick,
  Umschalt+Klick, Strg+A). Markierte Einträge werden relativ zum aktuellen
  Ordner entpackt. Zielordner frei wählbar; vorgeschlagen wird ein Ordner neben
  dem Archiv mit dessen Namen.
- **Neues Archiv**: Dateien und Ordner per Ziehen und Ablegen oder per Dialog
  hinzufügen, Format (ZIP, 7z, TAR, TAR.GZ, GZ für eine einzelne Datei) und
  Stärke (Schnell/Normal/Stark) wählen, Name und Ordner festlegen, erstellen.
- **Passwörter**: ZIP mit AES-256 (die Dateinamen bleiben sichtbar), 7z mit
  AES-256 inklusive verschlüsselter Dateinamen. Verschlüsselte Archive fragen
  beim Öffnen bzw. Entpacken nach dem Passwort; ein falsches Passwort wird
  erkannt und es bleiben keine halben Dateien liegen.
- **Fortschritt mit Abbrechen** für Lesen, Entpacken und Packen; die Arbeit läuft
  im Hintergrund, die Oberfläche bleibt bedienbar.
- **Zweisprachig** (Deutsch/Englisch) und **hell/dunkel**, wie im Launcher
  gewählt.

## Schutz

- **Zip Slip**: Einträge mit «..», absoluten Pfaden, Laufwerksangaben oder – unter
  Windows – ungültigen bzw. reservierten Namen (CON, NUL, «a:b» …) werden nicht
  entpackt. Jeder Zielpfad wird zusätzlich gegen den Zielordner geprüft. Solche
  Einträge sind in der Ansicht rot mit ⚠ markiert und erscheinen im Bericht.
- **Verknüpfungen** (Symlinks, Hardlinks) aus Archiven werden nie angelegt, nur
  gemeldet. Beim Packen werden Verknüpfungen übersprungen.
- **Archiv-Bomben**: Vor dem Entpacken warnt Packwerk, wenn das Ergebnis grösser
  als 8 GB wird, stärker als 1:100 gepackt ist (ab 256 MB), mehr als 200 000
  Einträge hat oder sich ZIP-Einträge überlappen. Beim Entpacken wird pro Eintrag
  höchstens die angegebene Grösse geschrieben.
- **Keine Datenverluste**: Gibt es Dateien im Ziel schon, fragt Packwerk
  (Beide behalten / Überspringen / Überschreiben). Ein bestehendes Archiv wird
  nur nach Rückfrage ersetzt. Geschrieben wird immer zuerst in eine versteckte
  Teil-Datei, die erst am Ende umbenannt wird; bei Abbruch oder Fehler wird sie
  gelöscht.

## Bedienung

- Archiv öffnen: «Öffnen …», ins Fenster ziehen oder im Dateimanager mit
  «Öffnen mit» Packwerk wählen.
- Dateien oder Ordner ins Fenster ziehen, die kein Archiv sind: Packwerk
  wechselt zu «Neues Archiv» und nimmt sie in die Liste.

### Kommandozeile

```
packwerk <archiv>                       Archiv anzeigen («Öffnen mit»)
packwerk --entpacken <archiv> [ziel]    sofort entpacken (ohne Ziel: Ordner neben dem Archiv)
packwerk <datei/ordner> …               neues Archiv mit diesen Einträgen vorbereiten
packwerk --hilfe                        kurze Hilfe
```

`--entpacken` darf auch `-x` oder `--extract` heissen. Auch beim Entpacken über
die Kommandozeile erscheinen Rückfragen (Passwort, vorhandene Dateien, Warnung
vor Archiv-Bomben) im Fenster.

Dateitypen für «Öffnen mit»: `.zip .7z .tar .gz .tgz` (auch `.tar.gz`).

## Grenzen

- Bestehende Archive werden nicht verändert (kein Hinzufügen oder Löschen
  einzelner Einträge in ein vorhandenes Archiv) – stattdessen ein neues Archiv
  erstellen.
- RAR, XZ/TAR.XZ, BZ2/TAR.BZ2 und mehrteilige Archive werden nicht unterstützt.
- ZIP mit Zstandard-Kompression lässt sich nicht entpacken; alte
  ZipCrypto-Verschlüsselung nur lesen, Packwerk selbst verschlüsselt immer mit
  AES-256. Ältere Windows-Explorer öffnen AES-ZIPs nicht (7-Zip und Packwerk schon).
- 7z wird «solid» gepackt: Einzelne Einträge aus grossen 7z-Archiven zu entpacken
  braucht so lange wie alles bis dahin.
- Dateirechte werden nur grob übernommen (unter Linux: ausführbar ja/nein),
  Besitzer und erweiterte Attribute nicht.
- Dateinamen, die kein gültiges Unicode sind, werden beim Packen angepasst.
- Der freie Speicherplatz wird vor dem Entpacken nicht geprüft.

## Bauen und starten

```
cargo run --release -- [archiv]
```

Fertige Datei danach unter `target/release/packwerk` (bzw. `packwerk.exe` unter
Windows). Unter Linux braucht der Bau die üblichen GTK-/X11-Pakete (siehe
Workflow «Eigene-Release»).

## Aufbau

- `src/regeln.rs` – reine Regeln ohne Oberfläche: Archivart erkennen,
  Eintragspfade prüfen (Zip Slip), freie Namen («(2)»), Warnungen vor
  Archiv-Bomben, Kommandozeile. **Eigenständig getestet:**
  `rustc --edition 2021 --test src/regeln.rs -o /tmp/r && /tmp/r`
- `src/anzeige.rs` – Grössen und Zeiten formatieren, Ordnerbaum aus der flachen
  Eintragsliste, natürliches Sortieren. **Eigenständig getestet:**
  `rustc --edition 2021 --test src/anzeige.rs -o /tmp/a && /tmp/a`
- `src/archiv.rs` – Motor: Lesen, Entpacken und Packen für alle Formate, mit
  Fortschritt und Abbruch (läuft in Hintergrund-Threads).
- `src/app.rs` – die egui-Oberfläche (Ansicht, Neues Archiv, Rückfragen).
- `src/main.rs` – Fenster, Start, Kommandozeile.
- `src/sprache.rs`, `src/en.tsv` – Deutsch/Englisch (Prüftest:
  `rustc --edition 2021 --test src/sprache.rs -o /tmp/s && /tmp/s`).
- `src/thema.rs` – hell/dunkel.

## Abhängigkeiten und Lizenzen

Verwendet werden egui/eframe, rfd, zip, sevenz-rust2, tar, flate2 und chrono –
alle unter MIT und/oder Apache-2.0; ihre Abhängigkeiten stehen unter ähnlich
freien Lizenzen (z. B. die BSD-artige bzip2-Lizenz). Packwerk selbst steht unter der Lizenz der Werkbank
(siehe `LICENSE` im Hauptordner).
