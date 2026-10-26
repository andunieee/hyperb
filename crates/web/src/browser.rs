//! The browser in the page: an address bar, a connection to a hypereffect
//! server, and the rendered session. Runs only in the browser (feature
//! `hydrate`); the server renders a placeholder.
//!
//! Entering an address opens a connection (HTTP with SSE, over fetch); when it
//! opens the page sends `hello` and from then on feeds every message to the
//! [`Client`]. When the transport closes the last state stays on screen with
//! controls disabled, and a banner offers to reconnect.
//!
//! The identity is kept in `localStorage`. `?url=<address>` opens an address.

use crate::render::Children;
use hyperb_core::address;
use hyperb_core::cache;
use hyperb_core::hypereffect::client::Client;
use hyperb_core::hypereffect::{Component, Map, Value};
use hyperb_core::identity::Identity;
use hyperb_core::library::Library;
use hyperb_core::net::{Connection, Event};
use leptos::ev;
use leptos::logging::warn;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_meta::Title;
use std::collections::HashMap;
use url::Url;
use wasm_bindgen::prelude::*;

const IDENTITY_KEY: &str = "hyperb.identity";
const LIBRARY_KEY: &str = "hyperb.library";

/// Registers `web+hyperb://` links with the browser: they open this page with
/// `?url=<the whole link>`. Browsers only allow the `web+` prefix, hence the
/// alias; [`hyperb_core::address::normalize`] accepts it.
#[wasm_bindgen(inline_js = r#"
export function hyperb_register_handler(template) {
  try {
    if (!navigator.registerProtocolHandler) return false;
    navigator.registerProtocolHandler("web+hyperb", template);
    return true;
  } catch (e) {
    return false;
  }
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = hyperb_register_handler)]
    fn register_handler(template: &str) -> bool;
}

fn register_protocol_handler() {
    let Ok(origin) = window().location().origin() else {
        return;
    };
    let path = window()
        .location()
        .pathname()
        .unwrap_or_else(|_| "/".into());
    // The page may be served from a subdirectory: links must reopen this
    // renderer, not the site root.
    let dir = path.rfind('/').map(|i| &path[..=i]).unwrap_or("/");
    let _ = register_handler(&format!("{origin}{dir}?url=%s"));
}

fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

fn load_library() -> Library {
    window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|s| s.get_item(LIBRARY_KEY).ok().flatten())
        .map(|s| Library::from_json(&s))
        .unwrap_or_default()
}

fn save_library(lib: &Library) {
    if let Some(storage) = window().local_storage().ok().flatten() {
        let _ = storage.set_item(LIBRARY_KEY, &lib.to_json());
    }
}

#[derive(Clone, PartialEq)]
pub enum State {
    Connecting,
    Open,
    Closed(Option<String>),
}

/// The session, shared by every component of the page. All fields are arena
/// handles, so it is `Copy`.
#[derive(Clone, Copy)]
pub struct Ctx {
    client: StoredValue<Client, LocalStorage>,
    conn: StoredValue<Option<Connection>, LocalStorage>,
    identity: StoredValue<Identity>,
    /// Bumped on every connect, so events from replaced connections are dropped.
    generation: StoredValue<u64>,
    pub url: RwSignal<Option<Url>>,
    pub state: RwSignal<Option<State>>,
    /// History and favorites, mirrored to `localStorage`.
    library: RwSignal<Library>,
    /// Bumped whenever the client reports changes; read it to track the client.
    version: RwSignal<u64>,
    /// The selected tab of `tabs` elements without `bind`.
    pub local_tabs: RwSignal<HashMap<String, String>>,
}

impl Ctx {
    fn new(library: RwSignal<Library>) -> Self {
        Ctx {
            client: StoredValue::new_local(Client::new()),
            conn: StoredValue::new_local(None),
            identity: StoredValue::new(load_identity()),
            generation: StoredValue::new(0),
            url: RwSignal::new(None),
            state: RwSignal::new(None),
            library,
            version: RwSignal::new(0),
            local_tabs: RwSignal::new(HashMap::new()),
        }
    }

