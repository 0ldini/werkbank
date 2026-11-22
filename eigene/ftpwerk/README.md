# FTPwerk

FTP-, FTPS- und SFTP-Client (wie FileZilla/Cyberduck), Teil der Werkbank. Rust + egui, nativ (Windows, Linux).
Keine Browser-Version: ein Browser darf keine rohen FTP-/SSH-Verbindungen öffnen.

## Was es kann (v0.1)

- Protokolle: FTP, FTPS (explizit und implizit), SFTP (SSH, Passwort oder Schlüsseldatei)
- Zwei Fenster: lokal links, Server rechts; Doppelklick, Ziehen zwischen den Seiten und Dateien von aussen ins Fenster ziehen
- Warteschlange mit parallelen Übertragungen, Pause, Abbrechen, Wiederholen, Fortsetzen abgebrochener Dateien, Geschwindigkeit und Restzeit
- Ordner rekursiv hoch-/herunterladen und löschen, Umbenennen, Neuer Ordner, Rechte ändern (chmod)
- Versteckte Dateien wie `.htaccess` werden angezeigt und mit übertragen (Server ohne MLSD: `LIST -a`)
- Verbindungsverwaltung (`verbindungen.txt`), Passwörter nur unter Windows verschlüsselt (DPAPI) gespeichert
- Zertifikate/Server-Schlüssel: Rückfrage mit Fingerabdruck, danach gemerkt (`vertraut.txt`); Änderung wird deutlich gemeldet
- Deutsch/Englisch und hell/dunkel (folgt der Werkbank-Einstellung)

## Sicherheit

- **FTPS**: Zertifikate von bekannten Stellen (Mozilla-Liste) werden normal geprüft. Alles andere
  (selbst signiert, falscher Name, abgelaufen) führt nie stillschweigend zur Verbindung: FTPwerk
  bricht ab und zeigt den SHA-256-Fingerabdruck. Erst nach «Vertrauen und verbinden» wird er für
  diesen Server (Name:Port) gespeichert. Ein anderer Fingerabdruck als der gespeicherte wird rot
  als «Server hat sich geändert» gemeldet.
- **SFTP**: genauso mit dem Server-Schlüssel («SHA256:…», wie `ssh-keygen -lf`).
- **Passwörter**: unter Windows auf Wunsch mit DPAPI verschlüsselt (nur dasselbe Windows-Konto kann
  sie lesen). Unter Linux werden Passwörter nie gespeichert; FTPwerk fragt bei jeder Verbindung.
- Einfaches FTP (unverschlüsselt) wird in der Verbindungsverwaltung als solches gewarnt.

## Noch nicht drin

Ordner abgleichen, aktiver FTP-Modus, Proxy, Bandbreitenlimit, Bearbeiten in externem Editor.

## Aufbau

`ftp.rs`, `sftp.rs`, `tls.rs` (Protokolle), `xfer.rs` (Warteschlange/Motor, Threads), `app.rs` + `dialogs.rs` (Oberfläche), `store.rs` (Dateien/Einstellungen), `model.rs`, `listparse.rs`;
`stil.rs`, `thema.rs`, `sprache.rs` sind die gemeinsamen Werkbank-Dateien (Vorlagen in `eigene/`).

## Prüfen

- `cargo test`: Einheitstests (Listen, Pfade, Vertrauen, Warteschlange, Übersetzungen) und
  `e2e.rs` gegen echte Server, wenn deren Angaben als Umgebungsvariablen gesetzt sind.
- Pipeline «FTPwerk-Test» (`.github/workflows/ftpwerk-test.yml`): unter Linux mit pyftpdlib (FTP,
  FTP ohne MLSD, FTPS mit selbst signiertem Zertifikat) und OpenSSH (SFTP mit Passwort und
  Schlüssel); unter Windows die Einheitstests.

Gebaut wird in der Werkbank-Pipeline («Eigene-Release», Tag `ftpwerk-vX.Y.Z`).
