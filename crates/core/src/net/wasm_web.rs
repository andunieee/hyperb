//! Remote wasm app sessions in the browser (`SPEC.md`, section 4).
//!
//! Native hosts run the module on wasmtime ([`super::wasm`]); in the page it
//! runs on the browser's own WebAssembly engine. The contract is the same:
//! the module exports `memory`, `alloc(len) -> ptr` and `receive(ptr, len)`,
//! and imports `hyperb.send(ptr, len)`.
//!
//! The glue is a few lines of JavaScript, because that is where a `WebAssembly`
//! instance can be called synchronously from the `hyperb.send` import.

use crate::net::Event;
use url::Url;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Largest module accepted when downloading, as on native hosts.
const MAX_MODULE: usize = 64 << 20;

#[wasm_bindgen(inline_js = r#"
export async function hyperb_wasm_open(bytes, on_line) {
  let instance;
  const imports = {
    hyperb: {
      send(ptr, len) {
        const memory = instance.exports.memory;
        const view = new Uint8Array(memory.buffer, ptr, len);
        on_line(new TextDecoder().decode(view));
      }
    }
  };
  const result = await WebAssembly.instantiate(bytes, imports);
  instance = result.instance ? result.instance : result;
  const exports = instance.exports;
  if (!(exports.memory instanceof WebAssembly.Memory)) {
    throw new Error("the module exports no memory");
  }
  if (typeof exports.alloc !== "function") {
    throw new Error("the module exports no alloc");
  }
  if (typeof exports.receive !== "function") {
    throw new Error("the module exports no receive");
  }
  return instance;
}

export function hyperb_wasm_send(instance, line) {
  const exports = instance.exports;
  const bytes = new TextEncoder().encode(line);
  const ptr = exports.alloc(bytes.length);
  new Uint8Array(exports.memory.buffer).set(bytes, ptr);
  exports.receive(ptr, bytes.length);
}
"#)]
extern "C" {
    #[wasm_bindgen(catch, js_name = hyperb_wasm_open)]
    async fn open_module(bytes: &[u8], on_line: &js_sys::Function) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, js_name = hyperb_wasm_send)]
    fn send_line(instance: &JsValue, line: &str) -> Result<(), JsValue>;
}

pub(super) async fn run(
    url: Url,
    out: async_channel::Receiver<String>,
    events: &async_channel::Sender<Event>,
) -> Result<(), String> {
    let bytes = load(&url).await?;
    let _ = events.send(Event::Open).await;

    let tx = events.clone();
    let on_line = Closure::<dyn FnMut(JsValue)>::new(move |line: JsValue| {
        if let Some(text) = line.as_string() {
            for line in text.lines().filter(|l| !l.is_empty()) {
                let _ = tx.try_send(Event::Data(format!("{line}\n")));
            }
        }
    });
    let instance = open_module(&bytes, on_line.as_ref().unchecked_ref())
        .await
        .map_err(|e| describe("wasm", e))?;

    while let Ok(line) = out.recv().await {
        send_line(&instance, &line).map_err(|e| describe("wasm", e))?;
    }
    Ok(())
}

async fn load(url: &Url) -> Result<Vec<u8>, String> {
    if let Some(bytes) = crate::cache::get(url).await {
        return Ok(bytes);
    }
    let resp = reqwest::get(url.clone())
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    if resp.content_length().is_some_and(|n| n > MAX_MODULE as u64) {
        return Err("the wasm app is too large".into());
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MODULE {
        return Err("the wasm app is too large".into());
    }
    crate::cache::put(url, &bytes).await;
    Ok(bytes.to_vec())
}

/// A JavaScript exception as a session error.
fn describe(what: &str, e: JsValue) -> String {
    let message = e
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"));
    format!("{what}: {message}")
}
