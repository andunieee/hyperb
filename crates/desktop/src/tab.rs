//! One browser tab: a URL bar, a connection to a hypereffect server, and the
//! rendered session.
//!
//! Typing an address opens a connection; when it opens the tab sends `hello`
//! and from then on feeds every frame to the [`Client`], redrawing once per
//! batch of frames. When the transport closes the last state stays on screen
//! with controls disabled, and a banner offers to reconnect.

use crate::clamp::Clamp;
use crate::debug::{Debugger, Dir};
use crate::library::Store;
use crate::render::Renderer;
use gtk::prelude::*;
use gtk::{gdk, glib, pango};
use gtk4 as gtk;
use hyperb_core::address;
use hyperb_core::hypereffect::client::Client;
use hyperb_core::hypereffect::{json, Value};
use hyperb_core::identity::Identity;
use hyperb_core::library::Entry;
use hyperb_core::net::{Connection, Event};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// The widest the rendered page gets; the rest of the window is margin.
const CONTENT_WIDTH: i32 = 800;

pub struct Tab {
    /// The notebook page.
    pub page: gtk::Box,
    /// The notebook tab: spinner, title and close button.
    pub tab_label: gtk::Box,
    pub title: gtk::Label,
    pub close_button: gtk::Button,
    spinner: gtk::Spinner,
    url_entry: gtk::Entry,
    favorite_button: gtk::Button,
    banner: gtk::Box,
    banner_text: gtk::Label,
    content: gtk::Stack,
    debug: Debugger,
    session: RefCell<Session>,
    renderer: RefCell<Renderer>,
    quiet: Cell<bool>,
    /// History, favorites and the wasm cache, shared by every tab.
    library: Rc<Store>,
    identity: Identity,
}

struct Session {
    client: Client,
    conn: Option<Connection>,
    url: Option<Url>,
    /// Bumped on every connect, so events from replaced connections are dropped.
    generation: u64,
    state: State,
}

#[derive(Clone, PartialEq)]
enum State {
    Idle,
    Connecting,
    Open,
    Closed(Option<String>),
}

