# FTPwerk

FTP-, FTPS- und SFTP-Client (wie FileZilla/Cyberduck), Teil der Werkbank. Rust + egui, nativ (Windows, Linux).

## Was es kann (v0.1)

- Protokolle: FTP, FTPS (explizit und implizit), SFTP (SSH, Passwort oder Schlüsseldatei)
- Zwei Fenster: lokal links, Server rechts; Doppelklick, Ziehen zwischen den Seiten und Dateien von aussen ins Fenster ziehen
- Warteschlange mit parallelen Übertragungen, Pause, Abbrechen, Wiederholen, Fortsetzen abgebrochener Dateien, Geschwindigkeit und Restzeit
- Ordner rekursiv hoch-/herunterladen und löschen, Umbenennen, Neuer Ordner, Rechte ändern (chmod)
- Verbindungsverwaltung (`verbindungen.txt`), Passwörter nur unter Windows verschlüsselt (DPAPI) gespeichert
- Zertifikate/Server-Schlüssel: Rückfrage mit Fingerabdruck, danach gemerkt (`vertraut.txt`); Änderung wird deutlich gemeldet
- Deutsch/Englisch (folgt der Werkbank-Einstellung)

## Noch nicht drin

Ordner abgleichen, aktiver FTP-Modus, Proxy, Bandbreitenlimit, Bearbeiten in externem Editor.

## Aufbau

`ftp.rs`, `sftp.rs`, `tls.rs` (Protokolle), `xfer.rs` (Warteschlange/Motor, Threads), `app.rs` + `dialogs.rs` (Oberfläche), `store.rs` (Dateien/Einstellungen), `model.rs`, `listparse.rs`.

Gebaut wird in der Werkbank-Pipeline («Eigene-Release», Tag `ftpwerk-vX.Y.Z`).
