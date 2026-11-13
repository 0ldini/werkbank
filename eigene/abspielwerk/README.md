# Abspielwerk

Videos, Musik und Bilder ansehen – Teil der Werkbank. Ordner oder Dateien öffnen (oder ins
Fenster ziehen), in der Wiedergabeliste anklicken, fertig. Videos und Musik spielt
**libVLC** (der Motor von VLC), Bilder zeigt Abspielwerk selbst.

Rust + egui, passend zu Diawerk, Wandelwerk und den anderen Werken.

## Was es macht

Aufbau und Bedienung wie im **VLC media player** (nachgebaut nach dem, was VLC öffentlich
zeigt; keine Teile von VLC übernommen), im Aussehen der Werkbank:

- **Menüleiste** oben:
  - **Medien**: Datei öffnen …, Mehrere Dateien öffnen …, Ordner öffnen …, Netzwerkstream
    öffnen … (http, https, rtsp, rtp, udp, mms …), Zuletzt geöffnet (10), Wiedergabeliste
    speichern … (M3U oder XSPF), Wiedergabeliste leeren, In Papierkorb …, Beenden.
    M3U/M3U8-Listen lassen sich öffnen.
  - **Wiedergabe**: Titel und Kapitel (DVD/Blu-ray, falls vorhanden), Geschwindigkeit
    (0,25× bis 4×), Sprung vorwärts/rückwärts, Zu Zeit springen …, Abspielen/Pause, Stopp,
    Vorheriges, Nächstes, Bild für Bild, Wiederholen, Zufall.
  - **Audio**: Tonspur, Audiogerät, Stereo-Modus, lauter/leiser, stumm.
  - **Video**: Vollbild, Seitenverhältnis (Standard, 16:9, 4:3, 1:1, 16:10, 2.21:1, 2.35:1,
    2.39:1, 5:4), Zuschneiden (16:10, 16:9, 4:3, 1.85:1, 2.21:1, 2.35:1, 2.39:1, 5:3, 5:4,
    1:1), Deinterlace (blend, bob, yadif …), Bildschirmfoto; für Bilder Drehen, Einpassen,
    Diashow.
  - **Untertitel**: Untertiteldatei hinzufügen … (SRT, ASS, VTT …), Untertitelspur,
    Verzögerung ±50 ms.
  - **Werkzeuge**: Effekte und Filter (Audio-Equalizer mit den Vorlagen von libVLC,
    Vorverstärkung und 10 Bändern; Video-Anpassungen Helligkeit, Kontrast, Sättigung,
    Farbton, Gamma), Medieninformationen (Allgemein, Codec, Statistik), Einstellungen.
  - **Ansicht**: Wiedergabeliste, Minimale Ansicht (ohne Menü und Liste), Immer im
    Vordergrund, Vollbild. **Hilfe**: Tastenkürzel, Über.
  - Dieselben Menüs gibt es mit einem **Rechtsklick ins Bild** (auch in der minimalen Ansicht).
- **Bedienleiste** unten: Suchleiste mit abgelaufener Zeit und Gesamtzeit (Klick auf die
  Gesamtzeit: verbleibende Zeit), darunter Abspielen/Pause, Vorheriges, Stopp, Nächstes,
  Vollbild, Erweiterte Einstellungen (Effekte und Filter), Wiedergabeliste, Wiederholen
  (nicht / alle / eines), Zufall und rechts die **Lautstärke bis 125 %**.
- **Wiedergabeliste** rechts mit den Spalten Titel, Dauer, Album (aus den Angaben der Dateien,
  im Hintergrund gelesen), Suchfeld, Doppelklick spielt, Ziehen ordnet um, Entf nimmt den
  gewählten Eintrag heraus. Öffnen einer Datei füllt die Liste mit allen Videos, Musikstücken
  und Bildern ihres Ordners (natürlich sortiert: «Bild 2» vor «Bild 10»); hineingezogene
  Dateien und Ordner kommen hinten dazu.
- **Fortsetzen**: Abspielwerk merkt sich, wo ein Video (ab 1 min) oder ein langes Musikstück
  (ab 20 min) aufgehört hat, und fragt beim nächsten Öffnen «Fortsetzen ab 12:34?» (in den
  Einstellungen: Fragen, Immer, Nie).
- **Mausrad** über dem Video = Lautstärke, **Doppelklick** = Vollbild. Im Vollbild erscheint
  die Bedienung, sobald man die Maus bewegt, und verschwindet nach 3 s wieder (mit dem
  Mauszeiger). Kurze Anzeigen oben rechts im Bild (Lautstärke, Tempo, Zeit …) wie bei VLC.
- **Bildschirmfoto** (Umschalt+S): PNG im Ordner «Bilder» des Benutzers (oder einem anderen in
  den Einstellungen), Name wie «abspielwerk-2026-11-13-14h05m09s.png».
