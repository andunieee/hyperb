//! Where the page keeps its [`Library`]: history and favorites as JSON in
//! `localStorage`, and the modules of favorite wasm apps in the origin
//! private file system, under `wasm/` with the names of
//! [`library::module_file`].
//!
//! Every change reads the stored library again first, so pages open in
//! several tabs do not undo each other's changes.

use hyperb_core::library::{self, Library};
use hyperb_core::net;
use js_sys::{Reflect, Uint8Array};
use leptos::logging::warn;
use leptos::prelude::window;
use leptos::task::spawn_local;
use url::Url;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetDirectoryOptions,
    FileSystemGetFileOptions, FileSystemWritableFileStream,
};

const LIBRARY_KEY: &str = "hyperb.library";
const MODULES: &str = "wasm";

/// The stored library, or an empty one.
pub fn load() -> Library {
    window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(LIBRARY_KEY).ok().flatten())
        .and_then(|j| Library::from_json(&j).ok())
        .unwrap_or_default()
}

/// Applies `f` to the stored library, saves it and returns it.
pub fn update(f: impl FnOnce(&mut Library, u64)) -> Library {
    let mut lib = load();
    f(&mut lib, js_sys::Date::now() as u64);
    let saved = window()
        .local_storage()
        .ok()
        .flatten()
        .is_some_and(|s| s.set_item(LIBRARY_KEY, &lib.to_json()).is_ok());
    if !saved {
        warn!("cannot save the history and favorites");
    }
    lib
}

/// The kept module of `url`, if it has been downloaded.
pub async fn module(url: &Url) -> Option<Vec<u8>> {
    let name = library::module_file(url)?;
    let dir = modules(false).await.ok()?;
    let file: FileSystemFileHandle = JsFuture::from(dir.get_file_handle(&name))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    let blob: Blob = JsFuture::from(file.get_file())
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    let buf = JsFuture::from(blob.array_buffer()).await.ok()?;
    Some(Uint8Array::new(&buf).to_vec())
}

/// Downloads the module of `url` in the background, and keeps it if the
/// library (as stored by then) still keeps it. Failures are ignored: the app
/// runs from the network until it succeeds.
pub fn download(url: Url) {
    spawn_local(async move {
        let Ok(bytes) = net::fetch_module(&url).await else {
            return;
        };
        let Some(name) = library::module_file(&url) else {
            return;
        };
        if load().keeps_module(&url)
            && let Err(e) = write(&name, &bytes).await
        {
            warn!("cannot keep {url}: {e:?}");
        }
    });
}

/// Deletes the modules `lib` no longer keeps.
pub async fn prune(lib: Library) {
    let keep = lib.kept_modules();
    let Ok(dir) = modules(false).await else {
        return;
    };
    // Collect first: removing entries while iterating is undefined.
    let mut names = Vec::new();
    let keys = dir.keys();
    loop {
        let Ok(next) = keys.next() else { break };
        let Ok(step) = JsFuture::from(next).await else {
            break;
        };
        if Reflect::get(&step, &"done".into())
            .ok()
            .and_then(|d| d.as_bool())
            != Some(false)
        {
            break;
        }
        if let Some(name) = Reflect::get(&step, &"value".into())
            .ok()
            .and_then(|v| v.as_string())
        {
            names.push(name);
        }
    }
    for name in names.into_iter().filter(|n| !keep.contains(n)) {
        let _ = JsFuture::from(dir.remove_entry(&name)).await;
    }
}

async fn modules(create: bool) -> Result<FileSystemDirectoryHandle, JsValue> {
    let root: FileSystemDirectoryHandle =
        JsFuture::from(window().navigator().storage().get_directory())
            .await?
            .dyn_into()?;
    let opts = FileSystemGetDirectoryOptions::new();
    opts.set_create(create);
    JsFuture::from(root.get_directory_handle_with_options(MODULES, &opts))
        .await?
        .dyn_into()
}

async fn write(name: &str, bytes: &[u8]) -> Result<(), JsValue> {
    let dir = modules(true).await?;
    let opts = FileSystemGetFileOptions::new();
    opts.set_create(true);
    let file: FileSystemFileHandle = JsFuture::from(dir.get_file_handle_with_options(name, &opts))
        .await?
        .dyn_into()?;
    // The new contents replace the file only when the stream closes.
    let stream: FileSystemWritableFileStream =
        JsFuture::from(file.create_writable()).await?.dyn_into()?;
    JsFuture::from(stream.write_with_u8_array(bytes)?).await?;
    JsFuture::from(stream.close()).await?;
    Ok(())
}
