# Diawerk

Diashows aus Fotos und Drohnenvideos – Teil der Werkbank. Ordner mit Bildern und
Videos hineinziehen, Reihenfolge prüfen, **Diashow erstellen** – heraus kommt ein
MP4 (H.264) mit sauberen Übergängen.

Rust + egui, passend zu Codewerk, Wandelwerk und den anderen Werken.

## Was es macht

- **Bilder und Videos gemischt** in einer Liste (JPG, PNG, WebP, BMP, TIFF, GIF;
  MP4, MOV, MKV, AVI, WebM …). Die Drehung aus der Kamera (EXIF) wird beachtet.
- **Panoramen** werden automatisch erkannt (Seitenverhältnis ab 2,2 : 1, einstellbar).
  Sie laufen **länger** als normale Bilder und der Ausschnitt **fährt von links nach
  rechts** über das ganze Bild – gleichmässig, nur Anfang und Ende weich, mit
  Unterpixel-Genauigkeit (kein Ruckeln). Die Dauer ergibt sich aus Breite und Tempo
  (mindestens 8 s, höchstens 25 s, einstellbar). Vor und nach einem Panorama wird
  immer weich überblendet, der Schwenk läuft erst nach dem Übergang los.
- **Normale Bilder**: Bilder nahe 16:9 (z. B. 3:2) füllen den Schirm; Hochformat und
  4:3 stehen **ganz** im Bild vor einem unscharfen, abgedunkelten Hintergrund. Dazu
  ein sanfter Zoom (5 %), abwechselnd hinein und heraus. Alles abschaltbar.
- **Übergänge**: Überblenden, Schieben, Wischen (weiche Kante), Zoom-Überblendung oder
  „Gemischt (ruhig)" – meist Überblenden, dazwischen die anderen. Dauer einstellbar.
  Der Film blendet aus Schwarz ein und am Ende aus.
- **Videos** (Drohne): Standard ganze Länge (einstellbar: höchstens x s je Video), Ton wird
  weggelassen, Videos nehmen an den Übergängen teil. Andere Seitenverhältnisse als der
  Film bekommen schwarze Balken.
- **Reihenfolge von Hand**: Zeilen anklicken (Strg = einzeln dazu, Umschalt = Bereich),
  dann **⬆ ⬇**, **⏫ Anfang**, **⏬ Ende**, **Nr. … dorthin** – oder die Auswahl einfach mit der
  Maus an eine neue Stelle ziehen (die violette Linie zeigt, wo sie landet; am Rand scrollt
  die Liste mit). Dazu „Nach Namen" (natürlich: IMG_2 vor IMG_10) und „Nach Datum".
- **Einstreuen**: **Videos einstreuen**, **Panoramen einstreuen** oder **Beides einstreuen**
  verteilt sie gleichmässig zwischen die übrigen Bilder (die Reihenfolge der normalen
  Bilder bleibt). Danach lässt sich von Hand weiter feinschieben.
- **Ansicht** unter der Liste: das angeklickte Bild oder Video gross, daneben Grösse,
  Seitenverhältnis, Art (Querformat, Hochformat, Panorama, Video), Dauer im Film und
  Dateigrösse. Videos lassen sich dort direkt abspielen (▶ Abspielen / ⏸ Anhalten). Mit den
  Pfeiltasten ⬆ ⬇ durch die Liste blättern, **Entf** nimmt den Eintrag aus der Liste (die Datei
  bleibt). Die Nachbarbilder werden vorgeladen, damit das Blättern flüssig geht. Abschaltbar
  mit dem Haken **Ansicht**; die Höhe lässt sich an der Kante ziehen.
- **Hochformat erkennen**: Bilder und Videos, die höher als breit sind (Kamera-Drehung
  eingerechnet), tragen in der Liste die Marke **HOCH** bzw. «hoch». Oben steht, wie viele es
  sind, mit **wählen** (alle markieren) und **entfernen …** (nach Rückfrage alle aus der Liste
  nehmen).
- **Vorschau** im Programmfenster: **▶ Vorschau ab Anfang** (mit Vorspann), **▶ ab Auswahl**
  oder **▶ nur Auswahl**. Läuft verkleinert (640 Pixel breit, höchstens 25 Bilder/s) mit
  denselben Übergängen, Schwenks und Zeiten wie der fertige Film, ohne Musik; Tempo 1×/2×/4×,
  Stopp und Neustart im Vorschaufenster.
