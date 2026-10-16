# Schutzwerk

Sicherheits-Werkzeug für die Werkbank – **rein defensiv**. Hilft, den eigenen
Rechner sicherer zu machen: Passwörter prüfen, offene Ports und Geräte im
eigenen Netz sehen, den Schutzstatus kontrollieren und gelöschte Dateien aus dem
Papierkorb zurückholen. **Keine Angriffs-, Pentest- oder Auslese-Funktionen.**

Rust + egui, passend zu Codewerk, Umbenennwerk, Speicherwerk, Aufnahmewerk und
Wandelwerk. Für Windows.

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

## Bauen und starten

```
cargo run --release
```

Fertige Datei danach unter `target\release\schutzwerk.exe`.

Nur egui/eframe als Abhängigkeit. Die System-Abfragen nutzen bordeigene
Windows-Befehle (`netstat`, `arp`, `tasklist`, `netsh`, `powershell`) – es wird
nichts nachinstalliert. Für den vollständigen Papierkorb aller Laufwerke hilft
ein Start als Administrator.

## Aufbau

- `src/pwcheck.rs` – Passwort-Analyse (offline). **Eigenständig getestet.**
- `src/netparse.rs` – Parser für `netstat`/`arp` + Port-Risiko. **Getestet.**
- `src/recycle.rs` – Papierkorb-Datensatz ($I) binär lesen + Datum. **Getestet.**
- `src/sys.rs` – ruft die Windows-Befehle auf, liest den Papierkorb und stellt
  Dateien wieder her (nur Lesen/Status, plus das vom Nutzer ausgelöste
  Wiederherstellen).
- `src/app.rs` – die egui-Oberfläche.
- `src/main.rs` – Fenster und Start.

## Hinweis zum Stand

Die Analyse- und Parser-Logik (`pwcheck`, `netparse`, `recycle`) ist hier mit
`rustc` gegen echte Fälle geprüft. Oberfläche und Systemschicht sind gegen die
dokumentierten APIs von egui 0.29 geschrieben, liessen sich in meiner Umgebung
aber nicht bauen (kein Zugriff auf crates.io). Falls `cargo` beim ersten Bauen
meckert, schick mir die Meldung.
