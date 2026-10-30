//! A wasm app session (`SPEC.md`, section 4), run on wasmtime.
//!
//! The module talks to the renderer through two functions, one on each side:
//!
//! - `receive(ptr: i32, len: i32)`, exported by the module: a line (or
//!   lines) from the renderer is at `ptr` in the module's exported `memory`.
//!   The module gets the buffer from its exported `alloc(len: i32) -> i32`
//!   and owns it from then on.
//! - `hyperb.send(ptr: i32, len: i32)`, imported by the module: a line (or
//!   lines) from the app, at `ptr` in its memory.
//!
//! Optionally, for apps that change on their own:
//!
//! - `hyperb.now() -> f64`, imported: the wall clock, in whole milliseconds
//!   since the Unix epoch.
//! - `hyperb.wake_in(ms: i32)`, imported: calls the module's exported
//!   `wake()` once, in `ms` milliseconds. It replaces the wake-up already
//!   pending, if any; a negative `ms` only cancels it.
//!
//! The module has no other imports: no files or network. Functions it
//! imports from other modules trap when called. It keeps no
//! identity or session, so lines reach it exactly as the renderer wrote them.

use crate::net::Event;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;
use wasmtime::{
    Caller, Config, Engine, Extern, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};

/// Largest linear memory a module may grow to.
const MAX_MEMORY: usize = 256 << 20;
/// Instructions (roughly) one call into the module may run.
const FUEL: u64 = 20_000_000_000;

struct Host {
    events: async_channel::Sender<Event>,
    limits: StoreLimits,
    /// When to call `wake`, as the module last asked with `wake_in`.
    wake: Option<Instant>,
}

/// Runs the app at `url`, or `module` if given.
pub(super) async fn run(
    url: Url,
    module: Option<Vec<u8>>,
    out: async_channel::Receiver<String>,
    events: &async_channel::Sender<Event>,
) -> Result<(), String> {
    let bytes = match module {
        Some(m) => m,
        None => load(&url).await?,
    };
    let _ = events.send(Event::Open).await;
    let events = events.clone();
    tokio::task::spawn_blocking(move || exec(&bytes, out, events))
        .await
        .map_err(|e| e.to_string())?
}

async fn load(url: &Url) -> Result<Vec<u8>, String> {
    if let Some(path) = crate::address::wasm_path(url) {
        return std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()));
    }
    super::download(url).await
}

fn exec(
    wasm: &[u8],
    out: async_channel::Receiver<String>,
    events: async_channel::Sender<Event>,
) -> Result<(), String> {
    let err = |e: wasmtime::Error| format!("wasm: {e:#}");
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).map_err(err)?;
    let module = Module::new(&engine, wasm).map_err(err)?;

    let mut linker = Linker::<Host>::new(&engine);
    linker
        .func_wrap(
            "hyperb",
            "send",
            |mut caller: Caller<'_, Host>, ptr: i32, len: i32| -> wasmtime::Result<()> {
                let mem = caller
                    .get_export("memory")
                    .and_then(Extern::into_memory)
                    .ok_or_else(|| wasmtime::format_err!("the module exports no memory"))?;
                let data = mem
                    .data(&caller)
                    .get(ptr as u32 as usize..)
                    .and_then(|m| m.get(..len as u32 as usize))
                    .ok_or_else(|| wasmtime::format_err!("send: out of bounds"))?;
                let text = String::from_utf8_lossy(data).into_owned();
                for line in text.lines().filter(|l| !l.is_empty()) {
                    // Closed means the renderer is gone; the loop below ends.
                    let _ = caller
                        .data()
                        .events
                        .send_blocking(Event::Data(format!("{line}\n")));
                }
                Ok(())
            },
        )
        .map_err(err)?;
    linker
        .func_wrap("hyperb", "now", |_: Caller<'_, Host>| -> f64 { now_ms() })
        .map_err(err)?;
    linker
        .func_wrap(
            "hyperb",
            "wake_in",
            |mut caller: Caller<'_, Host>, ms: i32| {
                caller.data_mut().wake = u64::try_from(ms)
                    .ok()
                    .map(|ms| Instant::now() + Duration::from_millis(ms));
            },
        )
        .map_err(err)?;

    // Toolchains such as wasm-bindgen leave imports the app never calls.
    linker
        .define_unknown_imports_as_traps(&module)
        .map_err(err)?;

    let limits = StoreLimitsBuilder::new().memory_size(MAX_MEMORY).build();
    let mut store = Store::new(
        &engine,
        Host {
            events,
            limits,
            wake: None,
        },
    );
    store.limiter(|h| &mut h.limits);
    store.set_fuel(FUEL).map_err(err)?;
    let instance = linker.instantiate(&mut store, &module).map_err(err)?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or("the module exports no memory")?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut store, "alloc")
        .map_err(err)?;
    let receive = instance
        .get_typed_func::<(i32, i32), ()>(&mut store, "receive")
        .map_err(err)?;
    let wake = instance.get_typed_func::<(), ()>(&mut store, "wake").ok();
    // This runs on a blocking thread of the network runtime, which can still
    // wait for a line until the wake-up is due.
    let rt = tokio::runtime::Handle::current();

    // Ends when the connection (the sender of `out`) is dropped.
    loop {
        let next = match store.data().wake {
            Some(at) => rt.block_on(async {
                tokio::time::timeout_at(at.into(), out.recv())
                    .await
                    .map(Result::ok)
            }),
            None => Ok(out.recv_blocking().ok()),
        };
        store.set_fuel(FUEL).map_err(err)?;
        let line = match next {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(_) => {
                store.data_mut().wake = None;
                let wake = wake
                    .as_ref()
                    .ok_or("wasm: wake_in was called, but the module exports no function 'wake'")?;
                wake.call(&mut store, ()).map_err(err)?;
                continue;
            }
        };
        let len = i32::try_from(line.len()).map_err(|_| "line too long")?;
        let ptr = alloc.call(&mut store, len).map_err(err)?;
        memory
            .write(&mut store, ptr as u32 as usize, line.as_bytes())
            .map_err(|e| format!("wasm: {e}"))?;
        receive.call(&mut store, (ptr, len)).map_err(err)?;
    }
    Ok(())
}

