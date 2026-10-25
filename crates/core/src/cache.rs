//! Cache of downloaded wasm apps, kept only for favorites.
//!
//! Opening `https://…/app.wasm` downloads the module every time. Favorite
//! apps are stored instead: on native hosts under the directory set with
//! [`set_dir`], in the browser's origin private file system. Other apps are
//! never stored, and unfavoriting an app calls [`forget`] so its bytes go
//! away.
//!
//! Clients call [`set_favorites`] whenever the favorites change; that is what
//! makes [`put`] store a module and [`get`] use it.

use url::Url;

/// The cache key of `url`: the SHA-256 of the address, in hex.
pub fn key(url: &Url) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(url.as_str().as_bytes());
    hex::encode(digest)
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::key;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use url::Url;

    static DIR: OnceLock<PathBuf> = OnceLock::new();
    static FAVORITES: Mutex<Option<HashSet<String>>> = Mutex::new(None);

    /// Sets the directory modules are cached in, creating it if needed.
    pub fn set_dir(dir: PathBuf) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = DIR.set(dir);
    }

    pub fn set_favorites(urls: impl IntoIterator<Item = String>) {
        *FAVORITES.lock().unwrap() = Some(urls.into_iter().collect());
    }

    pub fn is_favorite(url: &Url) -> bool {
        FAVORITES
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|f| f.contains(url.as_str()))
    }

    fn path(url: &Url) -> Option<PathBuf> {
        Some(DIR.get()?.join(format!("{}.wasm", key(url))))
    }

    pub async fn get(url: &Url) -> Option<Vec<u8>> {
        if !is_favorite(url) {
            return None;
        }
        std::fs::read(path(url)?).ok()
    }

    pub async fn put(url: &Url, bytes: &[u8]) {
        if !is_favorite(url) {
            return;
        }
        let Some(path) = path(url) else { return };
        let _ = write_atomic(&path, bytes);
    }

    pub async fn forget(url: &Url) {
        if let Some(path) = path(url) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::key;
    use std::cell::RefCell;
    use std::collections::HashSet;
    use url::Url;
    use wasm_bindgen::prelude::*;

    thread_local! {
        static FAVORITES: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };
    }

    pub fn set_favorites(urls: impl IntoIterator<Item = String>) {
        FAVORITES.with(|f| *f.borrow_mut() = Some(urls.into_iter().collect()));
    }

    pub fn is_favorite(url: &Url) -> bool {
        FAVORITES.with(|f| {
            f.borrow()
                .as_ref()
                .is_some_and(|f| f.contains(url.as_str()))
        })
    }

    #[wasm_bindgen(inline_js = r#"
const hyperbCacheDir = async () => {
  const root = await navigator.storage.getDirectory();
  return root.getDirectoryHandle("hyperb-wasm", { create: true });
};

export async function hyperb_cache_get(key) {
  try {
    const dir = await hyperbCacheDir();
    const handle = await dir.getFileHandle(key);
    const file = await handle.getFile();
    return new Uint8Array(await file.arrayBuffer());
  } catch (e) {
    return null;
  }
}

export async function hyperb_cache_put(key, bytes) {
  try {
    const dir = await hyperbCacheDir();
    const handle = await dir.getFileHandle(key, { create: true });
    const writable = await handle.createWritable();
    await writable.write(bytes);
    await writable.close();
    return true;
  } catch (e) {
    return false;
  }
}

export async function hyperb_cache_forget(key) {
  try {
    const dir = await hyperbCacheDir();
    await dir.removeEntry(key);
    return true;
  } catch (e) {
    return false;
  }
}
"#)]
    extern "C" {
        #[wasm_bindgen(catch, js_name = hyperb_cache_get)]
        async fn cache_get(key: &str) -> Result<JsValue, JsValue>;
        #[wasm_bindgen(catch, js_name = hyperb_cache_put)]
        async fn cache_put(key: &str, bytes: &[u8]) -> Result<JsValue, JsValue>;
        #[wasm_bindgen(catch, js_name = hyperb_cache_forget)]
        async fn cache_forget(key: &str) -> Result<JsValue, JsValue>;
    }

    pub async fn get(url: &Url) -> Option<Vec<u8>> {
        if !is_favorite(url) {
            return None;
        }
        let value = cache_get(&key(url)).await.ok()?;
        if value.is_null() || value.is_undefined() {
            return None;
        }
        Some(js_sys::Uint8Array::new(&value).to_vec())
    }

    pub async fn put(url: &Url, bytes: &[u8]) {
        if !is_favorite(url) {
            return;
        }
        let _ = cache_put(&key(url), bytes).await;
    }

    pub async fn forget(url: &Url) {
        let _ = cache_forget(&key(url)).await;
    }
}

pub use imp::{forget, get, is_favorite, put, set_favorites};

#[cfg(not(target_arch = "wasm32"))]
pub use imp::set_dir;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_distinct_and_stable() {
        let a = Url::parse("https://x/a.wasm").unwrap();
        let b = Url::parse("https://x/b.wasm").unwrap();
        assert_eq!(key(&a), key(&a));
        assert_ne!(key(&a), key(&b));
        assert_eq!(key(&a).len(), 64);
    }
}
