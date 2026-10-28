//! One browser tab: a URL bar, a connection to a hypereffect server, and the
//! rendered session.
//!
//! Typing an address opens a connection; when it opens the tab sends `hello`
//! and from then on feeds every frame to the [`Client`], redrawing once per
//! batch of frames. When the transport closes the last state stays on screen
//! with controls disabled, and a banner offers to reconnect.
//!
//! Every address the user opens goes to the history; the toolbar's star adds
//! it to the favorites, and the popover next to it (and the page of a new
//! tab) lists both. The metadata the app sends names the tab, gives it an
//! icon, and does the same for its entries there.

use crate::clamp::Clamp;
use crate::debug::{Debugger, Dir};
use crate::library::{self, Library};
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
use url::Url;

/// The widest the rendered page gets; the rest of the window is margin.
const CONTENT_WIDTH: i32 = 800;

pub struct Tab {
    /// The notebook page.
    pub page: gtk::Box,
    /// The notebook tab: spinner or icon, title and close button.
    pub tab_label: gtk::Box,
    pub title: gtk::Label,
    pub close_button: gtk::Button,
    spinner: gtk::Spinner,
    icon: gtk::Image,
    url_entry: gtk::Entry,
    star: gtk::ToggleButton,
    banner: gtk::Box,
    banner_text: gtk::Label,
    content: gtk::Stack,
    debug: Debugger,
    session: RefCell<Session>,
    renderer: RefCell<Renderer>,
    quiet: Cell<bool>,
    identity: Identity,
    library: Rc<Library>,
    /// Called after the tab starts connecting to a new address.
    navigated: RefCell<Option<Box<dyn Fn()>>>,
}