    /// Reads the client, tracking its changes.
    pub fn with<T>(self, f: impl FnOnce(&Client) -> T) -> T {
        self.version.track();
        self.client.with_value(f)
    }

    pub fn connected(self) -> bool {
        self.with(Client::is_connected)
    }

    /// Connects to what the user typed.
    pub fn navigate(self, input: &str) -> Result<(), String> {
        let url = address::normalize(input).map_err(|e| format!("Invalid address: {e}"))?;
        self.connect(url);
        Ok(())
    }

    /// Closes the connection when the page is hidden: a page kept in the
    /// back/forward cache would otherwise hold its session open, frozen.
    fn suspend(self) {
        if self.conn.try_update_value(Option::take).flatten().is_some() {
            self.generation.update_value(|g| *g += 1);
            self.client.update_value(Client::close);
            self.state.set(Some(State::Closed(None)));
            self.refresh();
        }
    }

    pub fn reload(self) {
        if let Some(url) = self.url.get_untracked() {
            self.connect(url);
        }
    }

    fn connect(self, url: Url) {
        let conn = self.identity.with_value(|id| Connection::open(&url, id));
        let events = conn.events();
        // Until the new transport opens, nothing may be sent: hello must come first.
        self.client.update_value(Client::close);
        self.conn.set_value(Some(conn));
        self.generation.update_value(|g| *g += 1);
        let generation = self.generation.get_value();
        self.local_tabs.set(HashMap::new());
        self.url.set(Some(url));
        self.state.set(Some(State::Connecting));
        self.refresh();
        spawn_local(async move {
            while let Ok(ev) = events.recv().await {
                if self.generation.get_value() != generation {
                    break;
                }
                // Process everything that has arrived before redrawing.
                self.handle(ev);
                while let Ok(ev) = events.try_recv() {
                    self.handle(ev);
                }
                self.refresh();
            }
        });
    }

    fn handle(self, ev: Event) {
        match ev {
            Event::Open => {
                let hello = self
                    .client
                    .try_update_value(|c| c.hello())
                    .unwrap_or_default();
                self.send(hello);
                self.remember();
                self.state.set(Some(State::Open));
            }
            Event::Data(lines) => self.client.update_value(|c| c.receive(lines.as_bytes())),
            Event::Closed(reason) => {
                self.client.update_value(Client::close);
                self.conn.set_value(None);
                self.state.set(Some(State::Closed(reason)));
            }
        }
    }

    fn send(self, line: String) {
        self.conn.with_value(|c| {
            if let Some(c) = c {
                c.send(line)
            }
        });
    }

    /// Picks up what changed in the client.
    fn refresh(self) {
        let (diags, changed) = self
            .client
            .try_update_value(|c| (c.take_diagnostics(), !c.take_changes().is_empty()))
            .unwrap_or_default();
        if !diags.is_empty() {
            let url = self
                .url
                .get_untracked()
                .map(String::from)
                .unwrap_or_default();
            for d in diags {
                warn!("[{url}] {d}");
            }
        }
        if changed {
            self.version.update(|v| *v += 1);
        }
    }

    /// Records the open session in the history.
    fn remember(self) {
        let Some(url) = self.url.get_untracked() else {
            return;
        };
        let label = address::label(&url);
        let now = now_ms();
        self.library.update(|lib| {
            lib.record(url.as_str(), &label, now);
            save_library(lib);
        });
    }

    /// Adds or removes the open session from the favorites, and keeps the
    /// wasm cache in step: unfavoriting drops the cached module.
    fn set_favorite(self, url: &Url, favorite: bool) {
        let label = address::label(url);
        let now = now_ms();
        self.library.update(|lib| {
            if favorite {
                lib.favorite(url.as_str(), &label, now);
            } else {
                lib.unfavorite(url.as_str());
            }
            save_library(lib);
        });
        cache::set_favorites(self.library.with_untracked(Library::favorite_urls));
        if !favorite {
            let url = url.clone();
            spawn_local(async move { cache::forget(&url).await });
        }
    }

