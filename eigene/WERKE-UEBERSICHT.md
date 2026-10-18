# Werke – eigene Rust-Programme für die Werkbank

Elf eigene Werkzeuge, alle in Rust + egui (nativ, Windows), passend zu den
ArtCraft-/Werk-Apps. Jeder Ordner ist ein eigenständiges Cargo-Projekt
(`cargo run --release`); Details stehen im jeweiligen README.

1. **codewerk** – QR-Codes erstellen, inkl. Swiss QR Bill
2. **umbenennwerk** – Dateien stapelweise umbenennen
3. **speicherwerk** – Speicherplatz analysieren (Baum, grösste Dateien, Dateitypen)
4. **aufnahmewerk** – Bildschirm aufnehmen (Vollbild/Fenster/Bereich, System-Ton,
   Webcam, Cursor/Klicks, Countdown, Hotkey → MP4/H.264). Mikrofon-Mischung ist
   als Etappe 2 vorgesehen; braucht ffmpeg-nahe Teile bzw. windows-capture 2.0.1.
5. **wandelwerk** – Konverter für Ton, Bild & Video (Bilder in Rust, Ton/Video
   über ffmpeg; Seitenverhältnis 1:1/16:9/… pro Datei)
6. **schutzwerk** – Sicherheit, rein defensiv (Passwort-Check offline, Netzwerk,
   System-Schutz, Papierkorb). Keine Angriffs-/Auslese-Funktionen.
7. **mediathekwerk** – Filme & Serien über TMDB (Suche, Sammlung mit Status &
   eigener Bewertung). Braucht einen kostenlosen TMDB-API-Schlüssel.
8. **diawerk** – Diashows als Video aus Fotos und Drohnenvideos (Panoramen mit
   Schwenk, Übergänge, Musik als MP3, Vor-/Abspann; Formate wie Wandelwerk)
9. **mailwerk** – E-Mail-Programm (IMAP/SMTP am Desktop, JMAP auch im Browser, kein
   Server dazwischen; Beispielkonto zum Ausprobieren)
10. **organiwerk** – Organigramme und Diagramme (Karten, Verbinder, automatisches
   Anordnen als Baum, SVG-Export; auch im Browser)
11. **ftpwerk** – FTP-, FTPS- und SFTP-Client (zwei Fenster, Warteschlange, Fortsetzen,
   rekursive Ordner, Rechte; nur nativ)

## Stand

Pro Projekt sind die fehleranfälligen Rechen-/Parser-Kerne eigenständig mit
`rustc` gegen echte Fälle geprüft; die Oberflächen sind gegen egui 0.29
geschrieben. Gebaut (und damit endgültig verifiziert) werden sie auf einem
Windows-Rechner mit Internetzugang (crates.io). Falls `cargo` bei einem Tool
meckert: Meldung zurückgeben, dann wird es punktgenau angepasst.

## Externe Abhängigkeiten

- **aufnahmewerk**: windows-capture 2.0.1, nokhwa (Webcam), windows-Crate
- **wandelwerk**: image-Crate; für Ton/Video ein `ffmpeg.exe` (daneben legen
  oder auf dem PATH)
- **diawerk**: rfd; ffmpeg wird beim Release-Build eingebettet (sonst neben die
  .exe legen oder auf den PATH)
- **mediathekwerk**: egui_extras (Poster), ureq, serde; TMDB-API-Schlüssel
- **mailwerk**: ehttp (JMAP), rustls + webpki-roots (nur Desktop), rfd; IMAP, SMTP, MIME, JSON
  sind eigener Code
- **ftpwerk**: rfd, rustls + webpki-roots + sha2 (FTPS), russh + russh-sftp + tokio (SFTP)
- die übrigen: nur egui/eframe (schutzwerk nutzt bordeigene Windows-Befehle)
