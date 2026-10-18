//! Lokaler Zustand als JSON-Text: Konten, Zwischenspeicher, Auswahl.
//! Nativ und im Browser wird derselbe Text über eframe-Persistenz abgelegt.

use crate::json::{self, Json};
use crate::types::{AccCache, Account};

#[derive(Clone, Debug, Default)]
pub struct Saved {
    pub accounts: Vec<Account>,
    pub caches: Vec<(u64, AccCache)>,
    /// Zuletzt gewähltes Konto.
    pub selected: u64,
    pub next_id: u64,
}

impl Saved {
    pub fn new_id(&mut self) -> u64 {
        let used = self.accounts.iter().map(|a| a.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(used) + 1;
        self.next_id
    }

    pub fn to_json_string(&self) -> String {
        json::obj(vec![
            ("v", json::n(1)),
            ("selected", json::n(self.selected as i64)),
            ("next_id", json::n(self.next_id as i64)),
            ("accounts", Json::Arr(self.accounts.iter().map(|a| a.to_json()).collect())),
            (
                "caches",
                Json::Arr(
                    self.caches
                        .iter()
                        .map(|(id, c)| json::obj(vec![("id", json::n(*id as i64)), ("c", c.to_json())]))
                        .collect(),
                ),
            ),
        ])
        .to_string()
    }

    /// Defekter oder leerer Text ergibt einen leeren Zustand (nie einen Absturz).
    pub fn from_json_str(t: &str) -> Saved {
        let Ok(j) = Json::parse(t) else { return Saved::default() };
        let accounts: Vec<Account> = j.get("accounts").as_arr().iter().filter_map(Account::from_json).collect();
        let caches = j
            .get("caches")
            .as_arr()
            .iter()
            .filter_map(|e| Some((e.get("id").as_i64()? as u64, AccCache::from_json(e.get("c")))))
            .filter(|(id, _)| accounts.iter().any(|a| a.id == *id))
            .collect();
        Saved {
            selected: j.get("selected").as_i64().unwrap_or(0).max(0) as u64,
            next_id: j.get("next_id").as_i64().unwrap_or(0).max(0) as u64,
            accounts,
            caches,
        }
    }

    pub fn cache_mut(&mut self, id: u64) -> &mut AccCache {
        if let Some(i) = self.caches.iter().position(|(a, _)| *a == id) {
            return &mut self.caches[i].1;
        }
        self.caches.push((id, AccCache::default()));
        &mut self.caches.last_mut().unwrap().1
    }

    pub fn cache(&self, id: u64) -> Option<&AccCache> {
        self.caches.iter().find(|(a, _)| *a == id).map(|(_, c)| c)
    }

    pub fn remove_account(&mut self, id: u64) {
        self.accounts.retain(|a| a.id != id);
        self.caches.retain(|(a, _)| *a != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rundreise() {
        let mut s = Saved::default();
        let id = s.new_id();
        s.accounts.push(Account::demo(id));
        s.selected = id;
        s.cache_mut(id).folder_id = "INBOX".into();
        let id2 = s.new_id();
        assert_ne!(id, id2);
        s.cache_mut(id2).folder_id = "weg".into(); // Konto fehlt → fällt beim Lesen weg
        let t = s.to_json_string();
        let r = Saved::from_json_str(&t);
        assert_eq!(r.accounts.len(), 1);
        assert_eq!(r.selected, id);
        assert_eq!(r.cache(id).unwrap().folder_id, "INBOX");
        assert!(r.cache(id2).is_none());
        assert!(Saved::from_json_str("kaputt {").accounts.is_empty());
        assert!(Saved::from_json_str("").accounts.is_empty());
    }
}
