//! hyperb: a browser for hypereffect servers.
//!
//! ```text
//! hyperb-desktop [address...]            # each address opens in a tab
//! hyperb-desktop --links [address...]    # the same, for links from other apps
//! ```
//!
//! One instance runs per user: launching it again opens the addresses in the
//! running window. The system opens `hyperb://` and `web+hyperb://` links
//! with `--links` (see `com.hyperb.app.desktop`), which refuses local files.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod clamp;
mod debug;
mod library;
mod render;
mod tab;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4 as gtk;
use hyperb_core::identity::Identity;
use library::Library;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use tab::Tab;

const APP_ID: &str = "com.hyperb.app";

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.add_main_option(
        "links",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open the addresses as links from another app: local files are refused",
        None,
    );
    // Only the first instance gets here; later ones pass their command
    // line to it and exit.
    let state: Rc<RefCell<Option<Shared>>> = Rc::default();
    {
        let state = state.clone();
        app.connect_startup(move |_| {
            load_css();
            set_app_icon();
            let identity = Identity::load_default().unwrap_or_else(|e| {
                eprintln!("hyperb: cannot load identity ({e}); using a temporary one");
                Identity::generate()
            });
            *state.borrow_mut() = Some((identity, Library::open()));
        });
    }
    let window: Rc<RefCell<Weak<Browser>>> = Rc::default();
    app.connect_command_line(move |app, cmd| {
        let links = cmd.options_dict().contains("links");
        let urls: Vec<String> = cmd
            .arguments()
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .filter(|a| !a.is_empty())
            .collect();
        let b = window.borrow().upgrade();
        match b {
            Some(b) => {
                if urls.is_empty() {
                    b.add_tab(None, false);
                }
                b.open_all(&urls, links);
                b.window.present();
            }
            None => {
                let (identity, library) = state.borrow().clone().expect("started up");
                let restore = urls.is_empty();
                let session = library.session();
                let b = Browser::open(app, identity, library);
                if restore {
                    // Nothing was asked for: reopen what was open last time.
                    b.open_all(&session, false);
                } else {
                    b.open_all(&urls, links);
                }
                *window.borrow_mut() = Rc::downgrade(&b);
            }
        }
        glib::ExitCode::SUCCESS
    });
    app.set_accels_for_action("win.new-tab", &["<Ctrl>t"]);
    app.set_accels_for_action("win.close-tab", &["<Ctrl>w"]);
    app.set_accels_for_action("win.focus-url", &["<Ctrl>l", "F6"]);
    app.set_accels_for_action("win.reload", &["F5", "<Ctrl>r"]);
    app.run()
}

/// What every window of the instance shares.
type Shared = (Identity, Rc<Library>);

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Names the window icon. An installed app finds it in the user's icon theme
/// (`just desktop-install`); a dev run falls back to the copy in this crate.
fn set_app_icon() {
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display)
            .add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/icons"));
    }
    gtk::Window::set_default_icon_name(APP_ID);
}

struct Browser {
    window: gtk::ApplicationWindow,
    notebook: gtk::Notebook,
    tabs: RefCell<Vec<Rc<Tab>>>,
    identity: Identity,
    library: Rc<Library>,
}

impl Browser {
    fn open(app: &gtk::Application, identity: Identity, library: Rc<Library>) -> Rc<Browser> {
        let notebook = gtk::Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_tab_pos(gtk::PositionType::Left);
        notebook.set_show_border(false);
        let new_tab = gtk::Button::from_icon_name("list-add-symbolic");
        new_tab.add_css_class("flat");
        new_tab.set_tooltip_text(Some("New tab (Ctrl+T)"));
        let settings = gtk::Button::from_icon_name("emblem-system-symbolic");
        settings.add_css_class("flat");
        settings.set_tooltip_text(Some("Settings"));
        let ends = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ends.append(&settings);
        ends.append(&new_tab);
        notebook.set_action_widget(&ends, gtk::PackType::End);

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("hyperb")
            .default_width(1440)
            .default_height(760)
            .child(&notebook)
            .build();
        let b = Rc::new(Browser {
            window: window.clone(),
            notebook: notebook.clone(),
            tabs: RefCell::new(Vec::new()),
            identity,
            library,
        });

        {
            let b = Rc::downgrade(&b);
            settings.connect_clicked(move |_| {
                if let Some(b) = b.upgrade() {
                    b.show_settings();
                }
            });
        }
        {
            let b = Rc::downgrade(&b);
            new_tab.connect_clicked(move |_| {
                if let Some(b) = b.upgrade() {
                    b.add_tab(None, false);
                }
            });
        }
        {
            let b = Rc::downgrade(&b);
            notebook.connect_switch_page(move |_, page, _| {
                if let Some(b) = b.upgrade() {
                    if let Some(t) = b.tab_for(page) {
                        b.window.set_title(Some(&window_title(&t)));
                    }
                }
            });
        }
        let action = |name: &str, f: fn(&Rc<Browser>, &Rc<Tab>)| {
            let a = gio::SimpleAction::new(name, None);
            let b = Rc::downgrade(&b);
            a.connect_activate(move |_, _| {
                let Some(b) = b.upgrade() else { return };
                if let Some(t) = b.current() {
                    f(&b, &t);
                }
            });
            window.add_action(&a);
        };
        action("new-tab", |b, _| b.add_tab(None, false));
        action("close-tab", |b, t| b.close_tab(t));
        action("focus-url", |_, t| t.focus_url());
        action("reload", |_, t| t.reload());

        // The browser lives as long as its window.
        let keep = RefCell::new(Some(b.clone()));
        window.connect_destroy(move |_| {
            keep.borrow_mut().take();
        });
        window.present();
        b
    }

