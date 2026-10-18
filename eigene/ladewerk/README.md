# Ladewerk

Ein Download-Manager im Stil von JDownloader – Teil der Werkbank. Links (auch viele auf
einmal oder in ganzen Texten) einfügen, zu Paketen zusammenfassen, herunterladen, bei
Abbrüchen fortsetzen, prüfen und automatisch entpacken. Eigenständige Desktop-App für
Windows und Linux, Rust + egui, passend zu den anderen Werk-Programmen.

## Funktionen

- **Linksammler**: Text einfügen (Strg+V, auch HTML, Foren-Beiträge, Listen) – alle
  http(s)-Links werden erkannt, doppelte nur einmal. Einzelne Links lassen sich abwählen.
  Steht in derselben Zeile hinter einem Link eine SHA-256-Prüfsumme (64 Hex-Zeichen), wird
  sie übernommen. Links, die schon in der Warteschlange sind, werden markiert.
- **Strg+V im Hauptfenster** (ohne aktives Textfeld) übergibt den Text direkt an den
  Linksammler.
- **Zwischenablage beobachten** (optional): neu kopierte Links landen von selbst im
  Linksammler (was beim Einschalten schon kopiert war, nicht).
- **Pakete**: Gruppen von Links mit gemeinsamem Namen und Zielordner (auf Wunsch ein
  Unterordner mit dem Paketnamen), eigener Passwortliste und Entpack-Schalter. Der Name wird
  aus dem ersten Dateinamen vorgeschlagen (`Film.part01.rar` → «Film»).
- **Warteschlange** mit einstellbarer Zahl gleichzeitiger Downloads (1–20), Starten/Stoppen
  für alles, Pausieren/Fortsetzen je Link und je Paket. Ein einzeln fortgesetzter Link lädt
  auch, wenn die Warteschlange gestoppt ist.
- **Fortsetzen über HTTP-Range**: Es wird in `name.part` geladen; nach Pause, Abbruch oder
  Programmende geht es an derselben Stelle weiter. `If-Range` mit ETag/Last-Modified
  verhindert, dass eine inzwischen geänderte Datei zusammengestückelt wird; kann der Server
  nicht fortsetzen, beginnt Ladewerk sauber von vorne.
- **Anzeige**: Fortschritt, Grösse, Geschwindigkeit und Restzeit je Link und Paket,
  Gesamtgeschwindigkeit oben rechts.
- **Wiederholen bei Fehlern**: vorübergehende Fehler (Verbindung, Zeitüberschreitung, HTTP
  408/429/5xx) werden mit wachsender Wartezeit erneut versucht (Anzahl und erste Wartezeit
  einstellbar). Endgültige Fehler (404, 403 …) werden sofort angezeigt; «↻» versucht es
  erneut.
- **Geschwindigkeitsbegrenzung** für alle Downloads zusammen (KB/s, 0 = unbegrenzt).
- **Dateiname** aus `Content-Disposition` (auch `filename*=UTF-8''…`), sonst aus der Adresse
  (nach Weiterleitungen); für Windows und Linux bereinigt.
- **Nie überschreiben**: Gibt es den Namen schon, heisst die neue Datei `name (2).ext`.
  Entfernen aus der Liste lässt Dateien liegen; Teildateien werden nur nach Rückfrage
  gelöscht.
- **Prüfsumme**: optionaler SHA-256-Vergleich nach dem Download (Feld in den Details);
  bei fertigen Dateien auch nachträglich mit «Jetzt prüfen».
- **Automatisch entpacken**, sobald alle Links eines Pakets fertig sind:
  - ZIP direkt in Ladewerk (auch mit Passwort, ZipCrypto und AES).
  - 7z, RAR (auch `.part1.rar`/`.r00`) und geteilte ZIP (`.z01`, `.zip.001`) über ein
    installiertes **7-Zip** (`7z`/`7zz`; unter Windows wird `C:\Program Files\7-Zip` gefunden).
  - Passwörter: erst ohne, dann die des Pakets, dann die allgemeine Passwortliste.
  - Entpackt wird in einen versteckten Arbeitsordner und erst bei Erfolg an den Platz
    verschoben – nie über bestehende Dateien. Auf Wunsch jedes Archiv in einen eigenen
    Ordner und die Archive danach löschen (Standard: behalten).
- **Gespeichert und fortgesetzt**: Warteschlange und Einstellungen liegen in
  `%APPDATA%\Ladewerk\ladewerk.txt` bzw. `~/.config/ladewerk/ladewerk.txt` (alle paar
  Sekunden und beim Beenden). Beim nächsten Start geht es weiter (abschaltbar).