    fn is_favorite(self, url: &Url) -> bool {
        self.library.with(|lib| lib.is_favorite(url.as_str()))
    }

    /// User input on element `id`: writes `value` to its bound signal, if
    /// any, then runs the handler in attribute `event`, if any.
    pub fn input(self, id: &str, value: Option<Value>, event: Option<&str>) {
        let line = self.client.try_update_value(|c| {
            if let Some(v) = value {
                if let Some(Value::String(path)) = c.attr(id, "bind") {
                    c.write(&path, v);
                }
            }
            event.and_then(|e| c.trigger(id, e))
        });
        if let Some(line) = line.flatten() {
            self.send(line);
        }
        self.refresh();
    }

    /// Escape: dismisses the topmost open dialog that can be dismissed.
    fn dismiss_dialog(self) {
        let top = self.client.with_value(|c| {
            c.children(None).iter().rev().find_map(|id| {
                let r = c.get(id)?;
                let open = r.component == Component::Dialog
                    && r.attrs.get("visible") == Some(&Value::Bool(true))
                    && r.attrs.get("open") == Some(&Value::Bool(true));
                open.then(|| {
                    (
                        id.clone(),
                        !r.attrs.get("onDismiss").is_none_or(Value::is_null),
                    )
                })
            })
        });
        if let Some((id, true)) = top {
            self.input(&id, None, Some("onDismiss"));
        }
    }

    pub fn public_key(self) -> String {
        self.identity.with_value(|id| id.public().to_string())
    }
}

/// The identity kept in `localStorage`, created on first use. Without
/// storage (a private window, say) a temporary one is used.
fn load_identity() -> Identity {
    let storage = window().local_storage().ok().flatten();
    let stored = storage
        .as_ref()
        .and_then(|s| s.get_item(IDENTITY_KEY).ok().flatten())
        .and_then(|h| <[u8; 32]>::try_from(hex::decode(h.trim()).ok()?).ok());
    if let Some(secret) = stored {
        return Identity::from_secret(secret);
    }
    let id = Identity::generate();
    match storage {
        Some(s) if s.set_item(IDENTITY_KEY, &hex::encode(id.secret())).is_ok() => {}
        _ => warn!("cannot store the identity; using a temporary one"),
    }
    id
}

/// The address in `?url=`, if any.
fn query_address() -> Option<String> {
    let search = window().location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search)
        .ok()?
        .get("url")
}

/// Puts the address in `?url=` so reloading the page reconnects.
fn set_query_address(addr: &str) {
    let Ok(params) = web_sys::UrlSearchParams::new_with_str("") else {
        return;
    };
    params.append("url", addr);
    let path = window().location().pathname().unwrap_or_default();
    let new = format!("{path}?{}", String::from(params.to_string()));
    if let Ok(h) = window().history() {
        let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&new));
    }
}