- **Vor- und Abspann** für den Fotografen: Vorspann im Aufbau seiner Webseite (links kleine
  Titelzeile, grosser Name, goldene Linie, Text; rechts das Porträt), Abspann mit Dank.
  Alle Texte, das Porträt und die Dauer sind einstellbar, beides abschaltbar. Die Schrift
  sucht Diawerk selbst (Windows: Segoe UI Light); sonst eine .ttf wählen.
- **Musik** (optional): beliebig viele Titel (MP3, M4A, WAV, FLAC, OGG, OPUS) in einstellbarer
  Reihenfolge; sie laufen nacheinander, werden bei Bedarf wiederholt, blenden ein und am Ende aus.
- **Ausgabeformate** wie in Wandelwerk: MP4 (H.264), MKV (H.264), MOV (H.264), WebM (VP9 + Opus).
- **Auflösung** Full HD, 4K, HD oder 640×360 zum schnellen Ausprobieren; 24–60 Bilder
  pro Sekunde; Qualität Entwurf / Gut / Sehr gut.
- **Instagram und Hochformat**: Reel/Story 1080×1920, Beitrag 1080×1350 (4:5) oder Quadrat
  1080×1080. Querformat-Fotos und -Videos stehen dann ganz im Bild vor einem unscharfen
  Hintergrund, Hochformat-Fotos füllen den Schirm, Panoramen fahren als Schwenk durch. Vor-
  und Abspann sind gestapelt: Porträt oben, Text darunter. Knopf **Für Instagram einstellen**
  setzt alles für einen kurzen Werbefilm (1080×1920, 3 s pro Bild, 0,6 s Übergang, Videos
  höchstens 6 s, MP4). Ab 3 Minuten warnt Diawerk, dass der Film für ein Reel zu lang ist.
- **Film aus einer Auswahl**: Einträge wählen und unten **nur Auswahl** anhaken – dann
  kommen nur diese in den Film (in Listenreihenfolge), samt Vor- und Abspann. So entsteht
  aus der grossen Liste ein kurzer Werbefilm, ohne die Liste umzubauen.

## Bedienung

1. Ordner oder Dateien aufs Fenster ziehen (oder oben **+ Ordner …** / **+ Bilder / Videos …**).
2. Rechts Einstellungen prüfen. Unten steht die berechnete Länge und die Zahl der erkannten Panoramen.
3. Ausgabedatei wählen, **Diashow erstellen**. Fortschritt und Restzeit werden angezeigt, Abbrechen ist jederzeit möglich.

Tipp: Reihenfolge mit der Vorschau prüfen; für einen Probefilm „Vorschau 640×360" und Qualität „Entwurf" einen Probelauf machen, dann in Full HD / 4K.

## Rechenzeit

Auf einem schwachen 2-Kern-Rechner läuft Full HD etwa in Echtzeit (Filmminute ≈ eine
Rechenminute); mehr Kerne sind schneller, 4K braucht mehrfach so lange. Beispiel
315 Bilder + 6 Videos (zusammen 6 min): bei 6 s pro Bild und 1 s Übergang rund 33 Minuten
Film, bei 7 s rund 38 Minuten, bei 5 s rund 28 Minuten.

## Motor

Jedes Bild lädt ffmpeg einmal in der Zielgrösse (Drehen, Skalieren, unscharfer
Hintergrund). Danach rechnet Diawerk jeden Frame selbst (Schwenk, Zoom, Übergänge, auf
alle Kerne verteilt) und gibt die rohen Frames an einen zweiten ffmpeg zum Kodieren
weiter. Ein Ladethread bereitet das nächste Bild vor, während das aktuelle läuft.
Videos liest ein eigener ffmpeg-Dekoder. Es braucht keine Crates ausser egui/eframe/rfd.

- **ffmpeg** wird wie bei Wandelwerk gefunden: neben der Programmdatei, eingebaut
  (Release-Build über `werk-prebuild.sh`) oder auf dem PATH.
- Ein Bild, das sich nicht lesen lässt, wird als schwarzes Bild eingesetzt und unter
  „Hinweise" gemeldet; Dateien ohne erkennbare Bilddaten kommen gar nicht in die Liste.
