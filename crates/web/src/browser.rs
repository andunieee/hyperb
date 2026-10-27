//! The browser in the page: an address bar, a connection to a hypereffect
//! server, and the rendered session. Runs only in the browser (feature
//! `hydrate`); the server renders a placeholder.
//!
//! Entering an address opens a connection (HTTP with SSE over fetch, or a
//! remote wasm app run by the browser); when it opens the page sends `hello`
//! and from then on feeds every message to the [`Client`]. When the transport
//! closes the last state stays on screen with controls disabled, and a banner
//! offers to reconnect.
//!
//! The identity is kept in `localStorage`, and so are the history and the
//! favorites ([`crate::store`]), which the welcome page and the toolbar's
//! menu list. `?url=<address>` opens an address, and so does
//! `?link=<address>`, which is where the page has the browser send
//! `web+hyperb://` links once the user lets it handle them.

use crate::render::Children;
use crate::store;
use hyperb_core::address;
use hyperb_core::hypereffect::client::Client;
use hyperb_core::hypereffect::{Component, Map, Value};
use hyperb_core::identity::Identity;
use hyperb_core::library::{Entry, Library};
use hyperb_core::net::{Connection, Event};
use leptos::ev;
use leptos::logging::warn;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_meta::Title;
use std::collections::HashMap;
use url::Url;
use wasm_bindgen::JsCast;

const IDENTITY_KEY: &str = "hyperb.identity";

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
    /// Bumped whenever the client reports changes; read it to track the client.
    version: RwSignal<u64>,
    /// The selected tab of `tabs` elements without `bind`.
    pub local_tabs: RwSignal<HashMap<String, String>>,
    /// History and favorites, as last stored.
    library: RwSignal<Library>,
}