- **Proxy** aus der Umgebung (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`) oder fest
  in den Einstellungen (`http://…`, `socks5://…`).
- Deutsch und Englisch, helles und dunkles Design (beides kommt aus dem Werkbank-Launcher).

## Bedienung

1. Links kopieren und im Fenster **Strg+V** drücken (oder im Reiter «Linksammler» einfügen).
2. Im Linksammler Paketname, Zielordner, Entpacken und Passwörter prüfen, dann
   «… zur Warteschlange hinzufügen».
3. Im Reiter «Downloads» laufen die Pakete. **⏸/▶** pausiert bzw. setzt fort, **…** öffnet
   weitere Aktionen (Link kopieren, von vorne laden, entfernen). Ein Klick auf einen
   Dateinamen zeigt unten die Details (Link, Datei, SHA-256, Meldung).
4. Im Reiter «Einstellungen»: Download-Ordner, gleichzeitige Downloads, Grenze, Wiederholen,
   Entpacken, Passwortliste, Proxy.

## Grenzen

- Nur direkte http- und https-Links. Seiten von Filehostern (mit Wartezeit, Captcha oder
  Konto) werden nicht ausgewertet – geladen wird, was der Server unter dem Link liefert
  (bei solchen Seiten meist eine HTML-Seite).
- Kein FTP, keine Torrents, keine Videoseiten.
- Ein Link wird mit einer Verbindung geladen (kein Aufteilen in Stücke).
- Hängt eine Verbindung mitten im Download ohne Fehlermeldung, wartet dieser Download, bis
  das Betriebssystem die Verbindung aufgibt (auch eine Pause greift erst dann).
- 7z und RAR brauchen ein installiertes 7-Zip. Ohne 7-Zip meldet das Paket den Fehler; die
  Archive bleiben unangetastet.

## Mögliche Erweiterung: Hoster-Konten

Eine spätere Version könnte Premium-Konten bei Filehostern (z. B. rapidgator) über deren
offizielle Schnittstellen nutzen, also mit dem eigenen, bezahlten Konto des Benutzers.
Das ist bewusst **nicht** umgesetzt. Ladewerk umgeht weder Captchas noch Wartezeiten,
Bandbreiten- oder andere Beschränkungen von Anbietern und wird das auch nicht tun.

## Bauen und starten

```
cargo run --release
```

Fertige Datei danach unter `target/release/ladewerk` (bzw. `ladewerk.exe` unter Windows).

## Aufbau

- `src/links.rs` – reine Logik: Links in Text finden (mit SHA-256), Dateiname aus
  Content-Disposition und Adresse, sichere und freie Dateinamen, Weiterleitungsziele.
  **Eigenständig getestet.**
- `src/plan.rs` – reine Logik: Grössen-, Geschwindigkeits- und Restzeit-Anzeige,
  Geschwindigkeitsgrenze (Token-Eimer), Content-Range, HTTP-Antworten einordnen,
  Wartezeit beim Wiederholen, Archive und Archivteile erkennen, Entpack-Plan.
  **Eigenständig getestet.**
- `src/store.rs` – gespeicherter Zustand (Einstellungen, Pakete, Links) als Textdatei,
  nachsichtiges Lesen, sicheres Speichern. **Eigenständig getestet.**
- `src/net.rs` – ein Download (ureq 3 + rustls): Weiterleitungen, Range/If-Range,
  Teildatei, Grenze, Abbruch, Prüfsumme.
- `src/engine.rs` – Warteschlange im Hintergrund-Thread: verteilen, Fortschritt,
  Wiederholen, Entpacken anstossen, speichern.
- `src/unpack.rs` – Entpacken (zip-Crate, 7-Zip als externes Programm).
- `src/clip.rs` – Zwischenablage beobachten (arboard).
- `src/app.rs` – die egui-Oberfläche; `src/main.rs` – Fenster und Start.
- `src/sprache.rs`, `src/en.tsv`, `src/thema.rs` – Sprache und Design wie in allen
  Werk-Programmen.

Tests der reinen Logik (nur std):

```
rustc --edition 2021 --test src/links.rs -o /tmp/links && /tmp/links
rustc --edition 2021 --test src/plan.rs -o /tmp/plan && /tmp/plan
rustc --edition 2021 --test src/store.rs -o /tmp/store && /tmp/store
rustc --edition 2021 --test src/sprache.rs -o /tmp/sprache && /tmp/sprache
```

## Hinweis zum Stand

Die reine Logik ist mit `rustc` getestet; der Download-Kern (`net.rs`) und das Entpacken über
7-Zip (`unpack.rs`) wurden zusätzlich gegen einen lokalen Testserver bzw. ein nachgebildetes
7-Zip durchgespielt (Fortsetzen nach Abbruch, Server ohne Range, Weiterleitung,
Content-Disposition, 404, falsche Prüfsumme, Grenze, Passwortliste). Gebaut mit `cargo`
werden konnte das Programm in dieser Umgebung nicht (kein Zugriff auf crates.io); die
Aufrufe folgen den dokumentierten APIs von egui/eframe 0.29, ureq 3, zip 2, sha2 0.10,
arboard 3 und rfd 0.14.