#[component]
pub fn Browser() -> impl IntoView {
    let library = RwSignal::new(load_library());
    cache::set_favorites(library.with_untracked(Library::favorite_urls));
    let ctx = Ctx::new(library);
    register_protocol_handler();
    let address = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let show_library = RwSignal::new(false);
    let go = move |input: String| match ctx.navigate(&input) {
        Ok(()) => {
            let url = ctx
                .url
                .get_untracked()
                .map(String::from)
                .unwrap_or_default();
            set_query_address(&url);
            address.set(url);
            error.set(None);
        }
        Err(e) => error.set(Some(e)),
    };
    let open = Callback::new(go);
    let open_and_close = Callback::new(move |input: String| {
        open.run(input);
        show_library.set(false);
    });
    if let Some(a) = query_address() {
        go(a);
    }
    window_event_listener(ev::keydown, move |e| {
        if e.key() == "Escape" {
            ctx.dismiss_dialog();
        }
    });
    window_event_listener_untyped("pagehide", move |_| ctx.suspend());
    window_event_listener_untyped("pageshow", move |e| {
        if e.unchecked_into::<web_sys::PageTransitionEvent>()
            .persisted()
        {
            ctx.reload();
        }
    });

    let state = move || ctx.state.get();
    let has_page = Memo::new(move |_| ctx.url.with(Option::is_some));
    let title = move || match ctx.url.get() {
        Some(u) => format!("{} · hyperb", address::label(&u)),
        None => "hyperb".into(),
    };
    let (status, status_title) = (
        move || match (
            state(),
            ctx.url.with(|u| u.as_ref().map(address::is_secure)),
        ) {
            (Some(State::Open), Some(true)) => "secure",
            (Some(State::Open), _) => "insecure",
            (Some(State::Closed(_)), _) => "offline",
            _ => "",
        },
        move || match (
            state(),
            ctx.url.with(|u| u.as_ref().map(address::is_secure)),
        ) {
            (Some(State::Open), Some(true)) => "Connected (encrypted)",
            (Some(State::Open), _) => "Connected (not encrypted)",
            (Some(State::Closed(_)), _) => "Disconnected",
            _ => "",
        },
    );
    let banner = move || match state() {
        Some(State::Closed(Some(r))) => Some(format!("Disconnected: {r}")),
        Some(State::Closed(None)) => Some("Disconnected. The server closed the connection.".into()),
        _ => error.get(),
    };
    let favorite = move || {
        ctx.url
            .with(|u| u.as_ref().is_some_and(|u| ctx.is_favorite(u)))
    };

    view! {
        <Title text=title/>
        <div class="hb">
            <form
                class="hb-toolbar"
                on:submit=move |e| {
                    e.prevent_default();
                    go(address.get_untracked());
                }
            >
                <button
                    type="button"
                    class="hb-icon-button"
                    title="Reconnect"
                    aria-label="Reconnect"
                    on:click=move |_| match ctx.url.get_untracked() {
                        Some(_) => ctx.reload(),
                        None => go(address.get_untracked()),
                    }
                >
                    "⟳"
                </button>
                <div class="hb-url">
                    <span class="hb-status" data-status=status title=status_title></span>
                    <input
                        type="text"
                        inputmode="url"
                        autocomplete="off"
                        spellcheck="false"
                        placeholder="Server address, e.g. https://example.com/app or localhost:9001/restaurant"
                        aria-label="Server address"
                        prop:value=move || address.get()
                        on:input:target=move |ev| address.set(ev.target().value())
                    />
                </div>
                <button
                    type="button"
                    class="hb-icon-button"
                    class:he-hidden=move || ctx.url.get().is_none()
                    title=move || if favorite() { "Remove from favorites" } else { "Add to favorites" }
                    aria-label="Favorite"
                    on:click=move |_| {
                        if let Some(url) = ctx.url.get_untracked() {
                            let on = !ctx.is_favorite(&url);
                            ctx.set_favorite(&url, on);
                        }
                    }
                >
                    {move || if favorite() { "★" } else { "☆" }}
                </button>
                <button
                    type="button"
                    class="hb-icon-button"
                    title="History and favorites"
                    aria-label="History and favorites"
                    on:click=move |_| show_library.update(|open| *open = !*open)
                >
                    "☰"
                </button>
            </form>
            <div class="hb-progress" class:he-hidden=move || state() != Some(State::Connecting)></div>
            <div class="hb-banner" class:he-hidden=move || banner().is_none() role="status">
                <span>{banner}</span>
                <button
                    class="hb-link"
                    class:he-hidden=move || !matches!(state(), Some(State::Closed(_)))
                    on:click=move |_| ctx.reload()
                >
                    "Reconnect"
                </button>
            </div>
            <Show when=move || show_library.get()>
                <LibraryPanel ctx=ctx open=open_and_close/>
            </Show>
            {move || {
                if has_page.get() {
                    view! {
                        <main class="he-root he-column" data-gap="medium" data-align="stretch">
                            <Children ctx parent=None/>
                        </main>
                    }
                        .into_any()
                } else {
                    view! { <Welcome ctx/> }.into_any()
                }
            }}
        </div>
    }
}

