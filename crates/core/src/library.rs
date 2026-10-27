//! What renderers remember between runs: the history of the apps the user
//! opened, their favorites, and the modules of favorite remote wasm apps, kept
//! so that they open without downloading them again (or without a network).
//!
//! [`Library`] is the data, as JSON. Where it is kept is up to the platform:
//! on native hosts [`Store`] keeps it in `<data dir>/library.json` and the
//! modules in `<data dir>/wasm/` (see [`Store::default_dir`]); the web
//! renderer uses `localStorage` and the origin private file system.
//!
//! Entries are keyed by their `hyperb://` address ([`address::hyperb`]), so
//! the same app typed two ways is one entry.

use crate::address::{self, Kind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

/// The most entries the history keeps; the oldest go first.
pub const HISTORY_LIMIT: usize = 500;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The `hyperb://` address.
    pub address: String,
    /// A short name for lists ([`address::label`]).
    pub title: String,
    /// When it was last opened (history) or added (favorites), in
    /// milliseconds since the Unix epoch.
    pub time: u64,
}

impl Entry {
    fn new(url: &Url, now: u64) -> Self {
        Entry {
            address: address::hyperb(url),
            title: address::label(url),
            time: now,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    /// Most recent first, one entry per address.
    #[serde(default)]
    pub history: Vec<Entry>,
    /// In the order they were added.
    #[serde(default)]
    pub favorites: Vec<Entry>,
    /// The `hyperb://` addresses that were open when the app last changed
    /// them, in tab order; renderers reopen them on startup unless they are
    /// started with an address.
    #[serde(default)]
    pub session: Vec<String>,
}

impl Library {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a library is always valid JSON")
    }

    /// Records that `url` was opened at `now`.
    pub fn visit(&mut self, url: &Url, now: u64) {
        let entry = Entry::new(url, now);
        self.history.retain(|e| e.address != entry.address);
        self.history.insert(0, entry);
        self.history.truncate(HISTORY_LIMIT);
    }

    /// Removes `address` from the history.
    pub fn forget(&mut self, address: &str) {
        self.history.retain(|e| e.address != address);
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    pub fn is_favorite(&self, url: &Url) -> bool {
        let address = address::hyperb(url);
        self.favorites.iter().any(|e| e.address == address)
    }

    /// Adds `url` to the favorites (at the end) or removes it.
    pub fn set_favorite(&mut self, url: &Url, on: bool, now: u64) {
        let entry = Entry::new(url, now);
        let pos = self
            .favorites
            .iter()
            .position(|e| e.address == entry.address);
        match (on, pos) {
            (true, None) => self.favorites.push(entry),
            (false, Some(i)) => {
                self.favorites.remove(i);
            }
            _ => {}
        }
    }

    /// Whether the module of `url` should be kept: it is a remote wasm app
    /// and a favorite runs it (with any parameters).
    pub fn keeps_module(&self, url: &Url) -> bool {
        let Some(file) = module_file(url) else {
            return false;
        };
        self.favorites.iter().any(|e| {
            address::normalize(&e.address)
                .ok()
                .and_then(|u| module_file(&u))
                .is_some_and(|f| f == file)
        })
    }

    /// The file names ([`module_file`]) of every module the library keeps.
    pub fn kept_modules(&self) -> Vec<String> {
        let mut files: Vec<String> = self
            .favorites
            .iter()
            .filter_map(|e| module_file(&address::normalize(&e.address).ok()?))
            .collect();
        files.sort();
        files.dedup();
        files
    }
}

/// The name a remote wasm app's module is kept under: the SHA-256 of its
/// address without the parameters (the fragment), so every set of parameters
/// of an app shares it, as hex with a `.wasm` extension. `None` for other sessions.
pub fn module_file(url: &Url) -> Option<String> {
    if address::kind(url) != Kind::RemoteWasm {
        return None;
    }
    let mut key = url.clone();
    key.set_fragment(None);
    let hash = Sha256::digest(key.as_str().as_bytes());
    Some(format!("{}.wasm", hex::encode(hash)))
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use super::{module_file, Library};
    use std::io;
    use std::path::{Path, PathBuf};
    use url::Url;

    const LIBRARY: &str = "library.json";
    const MODULES: &str = "wasm";

    /// A [`Library`] kept in a directory, with the modules it keeps.
    ///
    /// Every change reads the file again first, so several processes can
    /// share a directory without losing each other's changes.
    pub struct Store {
        dir: PathBuf,
        library: Library,
    }

    impl Store {
        /// The library in `dir`, which need not exist yet. A file that
        /// cannot be parsed is moved aside to `library.json.broken`.
        pub fn open(dir: impl Into<PathBuf>) -> Self {
            let dir = dir.into();
            let library = load(&dir);
            Store { dir, library }
        }

        /// `$XDG_DATA_HOME/hyperb`, falling back to `~/.local/share` (or
        /// `%APPDATA%` on Windows).
        pub fn default_dir() -> Option<PathBuf> {
            let var = |k| {
                std::env::var_os(k)
                    .filter(|v| !v.is_empty())
                    .map(PathBuf::from)
            };
            let base = if cfg!(windows) {
                var("APPDATA")
            } else {
                var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local/share")))
            };
            Some(base?.join("hyperb"))
        }

        pub fn dir(&self) -> &Path {
            &self.dir
        }

        pub fn library(&self) -> &Library {
            &self.library
        }

        /// Reads the library again, for changes made by other processes.
        pub fn reload(&mut self) {
            self.library = load(&self.dir);
        }

        /// Applies `f` to the library as it is on disk and saves it.
        pub fn update(&mut self, f: impl FnOnce(&mut Library, u64)) -> io::Result<()> {
            self.reload();
            f(&mut self.library, now());
            write_atomic(&self.dir.join(LIBRARY), self.library.to_json().as_bytes())
        }

        pub fn visit(&mut self, url: &Url) -> io::Result<()> {
            self.update(|l, now| l.visit(url, now))
        }

        /// Adds `url` to the favorites or removes it. Removing the last
        /// favorite that runs a module deletes the module; adding one does
        /// not download it (see [`Store::download`]).
        pub fn set_favorite(&mut self, url: &Url, on: bool) -> io::Result<()> {
            self.update(|l, now| l.set_favorite(url, on, now))?;
            self.prune()
        }

        /// The kept module of `url`, if the library keeps it and it has been
        /// downloaded.
        pub fn module(&self, url: &Url) -> Option<Vec<u8>> {
            if !self.library.keeps_module(url) {
                return None;
            }
            std::fs::read(self.dir.join(MODULES).join(module_file(url)?)).ok()
        }

        /// Deletes the modules the library no longer keeps.
        pub fn prune(&self) -> io::Result<()> {
            let keep = self.library.kept_modules();
            let entries = match std::fs::read_dir(self.dir.join(MODULES)) {
                Ok(e) => e,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                if !keep.iter().any(|k| name == k.as_str()) {
                    std::fs::remove_file(entry.path())?;
                }
            }
            Ok(())
        }
    }

    #[cfg(feature = "net")]
    impl Store {
        /// Opens a session at `url`, like [`crate::net::Connection::open`].
        /// A remote wasm app that the library keeps runs from its kept
        /// module when there is one, and the module is downloaded again in
        /// the background, so the next session gets any update.
        pub fn connect(
            &self,
            url: &Url,
            identity: &crate::identity::Identity,
        ) -> crate::net::Connection {
            use crate::net::Connection;
            if !self.library.keeps_module(url) {
                return Connection::open(url, identity);
            }
            #[cfg(feature = "wasm")]
            let conn = match self.module(url) {
                Some(m) => Connection::open_module(url, m),
                None => Connection::open(url, identity),
            };
            #[cfg(not(feature = "wasm"))]
            let conn = Connection::open(url, identity);
            self.download(url);
            conn
        }

        /// Downloads the module of `url` in the background, and keeps it if
        /// the library (as it is on disk by then) still keeps it. Failures
        /// are ignored: the app runs from the network until it succeeds.
        pub fn download(&self, url: &Url) {
            let (dir, url) = (self.dir.clone(), url.clone());
            crate::net::spawn(async move {
                let Ok(bytes) = crate::net::fetch_module(&url).await else {
                    return;
                };
                let Some(file) = module_file(&url) else {
                    return;
                };
                if load(&dir).keeps_module(&url) {
                    let _ = write_atomic(&dir.join(MODULES).join(file), &bytes);
                }
            });
        }
    }

    fn load(dir: &Path) -> Library {
        let path = dir.join(LIBRARY);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Library::default();
        };
        Library::from_json(&text).unwrap_or_else(|_| {
            let _ = std::fs::rename(&path, path.with_extension("json.broken"));
            Library::default()
        })
    }

    /// Writes through a temporary file, so readers never see half a file.
    fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, path)
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use store::Store;

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        address::normalize(s).unwrap()
    }

    #[test]
    fn history_is_most_recent_first_without_duplicates() {
        let mut l = Library::default();
        l.visit(&url("localhost:9001/a"), 1);
        l.visit(&url("localhost:9001/b"), 2);
        l.visit(&url("http://localhost:9001/a"), 3);
        let addrs: Vec<_> = l.history.iter().map(|e| e.address.as_str()).collect();
        assert_eq!(
            addrs,
            ["hyperb://localhost:9001/a", "hyperb://localhost:9001/b"]
        );
        assert_eq!(l.history[0].time, 3);
        assert_eq!(l.history[0].title, "localhost:9001/a");
        l.forget("hyperb://localhost:9001/a");
        assert_eq!(l.history.len(), 1);
        for i in 0..HISTORY_LIMIT + 10 {
            l.visit(&url(&format!("localhost/{i}")), i as u64);
        }
        assert_eq!(l.history.len(), HISTORY_LIMIT);
        l.clear_history();
        assert!(l.history.is_empty());
    }

    #[test]
    fn favorites() {
        let mut l = Library::default();
        let a = url("example.com/a");
        l.set_favorite(&a, true, 1);
        l.set_favorite(&a, true, 2);
        assert!(l.is_favorite(&url("hyperb://example.com/a")));
        assert_eq!(l.favorites.len(), 1);
        assert_eq!(l.favorites[0].time, 1);
        l.set_favorite(&a, false, 3);
        assert!(!l.is_favorite(&a));
    }

    #[test]
    fn modules() {
        let mut l = Library::default();
        let app = url("example.com/app.wasm#a=1");
        assert!(!l.keeps_module(&app));
        l.set_favorite(&app, true, 0);
        l.set_favorite(&url("example.com/server"), true, 0);
        l.set_favorite(&url("/opt/local.wasm"), true, 0);
        // Every set of parameters of the app shares the module; other sessions have none.
        assert!(l.keeps_module(&url("example.com/app.wasm")));
        assert!(!l.keeps_module(&url("example.com/other.wasm")));
        assert!(!l.keeps_module(&url("example.com/server")));
        assert!(!l.keeps_module(&url("/opt/local.wasm")));
        assert_eq!(l.kept_modules(), [module_file(&app).unwrap()]);
        assert_eq!(
            module_file(&app),
            module_file(&url("hyperb://example.com/app.wasm"))
        );
        assert_eq!(module_file(&url("/opt/local.wasm")), None);
    }

    #[test]
    fn json_round_trips_and_tolerates_missing_fields() {
        let mut l = Library::default();
        l.visit(&url("example.com"), 5);
        l.set_favorite(&url("example.com"), true, 6);
        assert_eq!(Library::from_json(&l.to_json()), Ok(l));
        assert_eq!(Library::from_json("{}"), Ok(Library::default()));
        assert!(Library::from_json("nope").is_err());
    }

    #[test]
    fn store() {
        let dir = std::env::temp_dir().join(format!("hyperb-lib-{}", std::process::id()));
        let app = url("example.com/app.wasm");
        let mut a = Store::open(&dir);
        let mut b = Store::open(&dir);
        a.visit(&url("example.com")).unwrap();
        b.set_favorite(&app, true).unwrap();
        // b's change re-read the file, so a's visit survives.
        assert_eq!(b.library().history.len(), 1);
        let file = dir.join("wasm").join(module_file(&app).unwrap());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"\0asm").unwrap();
        std::fs::write(dir.join("wasm").join("stale.wasm"), b"").unwrap();
        assert_eq!(b.module(&app).as_deref(), Some(&b"\0asm"[..]));
        b.prune().unwrap();
        assert!(file.exists());
        assert!(!dir.join("wasm").join("stale.wasm").exists());
        a.set_favorite(&app, false).unwrap();
        assert!(!file.exists());
        assert_eq!(a.module(&app), None);
        std::fs::write(dir.join("library.json"), "garbage").unwrap();
        assert_eq!(Store::open(&dir).library(), &Library::default());
        assert!(dir.join("library.json.broken").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
