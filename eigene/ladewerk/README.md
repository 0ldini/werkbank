# Ladewerk

Download-Manager im Stil von JDownloader – Teil der Werkbank, in Rust mit egui.

## Was es kann

- **Linksammler:** beliebigen Text einfügen (oder **Strg+V** im Fenster ohne aktives
  Textfeld) – die Links werden herausgesucht und geprüft (Name, Grösse, erreichbar). Pakete
  werden zu Unterordnern; Archive bekommen automatisch ein eigenes Paket. «Seiten nach Dateien
  durchsuchen» sammelt alle Datei-Links einer Webseite.
- **Prüfsumme:** steht in derselben Zeile hinter einem Link eine SHA-256-Prüfsumme
  (64 Zeichen), wird sie übernommen und nach dem Download geprüft. Bei falscher Prüfsumme wird
  nicht entpackt. Im Detailfenster (Klick auf den Namen) lässt sie sich nachtragen; «Jetzt
  prüfen» berechnet sie für jede fertige Datei.
- **Click'n'Load:** wie bei JDownloader auf Port 9666, auch verschlüsselt. Die Links landen
  im Linksammler, mitgeschickte Passwörter beim Paket. JDownloader muss dafür geschlossen sein.
- **Zwischenablage:** kopierte Datei- und Hoster-Links landen automatisch im Linksammler.
- **Downloads:** mehrere gleichzeitig, bis zu 8 Verbindungen pro Datei, Pause/Fortsetzen,
  Tempolimit, automatische Wiederholung; geht nach einem Neustart weiter.
- **Entpacken:** ZIP, RAR, 7z, tar – auch mehrteilig. Passwörter je Paket (🔑 im Paketkopf)
  und eine allgemeine Liste. Die Windows-Version bringt 7-Zip eingebaut mit.
- **Hoster-Konten:** Rapidgator (Premium) über die offizielle Schnittstelle. Das Passwort liegt
  unter Windows mit DPAPI verschlüsselt auf dem Computer.
- **Proxy:** in den Einstellungen (`http://…`, `socks5://…`), sonst aus der Umgebung
  (`HTTPS_PROXY`, `ALL_PROXY`).
- Deutsch/Englisch und hell/dunkel nach Wahl im Werkbank-Launcher.

Die Übertragung (inkl. HTTPS) erledigt das `curl`, das Windows 10/11, macOS und Linux
mitbringen. Daten und Einstellungen liegen im Ordner `.ladewerk` im Benutzerordner.

## Starten

- `ladewerk` – eigenes Fenster
- `ladewerk --browser` – Oberfläche im Standardbrowser (z. B. wenn das Fenster mangels
  OpenGL nicht aufgeht); mit `--no-open` ohne Browser, `--port=9000` für einen anderen Port

## Bauen und Testen

`cargo build --release`. Für das eingebaute 7-Zip vorher `KIND=windows ./werk-prebuild.sh`
(holt 7z.exe/7z.dll aus den offiziellen 7-Zip-Veröffentlichungen und setzt `WERK_7Z_DIR`).
Der Ablauf «Ladewerk-Test» (`.github/workflows/ladewerk-test.yml`) baut und prüft Ladewerk
unter Windows x64/ARM64 und Linux mit `test/windows-test.ps1`.

## 7-Zip

Die Windows-Version enthält `7z.exe` und `7z.dll` von Igor Pavlov (7-zip.org) unter der GNU
LGPL mit der unRAR-Einschränkung (RAR-Archive dürfen damit entpackt, nicht erstellt werden).
Der Lizenztext liegt als `LICENSE-7zip.txt` bei.
