//! History and favorites, shared by every client.
//!
//! The lists are plain data: each client decides where to keep them (a file,
//! `localStorage`, shared preferences) and serializes the whole [`Library`] as
//! JSON with [`Library::to_json`] / [`Library::from_json`]. Entries are kept
//! newest first and a URL appears at most once in each list.

use serde::{Deserialize, Serialize};

/// How many recently opened addresses are kept.
pub const HISTORY_LIMIT: usize = 100;

/// One address in the history or in the favorites.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The normalized address, e.g. `https://demo.hypereffect.dev/restaurant`.
    pub url: String,
    /// A short label for lists, normally [`crate::address::label`].
    pub label: String,
    /// When it was last opened (history) or starred (favorites), in Unix
    /// milliseconds.
    pub at: u64,
    /// How many times it was opened.
    #[serde(default)]
    pub visits: u32,
}

impl Entry {
    fn new(url: &str, label: &str, at: u64, visits: u32) -> Entry {
        Entry {
            url: url.into(),
            label: label.into(),
            at,
            visits,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub history: Vec<Entry>,
    #[serde(default)]
    pub favorites: Vec<Entry>,
}

impl Library {
    /// Records that `url` was opened: bumps it to the front of the history and
    /// counts the visit.
    pub fn record(&mut self, url: &str, label: &str, now: u64) {
        if let Some(i) = self.history.iter().position(|e| e.url == url) {
            let mut entry = self.history.remove(i);
            entry.at = now;
            entry.visits = entry.visits.saturating_add(1);
            entry.label = label.into();
            self.history.insert(0, entry);
        } else {
            self.history.insert(0, Entry::new(url, label, now, 1));
        }
        self.history.truncate(HISTORY_LIMIT);
    }

    /// Adds `url` to the favorites; returns whether it was not there already.
    pub fn favorite(&mut self, url: &str, label: &str, now: u64) -> bool {
        if self.is_favorite(url) {
            return false;
        }
        self.favorites.insert(0, Entry::new(url, label, now, 0));
        true
    }

    /// Removes `url` from the favorites; returns whether it was there.
    pub fn unfavorite(&mut self, url: &str) -> bool {
        let before = self.favorites.len();
        self.favorites.retain(|e| e.url != url);
        before != self.favorites.len()
    }

    /// Adds or removes `url`; returns whether it is a favorite afterwards.
    pub fn toggle_favorite(&mut self, url: &str, label: &str, now: u64) -> bool {
        if self.unfavorite(url) {
            false
        } else {
            self.favorite(url, label, now);
            true
        }
    }

    pub fn is_favorite(&self, url: &str) -> bool {
        self.favorites.iter().any(|e| e.url == url)
    }

    /// The favorites' addresses, for the wasm cache.
    pub fn favorite_urls(&self) -> Vec<String> {
        self.favorites.iter().map(|e| e.url.clone()).collect()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Parses a stored library; anything unreadable yields an empty one.
    pub fn from_json(text: &str) -> Library {
        serde_json::from_str(text).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_newest_first_and_counted() {
        let mut lib = Library::default();
        lib.record("a", "A", 1);
        lib.record("b", "B", 2);
        lib.record("a", "A2", 3);
        assert_eq!(lib.history.len(), 2);
        assert_eq!(lib.history[0].url, "a");
        assert_eq!(lib.history[0].visits, 2);
        assert_eq!(lib.history[0].label, "A2");
        assert_eq!(lib.history[1].url, "b");
    }

    #[test]
    fn history_is_bounded() {
        let mut lib = Library::default();
        for i in 0..(HISTORY_LIMIT + 10) {
            lib.record(&format!("u{i}"), "", i as u64);
        }
        assert_eq!(lib.history.len(), HISTORY_LIMIT);
        assert_eq!(lib.history[0].url, format!("u{}", HISTORY_LIMIT + 9));
    }

    #[test]
    fn favorites_toggle() {
        let mut lib = Library::default();
        assert!(lib.toggle_favorite("a", "A", 1));
        assert!(lib.is_favorite("a"));
        assert!(!lib.toggle_favorite("a", "A", 2));
        assert!(!lib.is_favorite("a"));
        assert!(!lib.unfavorite("a"));
    }

    #[test]
    fn round_trips_json() {
        let mut lib = Library::default();
        lib.record("https://x/a.wasm", "a.wasm", 7);
        lib.favorite("https://x/a.wasm", "a.wasm", 8);
        let back = Library::from_json(&lib.to_json());
        assert_eq!(back, lib);
        assert_eq!(Library::from_json("not json"), Library::default());
    }
}
