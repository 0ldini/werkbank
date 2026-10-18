//! Ordner-Rollen (Posteingang, Gesendet, …). Nur Standardbibliothek.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    Flagged,
    Other,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Inbox => "inbox",
            Role::Drafts => "drafts",
            Role::Sent => "sent",
            Role::Archive => "archive",
            Role::Junk => "junk",
            Role::Trash => "trash",
            Role::Flagged => "flagged",
            Role::Other => "other",
        }
    }

    pub fn from_str(s: &str) -> Role {
        match s {
            "inbox" => Role::Inbox,
            "drafts" => Role::Drafts,
            "sent" => Role::Sent,
            "archive" => Role::Archive,
            "junk" => Role::Junk,
            "trash" => Role::Trash,
            "flagged" => Role::Flagged,
            _ => Role::Other,
        }
    }

    /// Reihenfolge in der Seitenleiste (kleiner = weiter oben).
    pub fn sort_key(self) -> u8 {
        match self {
            Role::Inbox => 0,
            Role::Flagged => 1,
            Role::Drafts => 2,
            Role::Sent => 3,
            Role::Archive => 4,
            Role::Junk => 5,
            Role::Trash => 6,
            Role::Other => 7,
        }
    }

    /// Deutscher Anzeigename für Standardordner.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Role::Inbox => Some("Posteingang"),
            Role::Drafts => Some("Entwürfe"),
            Role::Sent => Some("Gesendet"),
            Role::Archive => Some("Archiv"),
            Role::Junk => Some("Spam"),
            Role::Trash => Some("Papierkorb"),
            Role::Flagged => Some("Markiert"),
            Role::Other => None,
        }
    }
}

/// Rolle aus IMAP-Sonderkennzeichen (`\Sent` …) oder, ersatzweise, aus dem Namen.
pub fn guess_role(name: &str, flags: &[String]) -> Role {
    for f in flags {
        match f.to_ascii_lowercase().as_str() {
            "\\inbox" => return Role::Inbox,
            "\\sent" => return Role::Sent,
            "\\drafts" => return Role::Drafts,
            "\\trash" => return Role::Trash,
            "\\junk" => return Role::Junk,
            "\\archive" | "\\all" => return Role::Archive,
            "\\flagged" => return Role::Flagged,
            _ => {}
        }
    }
    let last = name.rsplit(|c| c == '/' || c == '.').next().unwrap_or(name).trim().to_lowercase();
    if name.eq_ignore_ascii_case("inbox") {
        return Role::Inbox;
    }
    match last.as_str() {
        "sent" | "sent items" | "sent mail" | "gesendet" | "gesendete elemente" | "gesendete objekte" => Role::Sent,
        "drafts" | "draft" | "entwürfe" | "entwurf" | "entwuerfe" => Role::Drafts,
        "trash" | "deleted items" | "deleted messages" | "papierkorb" | "gelöschte elemente" | "gelöschte objekte" => Role::Trash,
        "junk" | "spam" | "junk e-mail" | "junk-e-mail" | "bulk mail" => Role::Junk,
        "archive" | "archiv" | "archives" | "all mail" => Role::Archive,
        _ => Role::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollen() {
        assert_eq!(guess_role("INBOX", &[]), Role::Inbox);
        assert_eq!(guess_role("Gesendet", &[]), Role::Sent);
        assert_eq!(guess_role("INBOX.Papierkorb", &[]), Role::Trash);
        assert_eq!(guess_role("[Gmail]/Alle Nachrichten", &["\\All".to_string()]), Role::Archive);
        assert_eq!(guess_role("Irgendwas", &["\\Sent".to_string()]), Role::Sent);
        assert_eq!(guess_role("Kunden", &[]), Role::Other);
        assert_eq!(Role::from_str(Role::Junk.as_str()), Role::Junk);
    }
}