/// The history and favorites, shown under the toolbar by the ☰ button.
#[component]
fn LibraryPanel(ctx: Ctx, open: Callback<String>) -> impl IntoView {
    let clear_history = move |_| {
        ctx.library.update(|lib| {
            lib.history.clear();
            save_library(lib);
        });
    };
    view! {
        <section class="hb-library">
            <div class="hb-library-head">
                <h2>"Favorites"</h2>
            </div>
            <ul class="hb-entries">
                {move || {
                    ctx.library
                        .get()
                        .favorites
                        .is_empty()
                        .then(|| view! { <li class="hb-empty">"Star an address to keep it here."</li> })
                }}
                {move || {
                    ctx.library
                        .get()
                        .favorites
                        .into_iter()
                        .map(|entry| {
                            let url = entry.url.clone();
                            let remove = entry.url.clone();
                            view! {
                                <li class="hb-entry-row">
                                    <button
                                        class="hb-entry"
                                        on:click=move |_| open.run(url.clone())
                                    >
                                        <span class="hb-entry-label">{entry.label}</span>
                                        <span class="hb-entry-url">{entry.url}</span>
                                    </button>
                                    <button
                                        class="hb-icon-button hb-entry-action"
                                        title="Remove from favorites"
                                        aria-label="Remove from favorites"
                                        on:click=move |_| {
                                            if let Ok(url) = Url::parse(&remove) {
                                                ctx.set_favorite(&url, false);
                                            }
                                        }
                                    >
                                        "✕"
                                    </button>
                                </li>
                            }
                        })
                        .collect_view()
                }}
            </ul>
            <div class="hb-library-head">
                <h2>"History"</h2>
                <button class="hb-link" on:click=clear_history>"Clear"</button>
            </div>
            <ul class="hb-entries">
                {move || {
                    ctx.library
                        .get()
                        .history
                        .is_empty()
                        .then(|| view! { <li class="hb-empty">"Nothing opened yet."</li> })
                }}
                {move || {
                    ctx.library
                        .get()
                        .history
                        .into_iter()
                        .map(|entry| {
                            let url = entry.url.clone();
                            let forget = entry.url.clone();
                            view! {
                                <li class="hb-entry-row">
                                    <button
                                        class="hb-entry"
                                        on:click=move |_| open.run(url.clone())
                                    >
                                        <span class="hb-entry-label">{entry.label}</span>
                                        <span class="hb-entry-url">{entry.url}</span>
                                    </button>
                                    <button
                                        class="hb-icon-button hb-entry-action"
                                        title="Remove from history"
                                        aria-label="Remove from history"
                                        on:click=move |_| {
                                            ctx.library.update(|lib| {
                                                lib.history.retain(|e| e.url != forget);
                                                save_library(lib);
                                            });
                                        }
                                    >
                                        "✕"
                                    </button>
                                </li>
                            }
                        })
                        .collect_view()
                }}
            </ul>
        </section>
    }
}

#[component]
fn Welcome(ctx: Ctx) -> impl IntoView {
    view! {
        <div class="hb-welcome">
            <h1 class="he-h1">"hyperb"</h1>
            <p class="he-caption">
                "Enter the address of a hypereffect server, e.g. hyperb://example.com/app. "
                "Remote servers use https; localhost, IP addresses and .onion use http."
            </p>
            <p class="he-caption">"Your key"</p>
            <code class="hb-key">{ctx.public_key()}</code>
        </div>
    }
}