- Nicht unterstützt: HEIC/RAW (kommt auf den ffmpeg-Build an). Nur nativ (Windows, Linux),
  keine Browser-Version.

## Aufbau

- `src/model.rs` – reine Logik: Einstellungen, Dateierkennung, Ausmessen (JPEG-Kopf mit
  EXIF, sonst `ffmpeg -i`), Zeitplan, Panorama-Regeln, ffmpeg-Filter. **Eigenständig getestet.**
- `src/frames.rs` – Bildrechnerei: Ausschnitt mit Bruchteilen von Pixeln, Übergänge,
  Ein-/Ausblenden. **Eigenständig getestet.**
- `src/render.rs` – Motor: Lader, Frame-Schleife, Kodierer, Fortschritt, Abbruch.
  **Mit echten Bildern und Videos durchgespielt.**
- `src/tool.rs` – ffmpeg finden, Vorschlag für die Ausgabedatei.
- `src/app.rs` – die egui-Oberfläche. `src/main.rs` – Fenster und Start.
- `testlauf/cli.rs` – Kommandozeilen-Testlauf des Motors ohne Oberfläche.

## Hinweis zum Stand

`model.rs`, `frames.rs` und der Motor (`render.rs`) sind hier ohne externe Crates
gebaut und geprüft: 24 Tests mit `rustc --test`, dazu Läufe mit Testbildern (4:3, 3:2,
16:9, Hochformat mit EXIF-Drehung, Panoramen 6:1, PNG, kaputte Dateien) und
Videos (Full HD, 4K, Hochformat) in allen Übergangsarten, mit und ohne Musik. Die
Länge des fertigen Films stimmte jeweils bis auf den Frame mit dem Zeitplan. Vor-/Abspann, Musik-Titelfolge und alle vier Ausgabeformate wurden ebenfalls durchgespielt.

Die Oberfläche (`app.rs`) ist gegen die dokumentierten APIs von egui/eframe 0.29 und
rfd 0.14 geschrieben, liess sich in meiner Umgebung aber nicht mit den echten Crates
bauen (kein Zugriff auf crates.io). Wenn `cargo` beim ersten Bauen meckert, Meldung
zurückgeben – dann wird es punktgenau angepasst.

## Portable Ausgabe (ffmpeg eingebaut)

Der Ablauf «Eigene-Release» baut Diawerk als **portable Windows-Version**: eine einzige
`diawerk.exe` im ZIP `diawerk-<version>-windows-x64-portable.zip`, ohne Installation. ffmpeg
steckt in der Programmdatei (`werk-prebuild.sh` holt es, `build.rs` bettet es ein) und wird
beim ersten Start einmal nach `%LOCALAPPDATA%\Diawerk` entpackt. Liegt eine eigene `ffmpeg.exe`
neben dem Programm, hat diese Vorrang. Die Programmdatei ist dadurch rund 100 MB gross.
`werk-prebuild.sh` bricht ab, wenn dem ffmpeg ein nötiger Encoder (H.264, VP9, AAC, Opus)
oder der Filter `drawtext` fehlt. Lizenz und Quellen von ffmpeg: `LICENSE-ffmpeg.txt` im ZIP.

## Bauen und starten

```
cargo run --release
```

Fertige Datei danach unter `target/release/diawerk` (bzw. `diawerk.exe`). Für den
Probelauf des Motors ohne Oberfläche:

```
rustc --edition 2021 -O testlauf/cli.rs -o diawerk-cli
./diawerk-cli --draft --size 640x360 --fps 15 MeinOrdner -o probe.mp4
```

(Optionen: `--still s`, `--trans s`, `--transition fade|push|wipe|zoom`, `--pano-ratio r`,
`--pano-speed px`, `--video-max s`, `--music datei` (mehrfach), `--portrait bild`, `--no-credits`, `--card-secs s`, `--format mkv|mov|webm`, `--spread`, `--no-crop`, `--best`.)

## Im Launcher

In `launcher/apps.toml` ergänzen:

```
[[app]]
id = "diawerk"
name = "Diawerk"
about = "Diashows aus Fotos und Drohnenvideos"
color = "#C2410C"
repo = "https://github.com/0ldini/werkbank"
tag = "diawerk-v"
no_web = true
```

Ordner `diawerk/` nach `eigene/` legen, dann «Eigene-Release» → `diawerk` starten.
