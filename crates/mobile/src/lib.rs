//! JNI bridge loaded by the Kotlin Compose app as `libhyperb_mobile.so`,
//! behind `com.hyperb.app.RustBridge`.
//!
//! A session is a [`Connection`] and its [`Client`], handed to Kotlin as an
//! opaque handle. Kotlin waits for network events on a background thread
//! with `await`, then reads the whole state as JSON with `snapshot` and
//! renders it; user input goes back through `input`.
//!
//! Handle lifetime: `cancel` ends the session, which makes `await` return
//! false; the thread that calls `await` then calls `free`, and nothing may
//! use the handle after that.

use hyperb_core::address;
use hyperb_core::hypereffect::client::Client;
use hyperb_core::hypereffect::{json, serde_json, Map, Value};
use hyperb_core::identity::Identity;
use hyperb_core::net::{Connection, Event};
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jlong, jstring};
use jni::{Env, EnvUnowned};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use url::Url;

static IDENTITY: OnceLock<Identity> = OnceLock::new();

struct Session {
    events: async_channel::Receiver<Event>,
    inner: Mutex<Inner>,
}

struct Inner {
    client: Client,
    conn: Option<Connection>,
    url: Url,
    state: State,
}

enum State {
    Connecting,
    Open,
    Closed(Option<String>),
}

impl Session {
    fn handle(&self, ev: Event) {
        let mut s = self.inner.lock().unwrap();
        let s = &mut *s;
        match ev {
            Event::Open => {
                let hello = s.client.hello(Map::new());
                if let Some(c) = &s.conn {
                    c.send(hello);
                }
                s.state = State::Open;
            }
            Event::Data(lines) => s.client.receive(lines.as_bytes()),
            Event::Closed(reason) => {
                s.client.close();
                s.conn = None;
                s.state = State::Closed(reason);
            }
        }
    }
}

impl Inner {
    fn snapshot(&mut self) -> String {
        for d in self.client.take_diagnostics() {
            log::warn!("[{}] {d}", self.url);
        }
        let _ = self.client.take_changes();
        let mut tree = serde_json::to_value(self.client.tree()).unwrap_or_default();
        self.add_bound_values(&mut tree);
        let (state, reason) = match &self.state {
            State::Connecting => ("connecting", None),
            State::Open => ("open", None),
            State::Closed(r) => ("closed", r.clone()),
        };
        json!({
            "url": self.url.as_str(),
            "label": address::label(&self.url),
            "state": state,
            "reason": reason,
            "connected": self.client.is_connected(),
            "tree": tree,
        })
        .to_string()
    }

    /// Puts the value of each element's `bind` path in its `$value`
    /// attribute, so Kotlin never resolves signal paths itself.
    fn add_bound_values(&self, nodes: &mut Value) {
        let Some(nodes) = nodes.as_array_mut() else {
            return;
        };
        for n in nodes {
            if let Some(attrs) = n.get_mut("attrs").and_then(Value::as_object_mut) {
                if let Some(path) = attrs.get("bind").and_then(Value::as_str) {
                    let v = self.client.value(path).cloned().unwrap_or(Value::Null);
                    attrs.insert("$value".into(), v);
                }
            }
            if let Some(kids) = n.get_mut("children") {
                self.add_bound_values(kids);
            }
        }
    }

    /// User input on element `id`: writes `value` to its bound signal, if
    /// any, then runs the handler in attribute `event`, if any.
    fn input(&mut self, id: &str, value: Option<Value>, event: Option<&str>) {
        if let Some(v) = value {
            if let Some(Value::String(path)) = self.client.attr(id, "bind") {
                self.client.write(&path, v);
            }
        }
        if let Some(line) = event.and_then(|e| self.client.trigger(id, e)) {
            if let Some(c) = &self.conn {
                c.send(line);
            }
        }
    }
}

fn session<'a>(handle: jlong) -> &'a Session {
    // SAFETY: handles come from `Box::into_raw` in `open` and Kotlin uses
    // them only until it calls `free` (see the module docs).
    unsafe { &*(handle as *const Session) }
}

fn opt_string(env: &Env, s: &JString) -> jni::errors::Result<Option<String>> {
    if s.is_null() {
        Ok(None)
    } else {
        s.try_to_string(env).map(Some)
    }
}

