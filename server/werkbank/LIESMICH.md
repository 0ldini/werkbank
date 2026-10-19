# Werkbank-Lizenzserver

Kleiner PHP-Server für jarns.ch: schaltet die Werkbank mit einem gemeinsamen Code frei und
zeigt, wie viele Geräte sie nutzen und welche Programme dort installiert sind.

## Einrichten (einmal)

1. Diesen Ordner nach `jarns.ch/werkbank/` hochladen (macht der Ablauf «jarns.ch» in GitHub
   Actions, sobald die FTP-Zugangsdaten hinterlegt sind – oder von Hand per FTP).
2. **`geheim.php` von Hand** in denselben Ordner legen. Die Datei enthält den geheimen Schlüssel
   für die Unterschrift der Lizenzen. Sie gehört **nicht** ins Git und wird nie weitergegeben.
   Geht sie verloren, müssen alle Programme mit einem neuen Schlüssel neu gebaut werden.
3. `data/` muss für PHP beschreibbar sein (Rechte 775, notfalls 777).
4. `https://jarns.ch/werkbank/` öffnen: Passwort für die Verwaltung und den Code festlegen.

Voraussetzung: PHP 7.4 oder neuer mit der Erweiterung **sodium** (bei den meisten Hostern
eingeschaltet). `https://jarns.ch/werkbank/api.php?status` zeigt, ob alles bereit ist.

## Wie es funktioniert

- Beim ersten Start fragt der Launcher nach dem Code und schickt ihn mit einer zufälligen
  Gerätekennung, dem Computernamen, Benutzer, System und den installierten Programmen an
  `api.php`. Stimmt der Code, bekommt das Gerät eine unterschriebene Lizenz (60 Tage gültig).
- Bei jedem Start mit Internet meldet sich der Launcher und erneuert die Lizenz. Gesperrte
  Geräte verlieren die Lizenz beim nächsten Start mit Internet, spätestens nach 60 Tagen.
- Ohne Lizenz starten weder Launcher noch eigene Programme; die ArtCraft-Programme laufen
  dann im Original (englisch, ohne Hell/Dunkel aus dem Launcher).
- «Code ändern» mit dem Haken «Alle Geräte müssen den neuen Code eingeben» schaltet alle ab,
  bis sie den neuen Code eingeben.

Daten: `data/einstellungen.json` (Passwort und Code nur als Hash), `data/geraete.json`.
