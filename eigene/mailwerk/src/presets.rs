//! Server-Vorschläge nach E-Mail-Domain. Nur Standardbibliothek.

use crate::types::{Account, Kind, Security};

pub struct Hint {
    pub note: &'static str,
}

/// Füllt die Serverfelder passend zur Adresse aus. Gibt einen Hinweis zurück, falls der Anbieter etwas Besonderes verlangt.
pub fn apply(acc: &mut Account) -> Option<Hint> {
    let domain = acc.email.split('@').nth(1)?.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return None;
    }
    let set = |a: &mut Account, imap: &str, smtp: &str, port: u16, sec: Security| {
        a.kind = Kind::Imap;
        a.imap_host = imap.into();
        a.imap_port = 993;
        a.smtp_host = smtp.into();
        a.smtp_port = port;
        a.smtp_security = sec;
    };
    match domain.as_str() {
        "gmail.com" | "googlemail.com" => {
            set(acc, "imap.gmail.com", "smtp.gmail.com", 465, Security::Tls);
            Some(Hint { note: "Gmail verlangt ein App-Passwort: In deinem Google-Konto die Bestätigung in zwei Schritten einschalten und dann ein App-Passwort erstellen." })
        }
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" | "outlook.ch" => {
            set(acc, "outlook.office365.com", "smtp-mail.outlook.com", 587, Security::StartTls);
            Some(Hint { note: "Microsoft hat die Anmeldung mit Passwort bei vielen Konten abgeschaltet. Falls es nicht klappt, ein App-Passwort erstellen." })
        }
        "icloud.com" | "me.com" | "mac.com" => {
            set(acc, "imap.mail.me.com", "smtp.mail.me.com", 587, Security::StartTls);
            Some(Hint { note: "iCloud verlangt ein App-spezifisches Passwort (Apple-ID → Anmeldung und Sicherheit)." })
        }
        "bluewin.ch" => {
            set(acc, "imaps.bluewin.ch", "smtpauths.bluewin.ch", 465, Security::Tls);
            None
        }
        "gmx.ch" | "gmx.net" | "gmx.de" | "gmx.at" => {
            set(acc, "imap.gmx.net", "mail.gmx.net", 465, Security::Tls);
            None
        }
        "web.de" => {
            set(acc, "imap.web.de", "smtp.web.de", 587, Security::StartTls);
            None
        }
        "yahoo.com" | "yahoo.de" => {
            set(acc, "imap.mail.yahoo.com", "smtp.mail.yahoo.com", 465, Security::Tls);
            Some(Hint { note: "Yahoo verlangt ein App-Passwort." })
        }
        "proton.me" | "protonmail.com" | "pm.me" => Some(Hint { note: "Proton Mail bietet keinen direkten Zugriff. Dafür wäre die «Proton Mail Bridge» nötig (Server 127.0.0.1)." }),
        "fastmail.com" | "fastmail.fm" | "messagingengine.com" => {
            acc.kind = Kind::Jmap;
            acc.jmap_url = "https://api.fastmail.com/jmap/session".into();
            Some(Hint { note: "Fastmail: Unter Einstellungen → Passwort & Sicherheit → Zugangsschlüssel einen Schlüssel mit Mail-Zugriff erstellen und hier als Passwort eintragen. Benutzername leer lassen." })
        }
        d => {
            set(acc, &format!("imap.{d}"), &format!("smtp.{d}"), 465, Security::Tls);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vorschlaege() {
        let mut a = Account::blank(1);
        a.email = "x@gmail.com".into();
        assert!(apply(&mut a).is_some());
        assert_eq!(a.imap_host, "imap.gmail.com");
        a.email = "x@outlook.com".into();
        apply(&mut a);
        assert_eq!((a.smtp_port, a.smtp_security), (587, Security::StartTls));
        a.email = "x@fastmail.com".into();
        apply(&mut a);
        assert_eq!(a.kind, Kind::Jmap);
        a.email = "x@meinefirma.ch".into();
        apply(&mut a);
        assert_eq!(a.kind, Kind::Imap);
        assert_eq!(a.imap_host, "imap.meinefirma.ch");
        a.email = "kaputt".into();
        assert!(apply(&mut a).is_none());
    }
}
