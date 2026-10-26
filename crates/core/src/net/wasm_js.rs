//! A wasm app session (`SPEC.md`, section 4) in the browser, run on its own
//! WebAssembly engine instead of wasmtime. The interface is the same as in
//! `wasm.rs`, with these differences:
//!
//! - Only remote apps (and modules handed over with
//!   [`super::Connection::open_module`]): a page cannot read local files.
//! - Only the binary format.
//! - No fuel or memory limits: the app runs on the page's thread, so one that
//!   never returns freezes the page.

use crate::address::{self, Kind};
use crate::net::Event;
use js_sys::{Function, Object, Reflect, Uint8Array, WebAssembly};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use url::Url;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// Runs the app at `url`, or `module` if given.
pub(super) async fn run(
    url: Url,
    module: Option<Vec<u8>>,
    out: async_channel::Receiver<String>,
    events: &async_channel::Sender<Event>,
) -> Result<(), String> {
    let bytes = match module {
        Some(m) => m,
        None if address::kind(&url) == Kind::LocalWasm => {
            return Err("the browser cannot open local apps".into())
        }
        None => super::download(&url).await?,
    };
    let _ = events.send(Event::Open).await;

    // `send` is called from inside the module, so it cannot throw; it
    // records the error and the loop below ends the session.
    let memory: Rc<RefCell<Option<WebAssembly::Memory>>> = Rc::default();
    let failed: Rc<Cell<Option<String>>> = Rc::default();
    let send = {
        let (memory, failed, events) = (memory.clone(), failed.clone(), events.clone());
        Closure::<dyn FnMut(i32, i32)>::new(move |ptr: i32, len: i32| {
            let Some(mem) = &*memory.borrow() else {
                failed.set(Some("send: called before the module started".into()));
                return;
            };
            let Some(data) = read(mem, ptr as u32, len as u32) else {
                failed.set(Some("send: out of bounds".into()));
                return;
            };
            let text = String::from_utf8_lossy(&data);
            for line in text.lines().filter(|l| !l.is_empty()) {
                // The channel is unbounded, so this only fails once closed.
                let _ = events.try_send(Event::Data(format!("{line}\n")));
            }
        })
    };
    let hyperb = Object::new();
    Reflect::set(&hyperb, &"send".into(), send.as_ref()).map_err(js_err)?;
    let imports = Object::new();
    Reflect::set(&imports, &"hyperb".into(), &hyperb).map_err(js_err)?;

    let result = JsFuture::from(WebAssembly::instantiate_buffer(&bytes, &imports))
        .await
        .map_err(js_err)?;
    let instance: WebAssembly::Instance = Reflect::get(&result, &"instance".into())
        .map_err(js_err)?
        .dyn_into()
        .map_err(js_err)?;
    let exports = instance.exports();
    let export = |name: &str| Reflect::get(&exports, &name.into()).ok();
    let mem: WebAssembly::Memory = export("memory")
        .and_then(|m| m.dyn_into().ok())
        .ok_or("the module exports no memory")?;
    let func = |name: &str| -> Result<Function, String> {
        export(name)
            .and_then(|f| f.dyn_into().ok())
            .ok_or_else(|| format!("wasm: the module exports no function '{name}'"))
    };
    let (alloc, receive) = (func("alloc")?, func("receive")?);
    *memory.borrow_mut() = Some(mem.clone());

    // Ends when the connection (the sender of `out`) is dropped.
    while let Ok(line) = out.recv().await {
        let len = i32::try_from(line.len()).map_err(|_| "line too long")?;
        let ptr = alloc
            .call1(&JsValue::UNDEFINED, &len.into())
            .map_err(js_err)?
            .as_f64()
            .ok_or("wasm: alloc did not return a pointer")? as i32;
        // `alloc` may have grown the memory, which replaces its buffer.
        let buf = Uint8Array::new(&mem.buffer());
        let (start, end) = (ptr as u32, ptr as u32 as u64 + len as u64);
        if end > buf.length() as u64 {
            return Err("wasm: alloc returned a buffer out of bounds".into());
        }
        buf.subarray(start, end as u32).copy_from(line.as_bytes());
        receive
            .call2(&JsValue::UNDEFINED, &ptr.into(), &len.into())
            .map_err(js_err)?;
        if let Some(e) = failed.take() {
            return Err(format!("wasm: {e}"));
        }
    }
    drop(send);
    Ok(())
}

fn read(mem: &WebAssembly::Memory, ptr: u32, len: u32) -> Option<Vec<u8>> {
    let buf = Uint8Array::new(&mem.buffer());
    let end = ptr.checked_add(len)?;
    (end <= buf.length()).then(|| buf.subarray(ptr, end).to_vec())
}

fn js_err(e: JsValue) -> String {
    let msg = match e.dyn_ref::<js_sys::Error>() {
        Some(err) => String::from(err.message()),
        None => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    };
    format!("wasm: {msg}")
}