impl Tab {
    pub fn new(identity: Identity, library: Rc<Store>) -> Rc<Tab> {
        Rc::new_cyclic(|weak| {
            // Tab strip entry.
            let spinner = gtk::Spinner::new();
            spinner.set_visible(false);
            let title = gtk::Label::new(Some("New Tab"));
            title.set_width_chars(18);
            title.set_max_width_chars(28);
            title.set_ellipsize(pango::EllipsizeMode::Middle);
            let close_button = gtk::Button::from_icon_name("window-close-symbolic");
            close_button.add_css_class("flat");
            close_button.add_css_class("circular");
            close_button.add_css_class("hb-tab-close");
            close_button.set_tooltip_text(Some("Close tab"));
            let tab_label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            tab_label.append(&spinner);
            tab_label.append(&title);
            tab_label.append(&close_button);

            // Toolbar.
            let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
            reload.set_tooltip_text(Some("Reconnect (F5)"));
            let url_entry = gtk::Entry::new();
            url_entry.set_hexpand(true);
            url_entry.set_placeholder_text(Some(
                "Server address, e.g. https://example.com/app or localhost:9001/restaurant",
            ));
            url_entry.set_input_purpose(gtk::InputPurpose::Url);
            let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            toolbar.add_css_class("hb-toolbar");
            toolbar.append(&reload);
            toolbar.append(&url_entry);

            // Favorite this address, and the history/favorites popover.
            let favorite_button = gtk::Button::from_icon_name("non-starred-symbolic");
            favorite_button.add_css_class("flat");
            favorite_button.set_tooltip_text(Some("Favorite"));
            favorite_button.set_sensitive(false);
            toolbar.append(&favorite_button);

            let library_button = gtk::MenuButton::new();
            library_button.set_icon_name("document-open-recent-symbolic");
            library_button.add_css_class("flat");
            library_button.set_tooltip_text(Some("History and favorites"));
            let library_popover = gtk::Popover::new();
            library_popover.set_has_arrow(false);
            library_button.set_popover(Some(&library_popover));
            toolbar.append(&library_button);
            {
                let weak = weak.clone();
                favorite_button.connect_clicked(move |_| {
                    if let Some(t) = weak.upgrade() {
                        t.toggle_favorite();
                    }
                });
            }
            {
                let weak = weak.clone();
                library_popover.connect_show(move |popover| {
                    if let Some(t) = weak.upgrade() {
                        popover.set_child(Some(&t.library_list(popover)));
                    }
                });
            }

            // Disconnected banner.
            let banner_text = gtk::Label::new(None);
            banner_text.set_hexpand(true);
            banner_text.set_xalign(0.0);
            banner_text.set_wrap(true);
            let banner_reload = gtk::Button::with_label("Reconnect");
            let banner = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            banner.add_css_class("hb-banner");
            banner.append(&gtk::Image::from_icon_name("network-offline-symbolic"));
            banner.append(&banner_text);
            banner.append(&banner_reload);
            banner.set_visible(false);

            // Rendered session: the root column scrolls, dialogs overlay it.
            let main = gtk::Box::new(gtk::Orientation::Vertical, 12);
            main.add_css_class("he-root");
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
            scroll.set_vexpand(true);
            scroll.set_child(Some(&Clamp::new(CONTENT_WIDTH, &main)));
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&scroll));

            let content = gtk::Stack::new();
            content.set_vexpand(true);
            content.add_named(&welcome(), Some("welcome"));
            content.add_named(&overlay, Some("page"));
            content.set_visible_child_name("welcome");

            let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
            page.append(&toolbar);
            page.append(&banner);
            let debug = Debugger::new(weak.clone());
            let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            body.append(&content);
            body.append(&debug.widget);
            // The debugger only takes space the page does not use.
            let panel = debug.widget.clone();
            body.add_tick_callback(move |body, _| {
                let room = body.width() >= CONTENT_WIDTH + Debugger::WIDTH;
                if panel.is_visible() != room {
                    panel.set_visible(room);
                }
                glib::ControlFlow::Continue
            });
            page.append(&body);

            {
                let w = weak.clone();
                url_entry.connect_activate(move |e| {
                    if let Some(t) = w.upgrade() {
                        t.navigate(&e.text());
                    }
                });
            }
            for b in [&reload, &banner_reload] {
                let w = weak.clone();
                b.connect_clicked(move |_| {
                    if let Some(t) = w.upgrade() {
                        t.reload();
                    }
                });
            }
            {
                let w = weak.clone();
                let keys = gtk::EventControllerKey::new();
                keys.set_propagation_phase(gtk::PropagationPhase::Capture);
                keys.connect_key_pressed(move |_, key, _, _| match w.upgrade() {
                    Some(t) if key == gdk::Key::Escape && t.dismiss_dialog() => {
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                });
                overlay.add_controller(keys);
            }

            Tab {
                page,
                tab_label,
                title,
                close_button,
                spinner,
                url_entry,
                favorite_button,
                banner,
                banner_text,
                content,
                debug,
                session: RefCell::new(Session {
                    client: Client::new(),
                    conn: None,
                    url: None,
                    generation: 0,
                    state: State::Idle,
                }),
                renderer: RefCell::new(Renderer::new(weak.clone(), overlay, main)),
                quiet: Cell::new(false),
                library,
                identity,
            }
        })
    }

    pub fn focus_url(&self) {
        self.url_entry.grab_focus();
    }

    /// Connects to what the user typed.
    pub fn navigate(self: &Rc<Self>, input: &str) {
        match address::normalize(input) {
            Ok(url) => {
                self.url_entry.set_text(url.as_str());
                self.title.set_text(&address::label(&url));
                self.title.set_tooltip_text(Some(url.as_str()));
                self.content.grab_focus();
                self.connect(url);
            }
            Err(e) => {
                self.banner_text.set_text(&format!("Invalid address: {e}"));
                self.banner.set_visible(true);
            }
        }
    }

    pub fn reload(self: &Rc<Self>) {
        let url = self.session.borrow().url.clone();
        match url {
            Some(url) => self.connect(url),
            None => self.navigate(&self.url_entry.text()),
        }
    }

    fn connect(self: &Rc<Self>, url: Url) {
        let conn = Connection::open(&url, &self.identity);
        let events = conn.events();
        let generation = {
            let mut s = self.session.borrow_mut();
            // Until the new transport opens, nothing may be sent: hello must come first.
            s.client.close();
            s.conn = Some(conn);
            s.url = Some(url);
            s.generation += 1;
            s.state = State::Connecting;
            s.generation
        };
        self.content.set_visible_child_name("page");
        self.render();
        self.update_chrome();

        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(ev) = events.recv().await {
                let Some(t) = weak.upgrade() else { break };
                if t.session.borrow().generation != generation {
                    break;
                }
                // Process everything that has arrived before redrawing.
                t.handle(ev);
                while let Ok(ev) = events.try_recv() {
                    t.handle(ev);
                }
                t.render();
                t.update_chrome();
            }
        });
    }

    fn handle(&self, ev: Event) {
        let opened = {
            let mut s = self.session.borrow_mut();
            let s = &mut *s;
            match ev {
                Event::Open => {
                    let hello = s.client.hello();
                    if let Some(c) = &s.conn {
                        self.debug.log(Dir::Sent, &hello);
                        c.send(hello);
                    }
                    s.state = State::Open;
                    true
                }
                Event::Data(lines) => {
                    for l in lines.lines() {
                        self.debug.log(Dir::Received, l);
                    }
                    s.client.receive(lines.as_bytes());
                    false
                }
                Event::Closed(reason) => {
                    s.client.close();
                    s.conn = None;
                    s.state = State::Closed(reason);
                    false
                }
            }
        };
        if opened {
            self.record_visit();
        }
    }

    /// Adds the open address to the history.
    fn record_visit(&self) {
        let url = self.session.borrow().url.clone();
        let Some(url) = url else { return };
        let label = address::label(&url);
        let now = now_ms();
        self.library
            .update(|lib| lib.record(url.as_str(), &label, now));
    }

    /// Stars or unstars the open address. Unstarring forgets its cached wasm
    /// module.
    fn toggle_favorite(self: &Rc<Self>) {
        let url = self.session.borrow().url.clone();
        let Some(url) = url else { return };
        let label = address::label(&url);
        let now = now_ms();
        self.library
            .update(|lib| lib.toggle_favorite(url.as_str(), &label, now));
        self.update_chrome();
    }

    /// The contents of the history/favorites popover, rebuilt each time it is
    /// opened.
    fn library_list(self: &Rc<Self>, popover: &gtk::Popover) -> gtk::Widget {
        let (favorites, history) = self
            .library
            .with(|lib| (lib.favorites.clone(), lib.history.clone()));

        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        list.add_css_class("hb-library");
        list.append(&heading("Favorites"));
        if favorites.is_empty() {
            list.append(&empty("Star an address to keep it here."));
        }
        for entry in &favorites {
            list.append(&self.entry_row(entry, true, popover));
        }

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&heading("History"));
        let clear = gtk::Button::with_label("Clear");
        clear.add_css_class("flat");
        clear.set_halign(gtk::Align::End);
        clear.set_hexpand(true);
        {
            let weak = Rc::downgrade(self);
            let popover = popover.clone();
            clear.connect_clicked(move |_| {
                if let Some(t) = weak.upgrade() {
                    t.library.update(|lib| lib.history.clear());
                }
                popover.popdown();
            });
        }
        row.append(&clear);
        list.append(&row);
        if history.is_empty() {
            list.append(&empty("Nothing opened yet."));
        }
        for entry in &history {
            list.append(&self.entry_row(entry, false, popover));
        }

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.set_min_content_height(320);
        scrolled.set_child(Some(&list));
        scrolled.upcast()
    }

    /// One history or favorites row: open the address, or forget the entry.
    fn entry_row(
        self: &Rc<Self>,
        entry: &Entry,
        favorite: bool,
        popover: &gtk::Popover,
    ) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        row.add_css_class("hb-entry-row");

        let open = gtk::Button::new();
        open.add_css_class("hb-entry");
        open.set_hexpand(true);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let label = gtk::Label::new(Some(&entry.label));
        label.set_xalign(0.0);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.add_css_class("hb-entry-label");
        let address_label = gtk::Label::new(Some(&entry.url));
        address_label.set_xalign(0.0);
        address_label.set_ellipsize(pango::EllipsizeMode::Middle);
        address_label.add_css_class("hb-entry-url");
        text.append(&label);
        text.append(&address_label);
        open.set_child(Some(&text));
        let tooltip = if entry.visits > 0 {
            format!("{} · opened {} times", entry.url, entry.visits)
        } else {
            entry.url.clone()
        };
        open.set_tooltip_text(Some(&tooltip));
        {
            let weak = Rc::downgrade(self);
            let popover = popover.clone();
            let address = entry.url.clone();
            open.connect_clicked(move |_| {
                if let Some(t) = weak.upgrade() {
                    t.navigate(&address);
                }
                popover.popdown();
            });
        }
        row.append(&open);

        let remove = gtk::Button::from_icon_name("window-close-symbolic");
        remove.add_css_class("flat");
        remove.add_css_class("hb-entry-action");
        remove.set_tooltip_text(Some(if favorite {
            "Remove from favorites and delete its cached copy"
        } else {
            "Remove from history"
        }));
        {
            let weak = Rc::downgrade(self);
            let popover = popover.clone();
            let address = entry.url.clone();
            remove.connect_clicked(move |_| {
                if let Some(t) = weak.upgrade() {
                    if favorite {
                        t.library.update(|lib| lib.unfavorite(&address));
                    } else {
                        t.library
                            .update(|lib| lib.history.retain(|e| e.url != address));
                    }
                    t.update_chrome();
                }
                popover.popdown();
            });
        }
        row.append(&remove);
        row.upcast()
    }

    pub fn is_quiet(&self) -> bool {
        self.quiet.get()
    }

    /// User input on element `id`: writes `value` to its bound signal, if
    /// any, then runs the handler in attribute `event`, if any.
    pub fn input(&self, id: &str, value: Option<Value>, event: Option<&str>) {
        let line = {
            let mut s = self.session.borrow_mut();
            if let Some(v) = value {
                let bind = s.client.attr(id, "bind");
                if let Some(path) = bind.as_ref().and_then(Value::as_str) {
                    s.client.write(path, v);
                }
            }
            event.and_then(|e| s.client.trigger(id, e))
        };
        if let Some(line) = line {
            if let Some(c) = &self.session.borrow().conn {
                self.debug.log(Dir::Sent, &line);
                c.send(line);
            }
        }
        self.render();
    }

    pub fn select_tab(&self, tabs: &str, tab: &str) {
        self.input(tabs, Some(json!(tab)), Some("onChange"));
    }

    fn dismiss_dialog(&self) -> bool {
        let top = {
            let s = self.session.borrow();
            self.renderer.borrow().top_dialog(&s.client)
        };
        match top {
            Some(id) => {
                self.input(&id, None, Some("onDismiss"));
                true
            }
            None => false,
        }
    }

    fn render(&self) {
        let changes = {
            let mut s = self.session.borrow_mut();
            for d in s.client.take_diagnostics() {
                let url = s.url.as_ref().map(Url::as_str).unwrap_or("-");
                eprintln!("[{url}] {d}");
            }
            s.client.take_changes()
        };
        if changes.is_empty() {
            return;
        }
        self.quiet.set(true);
        {
            let s = self.session.borrow();
            self.renderer.borrow_mut().apply(&s.client, changes);
        }
        self.quiet.set(false);
        self.debug.sync_elements(&self.session.borrow().client);
    }

    /// Outlines element `id` in the page, or clears the outline.
    pub fn highlight(&self, id: Option<&str>) {
        if let Ok(mut r) = self.renderer.try_borrow_mut() {
            r.highlight(id);
        }
    }

    fn update_chrome(&self) {
        let s = self.session.borrow();
        let connecting = s.state == State::Connecting;
        self.spinner.set_visible(connecting);
        self.spinner.set_spinning(connecting);
        let icon = match (&s.state, s.url.as_ref().map(Url::scheme)) {
            (State::Open, Some("https" | "iroh")) => Some("channel-secure-symbolic"),
            (State::Open, _) => Some("channel-insecure-symbolic"),
            (State::Closed(_), _) => Some("network-offline-symbolic"),
            _ => None,
        };
        self.url_entry.set_primary_icon_name(icon);
        self.url_entry
            .set_primary_icon_tooltip_text(match &s.state {
                State::Open if icon == Some("channel-secure-symbolic") => {
                    Some("Connected (encrypted)")
                }
                State::Open => Some("Connected (not encrypted)"),
                State::Closed(_) => Some("Disconnected"),
                _ => None,
            });
        let favorite = s
            .url
            .as_ref()
            .is_some_and(|u| self.library.with(|lib| lib.is_favorite(u.as_str())));
        self.favorite_button.set_sensitive(s.url.is_some());
        self.favorite_button.set_icon_name(if favorite {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        self.favorite_button.set_tooltip_text(Some(if favorite {
            "Remove from favorites"
        } else {
            "Favorite (caches its wasm app)"
        }));
        match &s.state {
            State::Closed(reason) => {
                let text = match reason {
                    Some(r) => format!("Disconnected: {r}"),
                    None => "Disconnected. The server closed the connection.".into(),
                };
                self.banner_text.set_text(&text);
                self.banner.set_visible(true);
            }
            _ => self.banner.set_visible(false),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn heading(text: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_markup(&format!("<b>{text}</b>"));
    label.set_xalign(0.0);
    label.add_css_class("hb-library-head");
    label
}

fn empty(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class("hb-empty");
    label
}

fn welcome() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    b.set_halign(gtk::Align::Center);
    b.set_valign(gtk::Align::Center);
    b.add_css_class("hb-welcome");
    let title = gtk::Label::new(Some("hyperb"));
    title.add_css_class("he-v-h1");
    let hint = gtk::Label::new(Some(
        "Enter the address of a hypereffect server, e.g. hyperb://example.com/app.\n\
         Remote servers use https; localhost, IP addresses and .onion use http.",
    ));
    hint.set_justify(gtk::Justification::Center);
    hint.add_css_class("he-v-caption");
    b.append(&title);
    b.append(&hint);
    b.upcast()
}
