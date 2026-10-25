# Schutzwerk

Sicherheits-Werkzeug für die Werkbank – **rein defensiv**. Hilft, den eigenen
Rechner zu schützen: vor Spionage- und Schadsoftware, vor Fernzugriff und vor
Missbrauch der eigenen Rechenleistung (z. B. versteckte Krypto-Miner). Dazu
Passwörter prüfen, offene Ports und Geräte im eigenen Netz sehen, den
Schutzstatus kontrollieren und gelöschte Dateien aus dem Papierkorb zurückholen.
**Keine Angriffs-, Pentest-, Auslese- oder Schnüffel-Funktionen.** Alles ist
Lesen des eigenen Rechners; Eingriffe gibt es nur auf Wunsch, mit Rückfrage und
rückgängig machbar.

> **Ehrlicher Hinweis:** Für die Virenerkennung nutzt Schutzwerk die Engine des
> Systems – Microsoft Defender unter Windows, ClamAV unter macOS/Linux – und hat
> **keine eigenen Virensignaturen**. Es ergänzt eigene Überwachung (Autostart,
> Ressourcen, Verbindungen, Kamera/Mikrofon, Fernzugriff). Eine vollständige
> Sicherheits-Suite ersetzt es nicht.

Rust + egui, passend zu Codewerk, Umbenennwerk, Speicherwerk, Aufnahmewerk und
Wandelwerk. Hauptsächlich für Windows; unter macOS und Linux läuft es ebenfalls,
was dort nicht geht, ist mit «unter diesem System nicht verfügbar» markiert.

## Bereiche

- **Passwort-Check** – prüft die Stärke **vollständig offline**: Länge,
  Zeichenklassen, Entropie, Muster (Wiederholungen, Reihen, Tastatur-Reihen),
  Abgleich mit bekannten Passwörtern und geschätzte Knackzeit. Nichts wird
  gesendet oder gespeichert.
- **Netzwerk** – lauschende Ports des **eigenen** Rechners mit Prozess und
  Risiko-Hinweis (z. B. RDP/SMB offen) sowie die Geräte im eigenen Netz
  (aus der ARP-Tabelle). Nur Lesen, keine Fremd-Scans.
- **System-Schutz** – Status von Firewall, Virenschutz (Defender-Echtzeit),
  Windows-Update und SmartScreen, mit einer groben Gesamtpunktzahl.
- **Papierkorb** – gelöschte Dateien auflisten (ursprünglicher Pfad, Grösse,
  Löschzeitpunkt) und einzeln an ihren Ursprungsort zurückholen.
- **Virenscan** – Windows: Microsoft Defender über `MpCmdRun.exe` (Schnellscan,
  vollständiger Scan, Ordner/Datei scannen, Signaturen aktualisieren), mit
  laufender Ausgabe, Abbrechen und Ergebnis; Defender-Status (Echtzeitschutz,
  Alter der Signaturen, letzte Scans, Betriebsart, Manipulationsschutz) und der
  Verlauf erkannter Bedrohungen (`Get-MpThreatDetection`/`Get-MpThreat`, nur
  Lesen). macOS/Linux: Ordner mit `clamscan` prüfen und `freshclam` starten;
  fehlt ClamAV, steht da, wie man es installiert.
- **Ressourcen-Wächter** – CPU (über 3 s gemessen), Speicher, Programmdatei und
  unter Windows die GPU-Last je Prozess. Markiert bekannte Miner-Namen,
  Programme aus Temp/Downloads/öffentlichen Ordnern (AppData als Hinweis),
  nachgeahmte Systemnamen (svchost.exe ausserhalb von System32, svch0st.exe …)
  und hohe Last, während niemand am Rechner ist. Signaturprüfung
  (Authenticode bzw. codesign) nur auf Knopfdruck für Gewählte/Auffällige.
  Optional eine **Hintergrund-Wache**, solange das Fenster offen ist: Warnleiste
  oben, wenn ein Programm z. B. 2 Minuten über 80 % liegt (einstellbar).
- **Autostart** – Windows: Run/RunOnce (HKCU/HKLM, auch WOW6432Node),
  Autostart-Ordner, geplante Aufgaben, automatisch startende Dienste ausserhalb
  von C:\Windows; macOS: LaunchAgents/LaunchDaemons; Linux: ~/.config/autostart,
  systemd-Benutzer-Units, crontab. Mit Herausgeber/Signatur, Markierung **neuer**
  Einträge seit dem bekannten Stand und – unter Windows, nach Rückfrage –
  «Deaktivieren» (Run-Wert erst sichern, dann entfernen; Aufgabe mit
  `schtasks /change /disable`) sowie «Wiederherstellen». Dateien werden nie
  gelöscht.
