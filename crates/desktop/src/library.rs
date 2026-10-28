//! History and favorites ([`hyperb_core::library`]), shared by every tab, and
//! the list of them shown on new tabs and in the toolbar's popover.

use gtk::prelude::*;
use gtk::{glib, pango};
use gtk4 as gtk;
use hyperb_core::identity::Identity;
use hyperb_core::library::{self, Entry, Store};
use hyperb_core::net::Connection;
use std::cell::RefCell;
use std::rc::Rc;
use url::Url;

pub struct Library {
    store: RefCell<Store>,
    /// Called after every change; dropped once they return false.
    listeners: RefCell<Vec<Box<dyn Fn() -> bool>>>,
}

impl Library {
    pub fn open() -> Rc<Library> {
        let dir = Store::default_dir().unwrap_or_else(|| std::env::temp_dir().join("hyperb"));
        Rc::new(Library {
            store: RefCell::new(Store::open(dir)),
            listeners: RefCell::new(Vec::new()),
        })
    }

    pub fn with<T>(&self, f: impl FnOnce(&library::Library) -> T) -> T {
        f(self.store.borrow().library())
    }

    pub fn is_favorite(&self, url: &Url) -> bool {
        self.with(|l| l.is_favorite(url))
    }

    /// Opens a session, from the kept module of a favorite wasm app if
    /// there is one.
    pub fn connect(&self, url: &Url, identity: &Identity) -> Connection {
        self.store.borrow().connect(url, identity)
    }

    /// The addresses that were open when the app last saved them.
    pub fn session(&self) -> Vec<String> {
        self.with(|l| l.session.clone())
    }

    /// Remembers the open addresses. Nothing displays them, so listeners
    /// are not called.
    pub fn save_session(&self, session: Vec<String>) {
        if let Err(e) = self.store.borrow_mut().update(|l, _| l.session = session) {
            eprintln!("hyperb: cannot save the session: {e}");
        }
    }

    pub fn visit(&self, url: &Url) {
        self.update(|s| s.visit(url));
    }

    /// Adds or removes a favorite; a new favorite wasm app is downloaded to
    /// be kept.
    pub fn set_favorite(&self, url: &Url, on: bool) {
        self.update(|s| {
            s.set_favorite(url, on)?;
            if on && s.library().keeps_module(url) {
                s.download(url);
            }
            Ok(())
        });
    }

    pub fn forget(&self, address: &str) {
        self.update(|s| s.update(|l, _| l.forget(address)));
    }

    pub fn clear_history(&self) {
        self.update(|s| s.update(|l, _| l.clear_history()));
    }

    /// Calls `f` after every change until it returns false.
    pub fn subscribe(&self, f: impl Fn() -> bool + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    fn update(&self, f: impl FnOnce(&mut Store) -> std::io::Result<()>) {
        if let Err(e) = f(&mut self.store.borrow_mut()) {
            eprintln!("hyperb: cannot save the library: {e}");
        }
        // Listeners may subscribe others; those join after this round.
        let mut listeners = self.listeners.take();
        listeners.retain(|f| f());
        let mut added = self.listeners.borrow_mut();
        listeners.append(&mut added);
        *added = listeners;
    }
}

/// Favorites, then the most recent `limit` history entries, rebuilt on every
/// change. Activating an entry calls `open` with its address.
pub fn view(lib: &Rc<Library>, limit: usize, open: impl Fn(&str) + 'static) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.add_css_class("hb-library");
    let open: Rc<dyn Fn(&str)> = Rc::new(open);
    fill(&root, lib, limit, &open);
    let weak_root = root.downgrade();
    let weak_lib = Rc::downgrade(lib);
    lib.subscribe(move || {
        let (Some(root), Some(lib)) = (weak_root.upgrade(), weak_lib.upgrade()) else {
            return false;
        };
        // Rebuilding from a button inside the list would destroy the
        // button while its handler runs.
        let open = open.clone();
        glib::idle_add_local_once(move || fill(&root, &lib, limit, &open));
        true
    });
    root
}

fn fill(root: &gtk::Box, lib: &Rc<Library>, limit: usize, open: &Rc<dyn Fn(&str)>) {
    while let Some(c) = root.first_child() {
        root.remove(&c);
    }
    let (favorites, history) = lib.with(|l| {
        let n = l.history.len().min(limit);
        (l.favorites.clone(), l.history[..n].to_vec())
    });
    if favorites.is_empty() && history.is_empty() {
        let empty = gtk::Label::new(Some("No favorites or history yet."));
        empty.add_css_class("he-v-caption");
        root.append(&empty);
        return;
    }
    let weak = Rc::downgrade(lib);
    if !favorites.is_empty() {
        root.append(&heading("Favorites", None));
        root.append(&list(
            &favorites,
            "starred-symbolic",
            "Remove from favorites",
            open,
            {
                let weak = weak.clone();
                move |addr| {
                    if let (Some(lib), Ok(url)) =
                        (weak.upgrade(), hyperb_core::address::normalize(addr))
                    {
                        lib.set_favorite(&url, false);
                    }
                }
            },
        ));
    }
    if !history.is_empty() {
        let clear = gtk::Button::with_label("Clear history");
        clear.add_css_class("flat");
        {
            let weak = weak.clone();
            clear.connect_clicked(move |_| {
                if let Some(lib) = weak.upgrade() {
                    lib.clear_history();
                }
            });
        }
        root.append(&heading("History", Some(&clear)));
        root.append(&list(
            &history,
            "document-open-recent-symbolic",
            "Remove from history",
            open,
            {
                let weak = weak.clone();
                move |addr| {
                    if let Some(lib) = weak.upgrade() {
                        lib.forget(addr);
                    }
                }
            },
        ));
    }
}

fn heading(text: &str, end: Option<&gtk::Button>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    b.add_css_class("hb-library-heading");
    let label = gtk::Label::new(Some(text));
    label.add_css_class("he-v-h3");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    b.append(&label);
    if let Some(e) = end {
        b.append(e);
    }
    b
}

fn list(
    entries: &[Entry],
    icon: &str,
    remove_tip: &str,
    open: &Rc<dyn Fn(&str)>,
    remove: impl Fn(&str) + 'static,
) -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.add_css_class("hb-library-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let remove = Rc::new(remove);
    for e in entries {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.append(&gtk::Image::from_icon_name(icon));
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        let title = gtk::Label::new(Some(&e.title));
        title.set_xalign(0.0);
        title.set_ellipsize(pango::EllipsizeMode::Middle);
        let address = gtk::Label::new(Some(&e.address));
        address.set_xalign(0.0);
        address.set_ellipsize(pango::EllipsizeMode::Middle);
        address.add_css_class("he-v-caption");
        text.append(&title);
        text.append(&address);
        row.append(&text);
        let button = gtk::Button::from_icon_name("window-close-symbolic");
        button.add_css_class("flat");
        button.add_css_class("circular");
        button.set_valign(gtk::Align::Center);
        button.set_tooltip_text(Some(remove_tip));
        {
            let (remove, addr) = (remove.clone(), e.address.clone());
            button.connect_clicked(move |_| remove(&addr));
        }
        row.append(&button);
        let r = gtk::ListBoxRow::new();
        r.set_child(Some(&row));
        r.set_tooltip_text(Some(&e.address));
        list.append(&r);
    }
    let addresses: Vec<String> = entries.iter().map(|e| e.address.clone()).collect();
    let open = open.clone();
    list.connect_row_activated(move |_, row| {
        if let Some(a) = addresses.get(row.index() as usize) {
            open(a);
        }
    });
    list
}