/// `init(context: Context, filesDir: String): String`. Call once before
/// anything else; returns the identity's public key.
#[no_mangle]
pub extern "system" fn Java_com_hyperb_app_RustBridge_init(
    mut env: EnvUnowned<'_>,
    _class: JClass,
    context: JObject,
    files_dir: JString,
) -> jstring {
    env.with_env(|env| -> jni::errors::Result<jstring> {
        #[cfg(target_os = "android")]
        {
            android_logger::init_once(
                android_logger::Config::default()
                    .with_max_level(log::LevelFilter::Info)
                    .with_tag("hyperb"),
            );
            rustls_platform_verifier::android::init_with_env(env, context)?;
        }
        #[cfg(not(target_os = "android"))]
        let _ = context;
        let dir = files_dir.try_to_string(env)?;
        let id = IDENTITY.get_or_init(|| {
            Identity::load_or_create(&Path::new(&dir).join("identity")).unwrap_or_else(|e| {
                log::error!("cannot load identity ({e}); using a temporary one");
                Identity::generate()
            })
        });
        Ok(env.new_string(id.public().to_string())?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// `open(address: String): Long`. Throws if the address is invalid.
#[no_mangle]
pub extern "system" fn Java_com_hyperb_app_RustBridge_open(
    mut env: EnvUnowned<'_>,
    _class: JClass,
    input: JString,
) -> jlong {
    env.with_env(|env| -> jni::errors::Result<jlong> {
        let input = input.try_to_string(env)?;
        let url = match address::normalize(&input) {
            Ok(u) => u,
            Err(e) => {
                let _ = env.throw(format!("Invalid address: {e}"));
                return Ok(0);
            }
        };
        let identity = IDENTITY.get_or_init(Identity::generate);
        let conn = Connection::open(&url, identity);
        let s = Box::new(Session {
            events: conn.events(),
            inner: Mutex::new(Inner {
                client: Client::new(),
                conn: Some(conn),
                url,
                state: State::Connecting,
            }),
        });
        Ok(Box::into_raw(s) as jlong)
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// `cancel(handle: Long)`: ends the session, waking a pending `await`.
#[no_mangle]
pub extern "system" fn Java_com_hyperb_app_RustBridge_cancel(
    _env: EnvUnowned<'_>,
    _class: JClass,
    handle: jlong,
) {
    let mut s = session(handle).inner.lock().unwrap();
    if s.conn.take().is_some() {
        s.client.close();
        s.state = State::Closed(None);
    }
}

/// `free(handle: Long)`: releases a session once `await` returned false.
#[no_mangle]
pub extern "system" fn Java_com_hyperb_app_RustBridge_free(
    _env: EnvUnowned<'_>,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: see `session`; this is the last use of the handle.
        drop(unsafe { Box::from_raw(handle as *mut Session) });
    }
}

/// `await(handle: Long): Boolean`. Blocks until network events arrive and
/// applies them; false once the session is over and nothing more will come.
#[no_mangle]
pub extern "system" fn Java_com_hyperb_app_RustBridge_await(
    _env: EnvUnowned<'_>,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    let s = session(handle);
    let Ok(ev) = s.events.recv_blocking() else {
        return false;
    };
    s.handle(ev);
    while let Ok(ev) = s.events.try_recv() {
        s.handle(ev);
    }
    true
}

/// `snapshot(handle: Long): String`: the connection state and the resolved
/// tree, as JSON.
#[no_mangle]
pub extern "system" fn Java_com_hyperb_app_RustBridge_snapshot(
    mut env: EnvUnowned<'_>,
    _class: JClass,
    handle: jlong,
) -> jstring {
    env.with_env(|env| -> jni::errors::Result<jstring> {
        let json = session(handle).inner.lock().unwrap().snapshot();
        Ok(env.new_string(json)?.into_raw())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// `input(handle: Long, id: String, value: String?, event: String?)`, with
/// `value` as JSON.
#[no_mangle]
pub extern "system" fn Java_com_hyperb_app_RustBridge_input(
    mut env: EnvUnowned<'_>,
    _class: JClass,
    handle: jlong,
    id: JString,
    value: JString,
    event: JString,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let id = id.try_to_string(env)?;
        let value = opt_string(env, &value)?.and_then(|v| serde_json::from_str(&v).ok());
        let event = opt_string(env, &event)?;
        session(handle)
            .inner
            .lock()
            .unwrap()
            .input(&id, value, event.as_deref());
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}