- **Verbindungen** – hergestellte TCP-Verbindungen nach aussen je Programm
  (`netstat -ano`, macOS `lsof`, Linux `ss`), mit Hinweisen auf ungewöhnliche
  Orte, fehlende Signatur und typische Mining-Ports. Keine Namensauflösung
  (wäre langsam und würde selbst Anfragen senden).
- **Kamera & Mikrofon** – Windows: welche Apps wann zugegriffen haben (aus dem
  Datenschutzprotokoll `CapabilityAccessManager\ConsentStore`), **«gerade aktiv»**
  gross angezeigt. Linux: welche eigenen Programme die Geräte gerade offen
  haben. macOS: nicht verfügbar (Hinweis auf die Systemeinstellungen).
- **Fernzugriff** – laufende oder installierte Fernwartungs-Programme
  (TeamViewer, AnyDesk, RustDesk, VNC, ScreenConnect …), Remotedesktop und
  Remoteunterstützung (Windows) bzw. SSH-Server, mit Erklärung und Knopf zur
  passenden Systemeinstellung. Schutzwerk schaltet nichts selbst um.
- **Übersicht** – Gesamtpunktzahl aus allen geprüften Bereichen, je Befund
  «Was heisst das? Was kann ich tun?» – sachlich, ohne Alarmismus.

## Bauen und starten

```
cargo run --release
```

Fertige Datei danach unter `target\release\schutzwerk.exe`.

Abhängigkeiten: egui/eframe, `sysinfo` (Prozesse), `serde`/`serde_json`
(bekannter Autostart-Stand und Sicherungen) und unter Windows `winreg`
(Registry lesen) und `windows-sys`. Die System-Abfragen nutzen bordeigene
Befehle (`netstat`, `arp`, `tasklist`, `netsh`, `powershell`, `schtasks`,
`MpCmdRun.exe`; macOS/Linux `lsof`, `ss`, `plutil`, `codesign`, `clamscan`) –
ohne Konsolenfenster und in Hintergrund-Threads. Es wird nichts nachinstalliert.
Für den vollständigen Papierkorb aller Laufwerke und für Eingriffe unter HKLM
hilft ein Start als Administrator.

Gespeichert wird nur im Einstellungsordner der Werkbank (Windows
`%LOCALAPPDATA%\Werkbank\config\schutzwerk`): `autostart-grundlage.json`
(bekannter Stand) und `autostart-sicherung.json` (deaktivierte Einträge).

## Aufbau

- `src/pwcheck.rs` – Passwort-Analyse (offline). **Eigenständig getestet.**
- `src/netparse.rs` – Parser für `netstat`/`arp` + Port-Risiko. **Getestet.**
- `src/recycle.rs` – Papierkorb-Datensatz ($I) binär lesen + Datum. **Getestet.**
- `src/scanparse.rs` – Ausgaben von MpCmdRun, Defender-Status/-Verlauf, clamscan. **Getestet.**
- `src/prozparse.rs` – Einstufung von Prozessen (Ort, Nachahmung, Miner,
  Fernzugriff), GPU-Last, Signatur-Ausgaben, Dauerlast-Wache. **Getestet.**
- `src/autoparse.rs` – Befehlszeilen, Umgebungsvariablen, crontab, .desktop,
  plist, systemd, Abgleich mit dem bekannten Stand. **Getestet.**
- `src/sys.rs` – ruft die Befehle auf, PowerShell, Einstellungsordner,
  Signaturen, liest den Papierkorb und stellt Dateien wieder her.
- `src/scan.rs` – Defender/ClamAV-Scans als Hintergrund-Prozess.
- `src/waechter.rs` – Prozess-Messung (sysinfo), Leerlauf, Hintergrund-Wache.
- `src/autostart.rs` – Autostart sammeln, Grundlage, Deaktivieren/Wiederherstellen.
- `src/zugriff.rs` – Verbindungen, Kamera & Mikrofon, Fernzugriff.
- `src/app.rs` – die egui-Oberfläche (Rahmen, ältere Bereiche, Übersicht).
- `src/seiten.rs` – die Seiten der neuen Bereiche und die Befunde der Übersicht.
- `src/main.rs` – Fenster und Start.

## Hinweis zum Stand

Die Parser (`netparse`, `scanparse`, `prozparse`, `autoparse`, `recycle`,
`pwcheck`) sind eigenständig mit `rustc --edition 2021 --test src/<datei>.rs`
gegen Beispielausgaben geprüft. Gebaut wird über GitHub Actions für Windows,
Linux und macOS. Die Oberfläche und die echten Abfragen (Defender-Scans,
Registry, PowerShell) konnten beim Entwickeln nicht auf einem echten Rechner
ausprobiert werden – bitte melden, wenn etwas nicht wie erwartet erscheint.
