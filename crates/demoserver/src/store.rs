//! In-memory profiles, keyed by the renderer's public key. Sessions end when
//! their event stream closes, but a profile stays for [`IDLE_TTL`] since its
//! last use, so a returning visitor with the same key is recognized again.

use hyperb_core::identity::PublicKey;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a profile survives without activity.
pub const IDLE_TTL: Duration = Duration::from_secs(2 * 60 * 60);

struct Entry<T> {
    value: T,
    seen: Instant,
}

/// A map from keys to remembered values, thread-safe, with idle expiry.
pub struct Memory<T>(Mutex<HashMap<PublicKey, Entry<T>>>);

impl<T> Default for Memory<T> {
    fn default() -> Self {
        Memory(Mutex::new(HashMap::new()))
    }
}

impl<T: Default> Memory<T> {
    /// A new, empty memory.
    pub fn new() -> Self {
        Memory(Mutex::new(HashMap::new()))
    }

    /// Drops entries idle longer than [`IDLE_TTL`].
    pub fn prune(&self) {
        let mut map = self.0.lock().unwrap();
        let cutoff = Instant::now() - IDLE_TTL;
        map.retain(|_, e| e.seen > cutoff);
    }

    /// Runs `f` with the entry for `key`, creating it first if needed, and
    /// refreshes the entry's idle timer. Returns `f`'s result.
    pub fn update<R>(&self, key: &PublicKey, f: impl FnOnce(&mut T) -> R) -> R {
        let mut map = self.0.lock().unwrap();
        let entry = map.entry(*key).or_insert_with(|| Entry {
            value: T::default(),
            seen: Instant::now(),
        });
        entry.seen = Instant::now();
        f(&mut entry.value)
    }
}

impl<T: Default + Clone> Memory<T> {
    /// A snapshot of the profile for `key`, or the default if unknown.
    pub fn snapshot(&self, key: &PublicKey) -> T {
        self.update(key, |p| p.clone())
    }
}
