//! hyperb: a browser for hypereffect servers.
//!
//! ```text
//! hyperb-desktop [address...]    # each address opens in a tab
//! ```

mod clamp;
mod debug;
mod render;
mod tab;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4 as gtk;
use hyperb_core::identity::Identity;
use std::cell::RefCell;
use std::rc::Rc;
use tab::Tab;

const APP_ID: &str = "com.hyperb.app";

fn main() -> glib::ExitCode {
    let urls: Vec<String> = std::env::args().skip(1).collect();
    let identity = Identity::load_default().unwrap_or_else(|e| {
        eprintln!("hyperb: cannot load identity ({e}); using a temporary one");
        Identity::generate()
    });
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|_| load_css());
    app.connect_activate(move |app| Browser::open(app, &urls, &identity));
    app.set_accels_for_action("win.new-tab", &["<Ctrl>t"]);
    app.set_accels_for_action("win.close-tab", &["<Ctrl>w"]);
    app.set_accels_for_action("win.focus-url", &["<Ctrl>l", "F6"]);
    app.set_accels_for_action("win.reload", &["F5", "<Ctrl>r"]);
    // Addresses are ours, not GTK's.
    app.run_with_args::<&str>(&[])
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

struct Browser {
    window: gtk::ApplicationWindow,
    notebook: gtk::Notebook,
    tabs: RefCell<Vec<Rc<Tab>>>,
    identity: Identity,
}

impl Browser {
    fn open(app: &gtk::Application, urls: &[String], identity: &Identity) {
        let notebook = gtk::Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_tab_pos(gtk::PositionType::Left);
        notebook.set_show_border(false);
        let new_tab = gtk::Button::from_icon_name("list-add-symbolic");
        new_tab.add_css_class("flat");
        new_tab.set_tooltip_text(Some("New tab (Ctrl+T)"));
        notebook.set_action_widget(&new_tab, gtk::PackType::End);

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
            identity: identity.clone(),
        });

        {
            let b = Rc::downgrade(&b);
            new_tab.connect_clicked(move |_| {
                if let Some(b) = b.upgrade() {
                    b.add_tab(None);
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
        action("new-tab", |b, _| b.add_tab(None));
        action("close-tab", |b, t| b.close_tab(t));
        action("focus-url", |_, t| t.focus_url());
        action("reload", |_, t| t.reload());

        if urls.is_empty() {
            b.add_tab(None);
        }
        for u in urls {
            b.add_tab(Some(u));
        }
        // The browser lives as long as its window.
        let keep = RefCell::new(Some(b));
        window.connect_destroy(move |_| {
            keep.borrow_mut().take();
        });
        window.present();
    }

    fn add_tab(self: &Rc<Self>, url: Option<&str>) {
        let t = Tab::new(self.identity.clone());
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
        self.tabs.borrow_mut().push(t.clone());
        self.notebook.set_current_page(Some(i));
        match url {
            Some(u) => t.navigate(u),
            None => t.focus_url(),
        }
    }

    fn close_tab(&self, t: &Rc<Tab>) {
        self.tabs.borrow_mut().retain(|x| !Rc::ptr_eq(x, t));
        if let Some(i) = self.notebook.page_num(&t.page) {
            self.notebook.remove_page(Some(i));
        }
        if self.tabs.borrow().is_empty() {
            self.window.close();
        }
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