struct Session {
    client: Client,
    conn: Option<Connection>,
    url: Option<Url>,
    /// The title and icon of `url`, from its library entry until the app
    /// sends metadata.
    entry: Option<Entry>,
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
    pub fn new(identity: Identity, library: Rc<Library>) -> Rc<Tab> {
        Rc::new_cyclic(|weak: &std::rc::Weak<Tab>| {
            // Tab strip entry.
            let spinner = gtk::Spinner::new();
            spinner.set_visible(false);
            let icon = gtk::Image::new();
            icon.set_pixel_size(16);
            icon.set_visible(false);
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
            tab_label.append(&icon);
            tab_label.append(&title);
            tab_label.append(&close_button);

            // Toolbar.
            let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
            reload.set_tooltip_text(Some("Reconnect (F5)"));
            let url_entry = gtk::Entry::new();
            url_entry.set_hexpand(true);
            url_entry.set_placeholder_text(Some("Server address"));
            url_entry.set_input_purpose(gtk::InputPurpose::Url);
            let star = gtk::ToggleButton::new();
            star.set_icon_name("non-starred-symbolic");
            star.set_tooltip_text(Some("Add to favorites"));
            star.set_sensitive(false);
            let library_button = gtk::MenuButton::new();
            library_button.set_icon_name("document-open-recent-symbolic");
            library_button.set_tooltip_text(Some("Favorites and history"));
            let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            toolbar.add_css_class("hb-toolbar");
            toolbar.append(&reload);
            toolbar.append(&url_entry);
            toolbar.append(&star);
            toolbar.append(&library_button);

            // Favorites and history, in the popover and on the welcome page.
            let open = |popover: Option<gtk::Popover>| {
                let w = weak.clone();
                move |addr: &str| {
                    if let Some(p) = &popover {
                        p.popdown();
                    }
                    if let Some(t) = w.upgrade() {
                        t.navigate(addr);
                    }
                }
            };
            let popover = gtk::Popover::new();
            let popover_scroll = gtk::ScrolledWindow::new();
            popover_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
            popover_scroll.set_propagate_natural_height(true);
            popover_scroll.set_max_content_height(560);
            popover_scroll.set_size_request(420, -1);
            popover_scroll.set_child(Some(&library::view(
                &library,
                50,
                open(Some(popover.clone())),
            )));
            popover.set_child(Some(&popover_scroll));
            library_button.set_popover(Some(&popover));

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
            content.add_named(
                &welcome(library::view(&library, 12, open(None))),
                Some("welcome"),
            );
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
            {
                let w = weak.clone();
                star.connect_toggled(move |b| {
                    if let Some(t) = w.upgrade() {
                        t.set_favorite(b.is_active());
                    }
                });
            }
            {
                let w = weak.clone();
                library.subscribe(move || match w.upgrade() {
                    Some(t) => {
                        t.update_star();
                        let entry = t.session.borrow().entry.clone();
                        if let Some(e) = entry {
                            t.show_icon(&e);
                        }
                        true
                    }
                    None => false,
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
                icon,
                url_entry,
                star,
                banner,
                banner_text,
                content,
                debug,
                session: RefCell::new(Session {
                    client: Client::new(),
                    conn: None,
                    url: None,
                    entry: None,
                    generation: 0,
                    state: State::Idle,
                }),
                renderer: RefCell::new(Renderer::new(weak.clone(), overlay, main)),
                quiet: Cell::new(false),
                identity,
                library,
                navigated: RefCell::new(None),
            }
        })
    }

    /// Calls `f` each time the tab opens an address.
    pub fn on_navigate(&self, f: impl Fn() + 'static) {
        *self.navigated.borrow_mut() = Some(Box::new(f));
    }

    /// The `hyperb://` address the tab shows, if it has opened one.
    pub fn address(&self) -> Option<String> {
        self.session.borrow().url.as_ref().map(address::hyperb)
    }

    pub fn focus_url(&self) {
        self.url_entry.grab_focus();
    }

    /// Connects to what the user typed or picked.
    pub fn navigate(self: &Rc<Self>, input: &str) {
        self.go(address::normalize(input));
    }

    /// Connects to an address from a link, which cannot open local files.
    pub fn open_link(self: &Rc<Self>, input: &str) {
        self.url_entry.set_text(input);
        self.go(address::from_link(input));
    }

    fn go(self: &Rc<Self>, url: Result<Url, String>) {
        match url {
            Ok(url) => {
                self.url_entry.set_text(&address::hyperb(&url));
                self.title
                    .set_tooltip_text(Some(address::hyperb(&url).as_str()));
                self.content.grab_focus();
                self.library.visit(&url);
                let entry = self
                    .library
                    .entry(&url)
                    .unwrap_or_else(|| Entry::new(&url, 0));
                self.show_entry(&entry);
                self.session.borrow_mut().entry = Some(entry);
                self.connect(url);
                if let Some(f) = &*self.navigated.borrow() {
                    f();
                }
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
        let conn = self.library.connect(&url, &self.identity);
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
        self.update_star();

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
        let mut s = self.session.borrow_mut();
        let s = &mut *s;
        match ev {
            Event::Open => {
                let params = s
                    .conn
                    .as_ref()
                    .map(|c| c.params().clone())
                    .unwrap_or_default();
                let hello = s.client.hello_with(&params);
                if let Some(c) = &s.conn {
                    self.debug.log(Dir::Sent, &hello);
                    c.send(hello);
                }
                s.state = State::Open;
            }
            Event::Data(lines) => {
                for l in lines.lines() {
                    self.debug.log(Dir::Received, l);
                }
                s.client.receive(lines.as_bytes())
            }
            Event::Closed(reason) => {
                s.client.close();
                s.conn = None;
                s.state = State::Closed(reason);
            }
        }
    }

    /// Takes the title and icon from the metadata the app sent.
    fn describe(&self) {
        let (url, entry, m) = {
            let mut s = self.session.borrow_mut();
            let s = &mut *s;
            let m = s.client.metadata().clone();
            let (Some(url), Some(entry)) = (&s.url, &mut s.entry) else {
                return;
            };
            if !entry.describe(&m) {
                return;
            }
            (url.clone(), entry.clone(), m)
        };
        self.show_entry(&entry);
        self.library.describe(&url, &m);
    }

    fn show_entry(&self, e: &Entry) {
        self.title.set_text(&e.title);
        self.show_icon(e);
    }

    /// Shows the icon, only while the spinner is hidden (see
    /// `update_chrome`). An `https` icon shows once the library has
    /// downloaded it, which calls this again.
    fn show_icon(&self, e: &Entry) {
        let tex = e.icon.as_deref().and_then(|i| self.library.icon(i));
        self.icon
            .set_visible(tex.is_some() && !self.spinner.is_visible());
        self.icon.set_paintable(tex.as_ref());
    }

    fn set_favorite(&self, on: bool) {
        let url = self.session.borrow().url.clone();
        if let Some(url) = url {
            if self.library.is_favorite(&url) != on {
                self.library.set_favorite(&url, on);
            }
        }
    }

    /// Shows whether the page is a favorite. Toggling the button to match
    /// calls `set_favorite`, which then has nothing to do.
    fn update_star(&self) {
        let url = self.session.borrow().url.clone();
        let on = url.as_ref().is_some_and(|u| self.library.is_favorite(u));
        self.star.set_sensitive(url.is_some());
        self.star.set_active(on);
        self.star.set_icon_name(if on {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        self.star.set_tooltip_text(Some(if on {
            "Remove from favorites"
        } else {
            "Add to favorites"
        }));
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
        if changes.metadata {
            self.describe();
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
        self.icon
            .set_visible(!connecting && self.icon.paintable().is_some());
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

fn welcome(library: gtk::Box) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    b.add_css_class("hb-welcome");
    let title = gtk::Label::new(Some("hyperb"));
    title.add_css_class("he-v-h1");
    let hint = gtk::Label::new(None);
    hint.set_markup(
        r#"A browser for <a href="https://github.com/andunieee/hyperb">hyperb</a> apps."#,
    );
    hint.add_css_class("he-v-caption");
    b.append(&title);
    b.append(&hint);
    library.set_margin_top(24);
    b.append(&library);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_child(Some(&Clamp::new(560, &b)));
    scroll.upcast()
}