- **Videos** (MP4, MKV, MOV, AVI, WebM, WMV, MPEG, TS …), **Musik** (MP3, FLAC, M4A, AAC, OGG,
  Opus, WAV, WMA, AIFF …) mit Albumbild, Titel, Künstler und Album.
- **Bilder** (JPG, PNG, WebP, GIF – erstes Bild –, BMP, TIFF): ins Fenster eingepasst, die
  Drehung aus der Kamera (EXIF) wird beachtet. Mausrad zoomt um den Mauszeiger, Ziehen
  verschiebt, Drehen (nur Ansicht), Einpassen, **Diashow** (2–20 s pro Bild), **In Papierkorb …**
  mit Rückfrage.
- Am Ende eines Videos oder Stücks geht es mit dem nächsten Video oder Stück weiter; Bilder
  blättert man selbst oder mit der Diashow.
- Merkt sich Lautstärke, stumm, Wiederholen/Zufall, Geschwindigkeit, Diashow-Zeit, die
  Wiedergabeliste ein/aus, verbleibende Zeit, Equalizer, Video-Anpassungen, Fortsetzen-
  Positionen, «Zuletzt geöffnet» und den zuletzt geöffneten Ordner.
- Startet mit einer Datei oder einem Ordner auf der Befehlszeile (`abspielwerk film.mkv`),
  z. B. aus dem Werkbank-Launcher oder mit «Öffnen mit».
- Deutsch oder Englisch, hell oder dunkel – wie im Launcher gewählt.

## Tasten

Wie in VLC (auch unter Hilfe › Tastenkürzel). Unter macOS steht ⌘ für Strg.

| Taste | Video / Musik | Bild |
|---|---|---|
| Leertaste | Abspielen / Pause | Diashow an / aus |
| S | Stopp | Diashow aus |
| N / P | nächstes / vorheriges | nächstes / vorheriges |
| F oder Doppelklick, Esc | Vollbild, Vollbild beenden | ebenso |
| ← / → (auch Alt+← / →) | 10 s zurück / vor | vorheriges / nächstes Bild |
| Umschalt+← / → | 3 s zurück / vor | – |
| Strg+← / → | 1 min zurück / vor | – |
| Strg+Alt+← / → | 5 min zurück / vor | – |
| ↑ / ↓, Strg+↑ / ↓, Mausrad | lauter / leiser (bis 125 %) | lauter / leiser (Mausrad: zoomen) |
| M | stumm | stumm |
| + / - / = | schneller / langsamer / normal | – |
| E / T | Bild für Bild / Zeit anzeigen | – |
| Umschalt+S | Bildschirmfoto | – |
| B / V | Tonspur / Untertitel wechseln | – |
| G / H | Untertitel-Verzögerung −50 / +50 ms | – |
| A / C / D | Seitenverhältnis / Zuschneiden / Deinterlace wechseln | – |
| L / R | Wiederholen / Zufall | ebenso |
| Entf | gewählten Eintrag aus der Liste nehmen | In Papierkorb … |
| Strg+O, Strg+Umschalt+O, Strg+F, Strg+N | Datei, mehrere Dateien, Ordner, Netzwerkstream öffnen | ebenso |
| Strg+Y, Strg+L, Strg+T, Strg+H | Liste speichern, Liste ein/aus, Zu Zeit springen, Minimale Ansicht | ebenso |
| Strg+E, Strg+I, Strg+P, Strg+Q, F1 | Effekte, Medieninformationen, Einstellungen, Beenden, Tastenkürzel | ebenso |

Seitenverhältnis und Zuschneiden rechnet Abspielwerk beim Zeichnen selbst (libVLC liefert die
Bilder über «vmem» in voller Grösse); Deinterlace, Equalizer und Video-Anpassungen macht libVLC.
Fehlt einer libVLC-Version eine dieser Funktionen, ist der Menüeintrag ausgegraut.

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

Einzeln prüfbar (ohne Bibliotheken), z. B.
`rustc --edition 2021 --test src/befehl.rs -o /tmp/befehl && /tmp/befehl` – ebenso
`src/merken.rs` (Fortsetzen, Zuletzt, M3U/XSPF, Bildschirmfoto-Namen), `src/medien.rs` und
`src/sprache.rs` (alle Texte übersetzt).

`cargo test` prüft dazu mit einem echten libVLC ein selbst erzeugtes Video von Anfang bis Ende
(Bildschirmfoto, Equalizer, Anpassungen, Deinterlace, Untertiteldatei, Bild für Bild, Adresse);
ohne libVLC wird dieser Test übersprungen. In GitHub läuft das im Ablauf «Abspielwerk-Test»
unter Linux und Windows.
