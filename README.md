# Werkbank

Zwölf Kreativ-Programme unter eigenem Namen, ein Launcher, Updates pro Programm.

| Programm | Wofür | Original |
|---|---|---|
| **Bildwerk** | Bildbearbeitung | [PhotoCraft](https://github.com/storytold/photocraft) |
| **Linienwerk** | Vektorgrafik | [VectorCraft](https://github.com/storytold/vectorcraft) |
| **Designwerk** | Layout | [DesignCraft](https://github.com/storytold/designcraft) |
| **Blattwerk** | PDF | [PdfCraft](https://github.com/storytold/pdfcraft) |
| **Lichtwerk** | Fotobibliothek, RAW | [LightCraft](https://github.com/storytold/lightcraft) |
| **Filmwerk** | Videoschnitt | [FilmCraft](https://github.com/storytold/filmcraft) |
| **Trickwerk** | Animation, Effekte | [EffectCraft](https://github.com/storytold/effectcraft) |
| **Klangwerk** | Musik und Ton | [SoundCraft](https://github.com/storytold/soundcraft) |
| **Schreibwerk** | Textverarbeitung | [WordCraft](https://github.com/storytold/wordcraft) |
| **Rechenwerk** | Tabellen | [GridCraft](https://github.com/storytold/gridcraft) |
| **Schauwerk** | Präsentationen | [DeckCraft](https://github.com/storytold/deckcraft) |
| **Planwerk** | CAD | [CADCraft](https://github.com/storytold/cadcraft) |

Die Programme sind umbenannte Ausgaben der Open-Source-Apps des ArtCraft-Teams
(MIT oder Apache-2.0). Werkbank ist kein Produkt des ArtCraft-Teams und wird von ihm
weder unterstützt noch empfohlen. Jedes Paket enthält die Lizenzen und den Urheberhinweis
des Originals.

## Aufbau

| Ordner | Inhalt |
|---|---|
| `launcher/` | Der Launcher (Rust): installieren, starten, einzeln aktualisieren, Browser-Version |
| `launcher/apps.toml` | Programmliste; der Launcher lädt sie beim Start von hier |
| `werk/rebrand.py` | Ersetzt Name, Logos und Links im Originalcode (ein Skript für alle) |
| `werk/apps/<app>/` | Pro Programm: Name (`werk.json`) und Icons |
| `werk/tools/` | Erzeugt Icons und Konfiguration neu |
| `.github/workflows/` | Build-Abläufe |

Der Originalcode liegt nicht in diesem Repository. Der Build holt ihn bei jedem Lauf frisch
vom Original, wendet die Umbenennung an und baut. Darum gibt es bei neuen Versionen des
Originals nie Konflikte.

## Updates

- **Automatisch:** Einmal täglich prüft «App-Release», ob ein Original eine neue Version
  hat, baut sie und veröffentlicht sie als Release `<app>-v<version>`.
- **Von Hand:** Reiter «Actions» → «App-Release» → «Run workflow», Programm wählen.
- Der Launcher zeigt danach pro Programm «neu verfügbar» und aktualisiert auf Knopfdruck.

Bricht ein Build mit «das Original hat sich geändert» ab, findet `rebrand.py` den
Programmnamen oder die Icons nicht mehr dort, wo sie waren. Die Meldung nennt die Stelle.

## Ersteinrichtung

Beim ersten Start fragt der Launcher, wo die Programme abgelegt werden sollen (Vorschlag unter
Windows: `C:\Program Files\Werkbank`). Der Ordner wird angelegt; liegt er an einem geschützten Ort,
fragt Windows einmalig nach Administratorrechten, danach gehört der Ordner dem Benutzer. Die Wahl
steht in `%LOCALAPPDATA%\Werkbank\config\config.toml`; löschen zeigt die Frage erneut. Eine
`portable.txt` neben `werkbank.exe` übergeht das alles (Daten im Ordner `data` daneben).

## Dateitypen

Knopf «Dateitypen» im Launcher (und Schritt 2 der Ersteinrichtung): pro Programm die Endungen
anhaken, die es öffnen soll. Die Werkbank trägt sich dafür im Benutzerprofil ein; die Programme
erscheinen danach unter Rechtsklick → «Öffnen mit» und in den Windows-Einstellungen «Standard-Apps».
Welches Programm Standard ist, bestätigt man dort selbst (Windows erlaubt das keinem Programm).
Die Zuordnung zeigt auf `werkbank.exe --open <app> <datei>`, überlebt also jedes Update. Welche
Endungen ein Programm anbietet, steht in `launcher/apps.toml` (`types`).

## Offline-Setup für Windows

Reiter «Actions» → «Offline-Setup» → «Run workflow». Der Ablauf packt den neuesten Launcher und
alle veröffentlichten Programme in ein Setup-Programm (`werkbank-setup-<Datum>.exe`, Release
`setup-v<Datum>`). Beim Installieren wählt man die Programme; das Häkchen «nach neueren Versionen
suchen» holt vor dem Einrichten das Aktuellste aus dem Netz, ohne Internet kommt alles aus dem Setup.
Der Launcher selbst landet in `%LOCALAPPDATA%\Programs\Werkbank`; die Programme dort, wo die
Ersteinrichtung es festlegt. Deinstallieren über Windows-Einstellungen → Apps.

## Launcher veröffentlichen

Reiter «Actions» → «Launcher-Release» → «Run workflow» und eine Versionsnummer angeben (z. B. `0.1.0`). Der Ablauf baut Windows und Linux und legt das Release `werkbank-v0.1.0` an.

## Eigene Programme aufnehmen

Rust-Programme (egui/eframe) liegen unter `eigene/<name>/`; der Ablauf «Eigene-Release» baut sie für
Windows, Linux und den Browser und veröffentlicht sie wie die anderen (siehe `eigene/README.md`).
Das erste ist **Codewerk** (QR-Codes). HTML5-Apps kommen mit `web_only = true` in `launcher/apps.toml`
und als `<id>-web-<version>.zip` (ein Ordner mit `index.html`) in ein Release `<id>-v<version>`.

## Was (noch) fehlt

- Kein macOS-Build (bräuchte Apple-Zertifikat).
- Windows-Programme und Setup sind unsigniert; Windows zeigt beim ersten Start eine Warnung.
- Die Programmdateien heissen wie im Original (`pdfcraft.exe` usw.); sichtbar ist überall der Werk-Name.

## Lizenz

Launcher, Skripte und Icons in diesem Repository: Apache-2.0 (siehe `LICENSE`).
Die gebauten Programme: wie das jeweilige Original, MIT oder Apache-2.0.
