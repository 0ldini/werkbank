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

## Launcher veröffentlichen

Tag `werkbank-vX.Y.Z` pushen; «Launcher-Release» baut Windows und Linux und legt das Release an.

## Eigene Apps aufnehmen

HTML5-Apps kommen als Eintrag mit `web_only = true` in `launcher/apps.toml`. Die App wird als
`<id>-web-<version>.zip` (ein Ordner mit `index.html`) in einem Release mit dem Tag
`<id>-v<version>` abgelegt, zusammen mit `SHA256SUMS.txt`.

## Was (noch) fehlt

- Kein macOS-Build (bräuchte Apple-Zertifikat).
- Windows-Programme sind unsigniert; Windows zeigt beim ersten Start eine Warnung.
- Die Programmdateien heissen wie im Original (`pdfcraft.exe` usw.); sichtbar ist überall der Werk-Name.

## Lizenz

Launcher, Skripte und Icons in diesem Repository: MIT (siehe `LICENSE`).
Die gebauten Programme: wie das jeweilige Original, MIT oder Apache-2.0.
