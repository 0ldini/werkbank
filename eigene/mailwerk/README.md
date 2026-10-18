# Mailwerk

E-Mail-Programm für die Werkbank (wie Thunderbird/Outlook, aber klein): drei Spalten
(Konten und Ordner, Nachrichtenliste, Lesebereich), Antworten, Allen antworten,
Weiterleiten, Anhänge, Markieren, Archivieren, Löschen, Suche und mehrere Konten.
Reines Rust mit **egui/eframe**. Läuft auf Windows und Linux und im Browser.
**Kein Server dazwischen:** Das Programm redet direkt mit dem Postfach-Anbieter.

## Was wo geht

| Konto-Art | Desktop (Windows/Linux) | Browser |
|---|---|---|
| **Normales Postfach (IMAP/SMTP)**: Bluewin, GMX, Gmail, iCloud, eigener Server … | ja | **nein**: Browser dürfen keine IMAP/SMTP-Verbindung öffnen. Das Konto zeigt dort einen Hinweis «nur Desktop». |
| **JMAP** (z. B. Fastmail) | ja | ja (sofern der Anbieter Zugriffe von fremden Webseiten erlaubt, siehe unten) |
| **Beispielkonto** (Übungsmails, ohne Netz) | ja | ja |

Im Browser wird nichts über einen Mailwerk-Server geleitet, weil es keinen gibt.
Dafür sind dort nur Anbieter mit JMAP möglich.

## Konto einrichten

«Konto hinzufügen», Adresse eingeben, «Server aus der Adresse vorschlagen», Passwort
eintragen, speichern. Einige Anbieter verlangen ein **App-Passwort** statt des normalen
Passworts (Gmail, iCloud, Yahoo, teils Microsoft); der Vorschlag zeigt den passenden
Hinweis. Fastmail: einen «Zugangsschlüssel» mit Mail-Zugriff erstellen und als Passwort
eintragen (Benutzername leer).

## Wichtig zu wissen

- **Passwörter werden unverschlüsselt gespeichert**, lokal auf dem Gerät (Desktop: im
  Programmordner des Benutzers, Browser: im Browser-Speicher dieser Seite). Das ist ein
  bekannter Mangel von Version 0.1; ein Schlüsselbund (Windows-Anmeldeinformationen,
  libsecret) ist die nächste Verbesserung. Bis dahin nur auf eigenen, gesperrten Geräten
  verwenden und für Mailwerk wenn möglich App-Passwörter nehmen.
- Es werden nur **Nachrichten als Text** angezeigt (HTML-Mails werden in Text umgewandelt,
  Bilder und Fernzugriffe gibt es nicht; das schützt vor Tracking).
- Die Verbindung ist immer verschlüsselt (TLS). Unverschlüsseltes IMAP (Port 143) wird
  nicht angeboten.
- Pro Befehl wird eine eigene Verbindung aufgebaut. Das ist einfach und robust, aber
  nicht so schnell wie bei Programmen mit dauernd offener Verbindung.
- Aktualisiert wird alle 5 Minuten und per Knopf. Es gibt kein Push.

## Noch nicht enthalten

Entwürfe speichern, Labels/Stichwörter, Mehrfachauswahl, Unterhaltungsansicht,
HTML-Anzeige, Anmeldung per OAuth (Gmail/Outlook ohne App-Passwort), Anhänge beim Senden
über JMAP, Kalender und Adressbuch, Schlüsselbund.

## Browser und CORS

Damit eine Webseite einen JMAP-Server direkt ansprechen darf, muss der Anbieter das
erlauben (CORS). Ob das bei deinem Anbieter so ist, ist nicht für alle geprüft. Falls die
Anmeldung im Browser mit «Verbindung fehlgeschlagen» endet, bitte die Desktop-Version
nehmen.

## Bauen

Voraussetzung: Rust (`https://rustup.rs`).

    cargo run --release              # Desktop
    trunk build --release            # Browser (rustup target add wasm32-unknown-unknown)

Bei Linux: die üblichen egui-Pakete (`libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev`).

Bei einem Push nach `eigene/mailwerk/**` baut die Aktion «Eigene-Release» alle drei
Versionen und veröffentlicht sie als `mailwerk-v<Version>`.

## Aufbau

Die Protokolle sind eigener Code ohne Fremdbibliotheken und haben Tests
(`b64`, `textutil`, `timeutil`, `mime`, `imap`, `smtp`, `json`, `jmap_proto`, `types`,
`store`, `demo`, `presets`). Dazu kommen die Teile mit Netzwerk und Oberfläche:

| Datei | Aufgabe |
|---|---|
| `imap.rs`, `smtp.rs` | IMAP- und SMTP-Client über einen beliebigen Datenstrom |
| `mime.rs`, `textutil.rs` | Mails lesen und bauen (Kopfzeilen, Zeichensätze, Anhänge) |
| `jmap_proto.rs`, `jmap.rs` | JMAP-Anfragen/-Antworten, HTTP über `ehttp` (Browser und Desktop) |
| `native.rs` | Desktop: TLS (`rustls`) und die Abläufe für IMAP/SMTP |
| `backend.rs` | verteilt Befehle an das richtige Konto-Verfahren |
| `demo.rs` | Beispielkonto |
| `store.rs`, `types.rs` | Zustand, Zwischenspeicher, Speicherung als JSON |
| `app.rs` | Oberfläche |

Tests der Kerne ohne Cargo: die Module sind nur von der Standardbibliothek abhängig
(`rustc --edition 2021 --test` mit einer kleinen Hauptdatei, die die Module einbindet).
Weil die Module ihre Meldungen über `sprache.rs` übersetzen, muss diese Hauptdatei auch
`mod sprache;` einbinden (`sprache.rs` liest `en.tsv` aus demselben Ordner). `json`, `b64`,
`role` und `timeutil` lassen sich weiterhin einzeln mit `rustc --edition 2021 --test src/<datei>.rs`
prüfen.

## Sprache

Deutsch oder Englisch, wie im Werkbank-Launcher gewählt (`WERKBANK_LANG` bzw. `sprache.txt`,
siehe `src/sprache.rs`); im Browser immer Deutsch. Die englischen Texte stehen in `src/en.tsv`.
Prüfen, ob alles übersetzt ist: `rustc --edition 2021 --test src/sprache.rs -o /tmp/sprache && /tmp/sprache`.
Der Inhalt von Mails (auch im Beispielkonto) wird nicht übersetzt.
