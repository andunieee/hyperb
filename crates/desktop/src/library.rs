//! History and favorites on disk, shared by every tab.
//!
//! The lists live in `$XDG_DATA_HOME/hyperb/library.json`; the wasm cache of
//! favorite apps is configured here too.

use gtk::glib;
use gtk4 as gtk;
use hyperb_core::cache;
use hyperb_core::library::Library;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use url::Url;

pub struct Store {
    path: PathBuf,
    library: RefCell<Library>,
}

impl Store {
    /// Loads the stored library and points the wasm cache at this host's
    /// cache directory.
    pub fn load() -> Rc<Store> {
        cache::set_dir(cache_dir());
        let path = data_dir().join("library.json");
        let library = std::fs::read_to_string(&path)
            .map(|text| Library::from_json(&text))
            .unwrap_or_default();
        cache::set_favorites(library.favorite_urls());
        Rc::new(Store {
            path,
            library: RefCell::new(library),
        })
    }

    pub fn with<R>(&self, f: impl FnOnce(&Library) -> R) -> R {
        f(&self.library.borrow())
    }

    /// Changes the library, saves it, and updates the cache: unfavorited
    /// modules are deleted.
    pub fn update(&self, f: impl FnOnce(&mut Library)) {
        let before = self.library.borrow().favorite_urls();
        {
            let mut library = self.library.borrow_mut();
            f(&mut library);
        }
        let after = self.library.borrow().favorite_urls();
        self.save();
        cache::set_favorites(after.clone());
        for url in before.iter().filter(|url| !after.contains(url)) {
            let Ok(url) = Url::parse(url) else { continue };
            glib::spawn_future_local(async move { cache::forget(&url).await });
        }
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.path, self.library.borrow().to_json());
    }
}

fn base(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_default();
            PathBuf::from(home).join(fallback)
        })
}

pub fn data_dir() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share").join("hyperb")
}

/// Where desktop entries live, so `hyperb://` links can find the app.
pub fn applications_dir() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share").join("applications")
}

pub fn cache_dir() -> PathBuf {
    base("XDG_CACHE_HOME", ".cache").join("hyperb/wasm")
}