fn now_ms() -> f64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    since.as_millis() as f64
}

#[cfg(test)]
mod tests {
    use crate::address;
    use crate::identity::Identity;
    use crate::net::{Connection, Event};

    /// An app that answers every line with the same line.
    const ECHO: &str = r#"(module
        (import "hyperb" "send" (func $send (param i32 i32)))
        (memory (export "memory") 1)
        (global $top (mut i32) (i32.const 1024))
        (func (export "alloc") (param $n i32) (result i32)
            (global.get $top)
            (global.set $top (i32.add (global.get $top) (local.get $n))))
        (func (export "receive") (param $p i32) (param $n i32)
            (call $send (local.get $p) (local.get $n))))"#;

    fn open(name: &str, wat: &str) -> Connection {
        let path =
            std::env::temp_dir().join(format!("hyperb-test-{}-{name}.wasm", std::process::id()));
        std::fs::write(&path, wat).unwrap();
        let url = address::normalize(&format!("hyperb://{}", path.display())).unwrap();
        Connection::open(&url, &Identity::generate())
    }

    #[test]
    fn echo_app() {
        let conn = open("echo", ECHO);
        let events = conn.events();
        assert_eq!(events.recv_blocking().unwrap(), Event::Open);
        conn.send("hello {\"a\":1}\n".into());
        conn.send("action {}\n".into());
        assert_eq!(
            events.recv_blocking().unwrap(),
            Event::Data("hello {\"a\":1}\n".into())
        );
        assert_eq!(
            events.recv_blocking().unwrap(),
            Event::Data("action {}\n".into())
        );
    }

    /// An app that asks to be woken on every line, and says "woke" when it is.
    const ALARM: &str = r#"(module
        (import "hyperb" "send" (func $send (param i32 i32)))
        (import "hyperb" "now" (func $now (result f64)))
        (import "hyperb" "wake_in" (func $wake_in (param i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "woke\n")
        (func (export "alloc") (param $n i32) (result i32) (i32.const 1024))
        (func (export "receive") (param $p i32) (param $n i32)
            (if (f64.lt (call $now) (f64.const 1e12)) (then unreachable))
            (call $wake_in (i32.const 10)))
        (func (export "wake")
            (call $send (i32.const 0) (i32.const 5))))"#;

    #[test]
    fn wake_up() {
        let conn = open("alarm", ALARM);
        let events = conn.events();
        assert_eq!(events.recv_blocking().unwrap(), Event::Open);
        conn.send("hello {}\n".into());
        assert_eq!(
            events.recv_blocking().unwrap(),
            Event::Data("woke\n".into())
        );
        // Once: the app did not ask again.
        conn.send("action {}\n".into());
        assert_eq!(
            events.recv_blocking().unwrap(),
            Event::Data("woke\n".into())
        );
    }

    #[test]
    fn preloaded_module() {
        let url = address::normalize("hyperb://localhost:1/echo.wasm").unwrap();
        let conn = Connection::open_module(&url, ECHO.as_bytes().to_vec());
        let events = conn.events();
        assert_eq!(events.recv_blocking().unwrap(), Event::Open);
        conn.send("hello {}\n".into());
        assert_eq!(
            events.recv_blocking().unwrap(),
            Event::Data("hello {}\n".into())
        );
    }

    #[test]
    fn broken_module_closes_with_reason() {
        let conn = open("broken", "(module)");
        let events = conn.events();
        assert_eq!(events.recv_blocking().unwrap(), Event::Open);
        conn.send("hello {}\n".into());
        assert!(matches!(
            events.recv_blocking().unwrap(),
            Event::Closed(Some(_))
        ));
        let missing = Connection::open(
            &address::normalize("hyperb:///nonexistent/app.wasm").unwrap(),
            &Identity::generate(),
        );
        assert!(matches!(
            missing.events().recv_blocking().unwrap(),
            Event::Closed(Some(_))
        ));
    }
}