    fn show_settings(&self) {
        let key = gtk::Label::new(Some(&self.identity.public().to_string()));
        key.set_selectable(true);
        key.set_wrap(true);
        key.set_xalign(0.0);
        key.add_css_class("monospace");
        let heading = gtk::Label::new(Some("Your key"));
        heading.set_xalign(0.0);
        heading.add_css_class("heading");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.append(&heading);
        content.append(&key);
        // Window managers without decorations (tiling ones) give no way to
        // close the window, so offer a button and Escape too.
        let close = gtk::Button::with_label("Close");
        close.set_halign(gtk::Align::End);
        close.set_margin_top(10);
        close.set_action_name(Some("window.close"));
        content.append(&close);
        let window = gtk::Window::builder()
            .title("Settings")
            .transient_for(&self.window)
            .modal(true)
            .resizable(false)
            .child(&content)
            .build();
        let escape = gtk::ShortcutController::new();
        escape.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("Escape"),
            Some(gtk::NamedAction::new("window.close")),
        ));
        window.add_controller(escape);
        window.present();
    }

    /// Opens each address in a new tab, or an empty tab if there are none
    /// and no tab yet.
    fn open_all(self: &Rc<Self>, urls: &[String], links: bool) {
        if urls.is_empty() && self.tabs.borrow().is_empty() {
            self.add_tab(None, false);
        }
        for u in urls {
            self.add_tab(Some(u), links);
        }
    }

    fn add_tab(self: &Rc<Self>, url: Option<&str>, link: bool) {
        let t = Tab::new(self.identity.clone(), self.library.clone());
        let i = self.notebook.append_page(&t.page, Some(&t.tab_label));
        self.notebook.set_tab_reorderable(&t.page, true);
        {
            let b = Rc::downgrade(self);
            let tw = Rc::downgrade(&t);
            t.close_button.connect_clicked(move |_| {
                if let (Some(b), Some(t)) = (b.upgrade(), tw.upgrade()) {
                    b.close_tab(&t);
                }
            });
        }
        {
            let b = Rc::downgrade(self);
            let tw = Rc::downgrade(&t);
            t.title.connect_label_notify(move |_| {
                let (Some(b), Some(t)) = (b.upgrade(), tw.upgrade()) else {
                    return;
                };
                if b.current().is_some_and(|c| Rc::ptr_eq(&c, &t)) {
                    b.window.set_title(Some(&window_title(&t)));
                }
            });
        }
        {
            let b = Rc::downgrade(self);
            t.on_navigate(move || {
                if let Some(b) = b.upgrade() {
                    b.save_session();
                }
            });
        }
        self.tabs.borrow_mut().push(t.clone());
        self.notebook.set_current_page(Some(i));
        match url {
            Some(u) if link => t.open_link(u),
            Some(u) => t.navigate(u),
            None => t.focus_url(),
        }
    }

    /// Remembers the open addresses, to reopen them on the next start.
    fn save_session(&self) {
        let session = self.tabs.borrow().iter().filter_map(|t| t.address()).collect();
        self.library.save_session(session);
    }

    fn close_tab(self: &Rc<Self>, t: &Rc<Tab>) {
        self.tabs.borrow_mut().retain(|x| !Rc::ptr_eq(x, t));
        if let Some(i) = self.notebook.page_num(&t.page) {
            self.notebook.remove_page(Some(i));
        }
        // Closing the last tab leaves an empty one; only closing the window
        // quits.
        if self.tabs.borrow().is_empty() {
            self.add_tab(None, false);
        }
        self.save_session();
    }

    fn current(&self) -> Option<Rc<Tab>> {
        let page = self.notebook.nth_page(self.notebook.current_page())?;
        self.tab_for(&page)
    }

    fn tab_for(&self, page: &gtk::Widget) -> Option<Rc<Tab>> {
        let tabs = self.tabs.borrow();
        tabs.iter()
            .find(|t| t.page.upcast_ref::<gtk::Widget>() == page)
            .cloned()
    }
}

fn window_title(t: &Tab) -> String {
    format!("{} — hyperb", t.title.label())
}
