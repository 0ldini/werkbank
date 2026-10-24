# Abspielwerk

Videos, Musik und Bilder ansehen – Teil der Werkbank. Ordner oder Dateien öffnen (oder ins
Fenster ziehen), in der Wiedergabeliste anklicken, fertig. Videos und Musik spielt
**libVLC** (der Motor von VLC), Bilder zeigt Abspielwerk selbst.

Rust + egui, passend zu Diawerk, Wandelwerk und den anderen Werken.

## Was es macht

- **Eine Liste für alles**: Öffnen einer Datei füllt die Wiedergabeliste mit allen Videos,
  Musikstücken und Bildern ihres Ordners (natürlich sortiert: «Bild 2» vor «Bild 10»). Jede
  Zeile trägt eine Marke **VIDEO**, **MUSIK** oder **BILD**. Hineingezogene Dateien und Ordner
  kommen hinten dazu. Rechtsklick: anzeigen oder aus der Liste nehmen. Die Liste lässt sich
  oben ein- und ausblenden.
- **Videos** (MP4, MKV, MOV, AVI, WebM, WMV, MPEG, TS …): Bild im Fenster mit schwarzen
  Balken, Suchleiste mit Zeit («0:42 / 1:23:10»), Lautstärke und stumm, **Tonspur** und
  **Untertitel** aus der Datei, **Geschwindigkeit** 0,5× bis 2×.
- **Musik** (MP3, FLAC, M4A, AAC, OGG, Opus, WAV, WMA, AIFF …): Albumbild mit Titel,
  Künstler und Album (aus den Angaben der Datei).
- **Bilder** (JPG, PNG, WebP, GIF – erstes Bild –, BMP, TIFF): ins Fenster eingepasst, die
  Drehung aus der Kamera (EXIF) wird beachtet. Mausrad zoomt um den Mauszeiger, Ziehen
  verschiebt, **Nach links/rechts drehen** (nur Ansicht, die Datei bleibt), **Einpassen**.
  **Diashow** mit wählbarer Zeit pro Bild (2–20 s). **In Papierkorb …** mit Rückfrage.
- **Wiederholen** (nicht / ganze Liste / aktuelles Stück) und **Zufall**. Am Ende eines Videos
  oder Stücks geht es mit dem nächsten Video oder Stück weiter; Bilder blättert man selbst
  oder mit der Diashow.
- **Vollbild** mit F oder Doppelklick; die Bedienung erscheint, sobald man die Maus bewegt.
- Merkt sich Lautstärke, stumm, Wiederholen/Zufall, Geschwindigkeit, Diashow-Zeit, die
  Wiedergabeliste ein/aus und den zuletzt geöffneten Ordner.
- Startet mit einer Datei oder einem Ordner auf der Befehlszeile (`abspielwerk film.mkv`),
  z. B. aus dem Werkbank-Launcher oder mit «Öffnen mit».
- Deutsch oder Englisch, hell oder dunkel – wie im Launcher gewählt.

## Tasten

| Taste | Video / Musik | Bild |
|---|---|---|
| Leertaste | Abspielen / Pause | Diashow an / aus |
| ← / → | 5 s zurück / vor | vorheriges / nächstes Bild |
| ↑ / ↓ | lauter / leiser | lauter / leiser |
| N / P | nächstes / vorheriges | nächstes / vorheriges |
| M | stumm | stumm |
| F oder Doppelklick | Vollbild | Vollbild |
| Esc | Vollbild beenden | Vollbild beenden |
| Entf | – | In Papierkorb … |

## libVLC

Abspielwerk lädt libVLC beim Start (es ist nicht fest eingebaut). Fehlt es, startet das
Programm trotzdem, meldet es und zeigt weiter Bilder.

- **Windows**: libVLC 3.0 (libvlc.dll, libvlccore.dll und der Ordner `plugins`) liegt im ZIP
  neben `abspielwerk.exe`. Nicht nötige Plugins (Oberflächen, Lua, Visualisierungen,
  Streaming-Ausgabe) sind weggelassen.
- **macOS** (Apple Silicon): libVLC liegt im Programm unter `Contents/MacOS/lib` und
  `Contents/MacOS/plugins`.
- **Linux**: libVLC des Systems, z. B. `sudo apt install vlc` (Debian/Ubuntu) oder
  `sudo dnf install vlc` (Fedora).

libVLC steht unter der GNU Lesser General Public License 2.1 oder später (einzelne Plugins
unter der GPL 2 oder später); der Quelltext ist unter https://code.videolan.org/videolan/vlc
zu finden. Der Lizenztext liegt dem Paket als `LICENSE-vlc.txt` bei.

## Bauen

`cargo run --release`. Der Ablauf «Eigene-Release» holt vorher mit `werk-prebuild.sh` libVLC
von videolan.org (Windows: ZIP, macOS: dmg; mit Prüfsumme) und packt es zum Programm.

Einzeln prüfbar (ohne Bibliotheken):
`rustc --edition 2021 --test src/medien.rs -o /tmp/medien && /tmp/medien` und
`rustc --edition 2021 --test src/sprache.rs -o /tmp/sprache && /tmp/sprache`.