impl Ctx {
    fn new() -> Self {
        Ctx {
            client: StoredValue::new_local(Client::new()),
            conn: StoredValue::new_local(None),
            identity: StoredValue::new(load_identity()),
            generation: StoredValue::new(0),
            url: RwSignal::new(None),
            state: RwSignal::new(None),
            version: RwSignal::new(0),
            local_tabs: RwSignal::new(HashMap::new()),
            library: RwSignal::new(store::load()),
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

    /// Connects to what the user typed or picked, or to a link, which
    /// cannot open local files.
    pub fn navigate(self, input: &str, link: bool) -> Result<(), String> {
        let url = if link {
            address::from_link(input)
        } else {
            address::normalize(input)
        };
        let url = url.map_err(|e| format!("Invalid address: {e}"))?;
        self.library.set(store::update(|l, now| l.visit(&url, now)));
        self.connect(url);
        Ok(())
    }

    fn is_favorite(self) -> bool {
        let url = self.url.get();
        self.library
            .with(|l| url.is_some_and(|u| l.is_favorite(&u)))
    }

    /// Adds `url` to the favorites or removes it. A new favorite wasm app
    /// is downloaded to be kept; a module no favorite runs is deleted.
    fn set_favorite(self, url: &Url, on: bool) {
        let lib = store::update(|l, now| l.set_favorite(url, on, now));
        if on && lib.keeps_module(url) {
            store::download(url.clone());
        }
        if !on {
            spawn_local(store::prune(lib.clone()));
        }
        self.library.set(lib);
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
        // Until the new transport opens, nothing may be sent: hello must come first.
        self.client.update_value(Client::close);
        self.conn.set_value(None);
        self.generation.update_value(|g| *g += 1);
        let generation = self.generation.get_value();
        self.local_tabs.set(HashMap::new());
        self.url.set(Some(url.clone()));
        self.state.set(Some(State::Connecting));
        self.refresh();
        let keeps = self.library.with_untracked(|l| l.keeps_module(&url));
        spawn_local(async move {
            // A favorite wasm app runs from its kept module, which is
            // downloaded again for next time.
            let module = match keeps {
                true => store::module(&url).await,
                false => None,
            };
            if self.generation.get_value() != generation {
                return;
            }
            let conn = match module {
                Some(m) => Connection::open_module(&url, m),
                None => self.identity.with_value(|id| Connection::open(&url, id)),
            };
            if keeps {
                store::download(url);
            }
            let events = conn.events();
            self.conn.set_value(Some(conn));
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
                let params = self
                    .conn
                    .with_value(|c| c.as_ref().map(|c| c.params().clone()))
                    .unwrap_or_default();
                let hello = self
                    .client
                    .try_update_value(|c| c.hello_with(&params))
                    .unwrap_or_default();
                self.send(hello);
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

/// Lets the browser send `web+hyperb://` links to this page, as
/// `?link=<address>`. The browser asks the user first.
fn register_links() -> Result<(), String> {
    let loc = window().location();
    let page = format!(
        "{}{}?link=%s",
        loc.origin().unwrap_or_default(),
        loc.pathname().unwrap_or_default()
    );
    window()
        .navigator()
        .register_protocol_handler("web+hyperb", &page, "hyperb")
        .map_err(|e| {
            e.dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
                .unwrap_or_else(|| "the browser refused".into())
        })
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

/// The address in `?url=` or `?link=`, if any.
fn query_address() -> Option<String> {
    let search = window().location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params.get("url").or_else(|| params.get("link"))
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
    let ctx = Ctx::new();
    let address = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    // Addresses in the page's own URL may come from any other page, so they
    // are links; addresses the user types or picks are not.
    let go = move |input: String, link: bool| match ctx.navigate(&input, link) {
        Ok(()) => {
            let addr = ctx
                .url
                .get_untracked()
                .map(|u| address::hyperb(&u))
                .unwrap_or_default();
            set_query_address(&addr);
            address.set(addr);
            error.set(None);
        }
        Err(e) => error.set(Some(e)),
    };
    if let Some(a) = query_address() {
        go(a, true);
    }
    let open = Callback::new(move |a: String| go(a, false));
    let menu = RwSignal::new(false);
    window_event_listener(ev::keydown, move |e| {
        if e.key() == "Escape" {
            ctx.dismiss_dialog();
        }
    });
    // Another tab changed the history or favorites.
    window_event_listener_untyped("storage", move |_| ctx.library.set(store::load()));
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

    view! {
        <Title text=title/>
        <div class="hb">
            <form
                class="hb-toolbar"
                on:submit=move |e| {
                    e.prevent_default();
                    go(address.get_untracked(), false);
                }
            >
                <button
                    type="button"
                    class="hb-icon-button"
                    title="Reconnect"
                    aria-label="Reconnect"
                    on:click=move |_| match ctx.url.get_untracked() {
                        Some(_) => ctx.reload(),
                        None => go(address.get_untracked(), false),
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
                    class="hb-icon-button hb-star"
                    class:hb-on=move || ctx.is_favorite()
                    disabled=move || ctx.url.with(Option::is_none)
                    title=move || if ctx.is_favorite() { "Remove from favorites" } else { "Add to favorites" }
                    aria-label="Favorite"
                    aria-pressed=move || ctx.is_favorite().to_string()
                    on:click=move |_| {
                        if let Some(u) = ctx.url.get_untracked() {
                            ctx.set_favorite(&u, !ctx.is_favorite());
                        }
                    }
                >
                    {move || if ctx.is_favorite() { "★" } else { "☆" }}
                </button>
                <button
                    type="button"
                    class="hb-icon-button"
                    title="Favorites and history"
                    aria-label="Favorites and history"
                    aria-expanded=move || menu.get().to_string()
                    on:click=move |_| menu.update(|m| *m = !*m)
                >
                    "☰"
                </button>
            </form>
            <Show when=move || menu.get()>
                <div class="hb-menu">
                    <LibraryView
                        ctx
                        limit=50
                        open=Callback::new(move |a: String| {
                            menu.set(false);
                            open.run(a);
                        })
                    />
                </div>
            </Show>
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
            {move || {
                if has_page.get() {
                    view! {
                        <main class="he-root he-column" data-gap="medium" data-align="stretch">
                            <Children ctx parent=None/>
                        </main>
                    }
                        .into_any()
                } else {
                    view! { <Welcome ctx open/> }.into_any()
                }
            }}
        </div>
    }
}

#[component]
fn Welcome(ctx: Ctx, open: Callback<String>) -> impl IntoView {
    let registered = RwSignal::new(None::<Result<(), String>>);
    view! {
        <div class="hb-welcome">
            <h1 class="he-h1">"hyperb"</h1>
            <p class="he-caption">
                "Enter the address of a hypereffect server, e.g. hyperb://example.com/app. "
                "Remote servers use https; localhost, IP addresses and .onion use http."
            </p>
            <LibraryView ctx limit=12 open/>
            <p class="he-caption">"Your key"</p>
            <code class="hb-key">{ctx.public_key()}</code>
            <p class="he-caption">
                <button class="hb-link" on:click=move |_| registered.set(Some(register_links()))>
                    "Open web+hyperb:// links here"
                </button>
                {move || match registered.get() {
                    Some(Ok(())) => " · Confirm it if your browser asks.".to_string(),
                    Some(Err(e)) => format!(" · Not available: {e}"),
                    None => String::new(),
                }}
            </p>
        </div>
    }
}

/// Favorites, then the most recent `limit` history entries.
#[component]
fn LibraryView(ctx: Ctx, limit: usize, open: Callback<String>) -> impl IntoView {
    let favorites = move || ctx.library.with(|l| l.favorites.clone());
    let history = move || {
        ctx.library
            .with(|l| l.history.iter().take(limit).cloned().collect::<Vec<_>>())
    };
    let unfavorite = Callback::new(move |a: String| {
        if let Ok(u) = address::normalize(&a) {
            ctx.set_favorite(&u, false);
        }
    });
    let forget = Callback::new(move |a: String| {
        ctx.library.set(store::update(|l, _| l.forget(&a)));
    });
    let clear = move |_| ctx.library.set(store::update(|l, _| l.clear_history()));
    view! {
        <div class="hb-library">
            <Show when=move || ctx.library.with(|l| l.favorites.is_empty() && l.history.is_empty())>
                <p class="he-caption">"Apps you open and your favorites show up here."</p>
            </Show>
            <Show when=move || ctx.library.with(|l| !l.favorites.is_empty())>
                <h2 class="hb-library-heading">"Favorites"</h2>
                <Entries entries=Signal::derive(favorites) icon="★" remove_title="Remove from favorites" open remove=unfavorite/>
            </Show>
            <Show when=move || ctx.library.with(|l| !l.history.is_empty())>
                <h2 class="hb-library-heading">
                    "History"
                    <button class="hb-link" on:click=clear>"Clear history"</button>
                </h2>
                <Entries entries=Signal::derive(history) icon="🕘" remove_title="Remove from history" open remove=forget/>
            </Show>
        </div>
    }
}

#[component]
fn Entries(
    entries: Signal<Vec<Entry>>,
    icon: &'static str,
    remove_title: &'static str,
    open: Callback<String>,
    remove: Callback<String>,
) -> impl IntoView {
    view! {
        <ul class="hb-entries">
            <For each=move || entries.get() key=|e| e.address.clone() let:e>
                {
                    let (a, b) = (e.address.clone(), e.address.clone());
                    view! {
                        <li>
                            <button class="hb-entry" title=e.address.clone() on:click=move |_| open.run(a.clone())>
                                <span class="hb-entry-icon" aria-hidden="true">{icon}</span>
                                <span class="hb-entry-text">
                                    <span class="hb-entry-title">{e.title.clone()}</span>
                                    <span class="hb-entry-address">{e.address.clone()}</span>
                                </span>
                            </button>
                            <button
                                class="hb-icon-button hb-entry-remove"
                                title=remove_title
                                aria-label=remove_title
                                on:click=move |_| remove.run(b.clone())
                            >
                                "×"
                            </button>
                        </li>
                    }
                }
            </For>
        </ul>
    }
}
