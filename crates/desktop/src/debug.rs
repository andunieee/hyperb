//! The debugger panel: the wire messages of a session and its live elements.
//!
//! Messages are listed newest first; each row expands to the pretty-printed
//! body. Hovering an element row highlights its widget in the page.

use crate::tab::Tab;
use gtk::prelude::*;
use gtk::{glib, pango};
use gtk4 as gtk;
use hyperb_core::hypereffect::client::Client;
use hyperb_core::hypereffect::Value;
use std::cell::RefCell;
use std::rc::Weak;

/// Messages kept in the log.
const MAX_MESSAGES: usize = 500;
/// Longest body shown when a message is expanded.
const MAX_DETAIL: usize = 20_000;

#[derive(Clone, Copy)]
pub enum Dir {
    Sent,
    Received,
}

pub struct Debugger {
    pub widget: gtk::Box,
    messages: gtk::Box,
    message_count: gtk::Label,
    elements: gtk::Box,
    element_count: gtk::Label,
    /// (id, depth, type) of the rows in `elements`.
    shown: RefCell<Vec<(String, usize, String)>>,
    host: Weak<Tab>,
}

impl Debugger {
    pub fn new(host: Weak<Tab>) -> Self {
        let (messages_pane, messages, message_count) = section("Messages");
        let (elements_pane, elements, element_count) = section("Elements");
        let split = gtk::Paned::new(gtk::Orientation::Vertical);
        split.set_vexpand(true);
        split.set_start_child(Some(&messages_pane));
        split.set_end_child(Some(&elements_pane));
        split.set_resize_start_child(true);
        split.set_resize_end_child(true);
        split.set_shrink_start_child(false);
        split.set_shrink_end_child(false);

        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.add_css_class("hb-debug");
        widget.set_size_request(Self::WIDTH, -1);
        widget.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        widget.append(&split);
        widget.set_visible(false);

        Debugger {
            widget,
            messages,
            message_count,
            elements,
            element_count,
            shown: RefCell::new(Vec::new()),
            host,
        }
    }

    pub const WIDTH: i32 = 360;

    /// Adds one wire line (with or without its line feed) to the log.
    pub fn log(&self, dir: Dir, line: &str) {
        let line = line.trim_end();
        if line.is_empty() {
            return;
        }
        let (tag, json) = line.split_once(' ').unwrap_or((line, ""));
        let body: Option<Value> = serde_json_from_str(json);
        let mut summary = String::new();
        if let Some(Value::Object(m)) = &body {
            match m.get("id").and_then(Value::as_str) {
                Some(id) => summary = id.to_string(),
                None => summary = m.keys().cloned().collect::<Vec<_>>().join(", "),
            }
        }
        let mut detail = match &body {
            Some(v) => serde_json_pretty(v),
            None => line.to_string(),
        };
        if detail.len() > MAX_DETAIL {
            let mut end = MAX_DETAIL;
            while !detail.is_char_boundary(end) {
                end -= 1;
            }
            detail.truncate(end);
            detail.push_str("\n…");
        }

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let (arrow, class) = match dir {
            Dir::Sent => ("→", "hb-sent"),
            Dir::Received => ("←", "hb-recv"),
        };
        let arrow = gtk::Label::new(Some(arrow));
        arrow.add_css_class(class);
        let tag = gtk::Label::new(Some(tag));
        tag.add_css_class("hb-tag");
        let sum = gtk::Label::new(Some(&summary));
        sum.set_hexpand(true);
        sum.set_xalign(0.0);
        sum.set_ellipsize(pango::EllipsizeMode::End);
        sum.add_css_class("dim-label");
        let time = gtk::Label::new(
            glib::DateTime::now_local()
                .ok()
                .and_then(|t| t.format("%T").ok())
                .as_deref(),
        );
        time.add_css_class("dim-label");
        for l in [&arrow, &tag, &sum, &time] {
            head.append(l);
        }

        let body_label = gtk::Label::new(Some(&detail));
        body_label.set_selectable(true);
        body_label.set_xalign(0.0);
        body_label.set_wrap(true);
        body_label.set_wrap_mode(pango::WrapMode::Char);
        body_label.add_css_class("hb-json");

        let row = gtk::Expander::new(None);
        row.set_label_widget(Some(&head));
        row.set_child(Some(&body_label));
        row.add_css_class("hb-msg");

        self.messages.prepend(&row);
        let mut n = 0;
        let mut c = self.messages.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            n += 1;
            if n > MAX_MESSAGES {
                self.messages.remove(&w);
            }
        }
        self.message_count
            .set_text(&n.min(MAX_MESSAGES).to_string());
    }

    /// Refreshes the element list from the client's tree, if its shape changed.
    pub fn sync_elements(&self, client: &Client) {
        let mut rows = Vec::new();
        collect(client, None, 0, &mut rows);
        if *self.shown.borrow() == rows {
            return;
        }
        // Rows are about to go away, so no leave event will clear a highlight.
        if let Some(t) = self.host.upgrade() {
            t.highlight(None);
        }
        while let Some(w) = self.elements.first_child() {
            self.elements.remove(&w);
        }
        for (id, depth, kind) in &rows {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("hb-element");
            row.set_margin_start(*depth as i32 * 12);
            let name = gtk::Label::new(Some(id));
            name.set_xalign(0.0);
            name.set_ellipsize(pango::EllipsizeMode::End);
            name.set_hexpand(true);
            let ty = gtk::Label::new(Some(kind));
            ty.add_css_class("dim-label");
            row.append(&name);
            row.append(&ty);

            let hover = gtk::EventControllerMotion::new();
            {
                let (host, id) = (self.host.clone(), id.clone());
                hover.connect_enter(move |_, _, _| {
                    if let Some(t) = host.upgrade() {
                        t.highlight(Some(&id));
                    }
                });
            }
            {
                let host = self.host.clone();
                hover.connect_leave(move |_| {
                    if let Some(t) = host.upgrade() {
                        t.highlight(None);
                    }
                });
            }
            row.add_controller(hover);
            self.elements.append(&row);
        }
        self.element_count.set_text(&rows.len().to_string());
        *self.shown.borrow_mut() = rows;
    }
}

fn collect(
    client: &Client,
    parent: Option<&str>,
    depth: usize,
    out: &mut Vec<(String, usize, String)>,
) {
    for id in client.children(parent) {
        let kind = client
            .get(id)
            .map(|r| format!("{:?}", r.component).to_lowercase())
            .unwrap_or_default();
        out.push((id.clone(), depth, kind));
        collect(client, Some(id), depth + 1, out);
    }
}

/// A titled, scrolling column. Returns the pane, the column and the count label.
fn section(title: &str) -> (gtk::Box, gtk::Box, gtk::Label) {
    let name = gtk::Label::new(Some(title));
    name.add_css_class("heading");
    let count = gtk::Label::new(Some("0"));
    count.add_css_class("dim-label");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.add_css_class("hb-debug-head");
    head.append(&name);
    head.append(&count);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));

    let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pane.set_size_request(-1, 120);
    pane.append(&head);
    pane.append(&scroll);
    (pane, list, count)
}

fn serde_json_from_str(s: &str) -> Option<Value> {
    hyperb_core::hypereffect::serde_json::from_str(s).ok()
}

fn serde_json_pretty(v: &Value) -> String {
    hyperb_core::hypereffect::serde_json::to_string_pretty(v).unwrap_or_default()
}
