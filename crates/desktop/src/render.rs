//! Draws a hypereffect element tree with GTK widgets.
//!
//! The [`Client`] keeps the tree, the signals and the expressions; the
//! renderer only turns its [`Changes`] into widget operations. Each element
//! gets an `outer` widget, which its parent places and shows or hides, and
//! containers get a box (or, for `tabs`, a stack) that holds their children.
//!
//! Widget callbacks go to the [`Tab`] that owns the session. While the
//! renderer is applying changes the tab is "quiet", and callbacks ignore the
//! signals that GTK emits for programmatic updates.

use crate::tab::Tab;
use base64::Engine;
use gtk::prelude::*;
use gtk::{gdk, glib, pango};
use gtk4 as gtk;
use hyperb_core::hypereffect::client::{Changes, Client, Resolved};
use hyperb_core::hypereffect::{json, Component, Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// How long an input waits after the last keystroke before running onChange.
const CHANGE_IDLE: Duration = Duration::from_millis(700);
/// The same, for slider changes made with the keyboard.
const SLIDER_IDLE: Duration = Duration::from_millis(400);

pub struct Renderer {
    host: Weak<Tab>,
    overlay: gtk::Overlay,
    main: gtk::Box,
    widgets: HashMap<String, W>,
    highlighted: Option<String>,
}

struct W {
    component: Component,
    outer: gtk::Widget,
    /// Where children go, for containers other than `tabs`.
    container: Option<gtk::Box>,
    kind: Kind,
}

enum Kind {
    Plain,
    Card {
        title: gtk::Label,
    },
    List {
        empty: gtk::Label,
    },
    Tabs {
        stack: gtk::Stack,
    },
    Dialog {
        header: gtk::Box,
        title: gtk::Label,
        close: gtk::Button,
    },
    Text(gtk::Label),
    Notice {
        icon: gtk::Image,
        title: gtk::Label,
        text: gtk::Label,
    },
    Badge(gtk::Label),
    Image {
        pic: gtk::Picture,
        alt: gtk::Label,
        src: RefCell<Option<String>>,
    },
    Divider(gtk::Separator),
    Progress {
        label: gtk::Label,
        bar: gtk::ProgressBar,
        spinner: gtk::Spinner,
        pulse: Rc<Cell<u64>>,
        pulsing: Cell<bool>,
    },
    Button(gtk::Button),
    Input {
        label: gtk::Label,
        entry: gtk::Entry,
        multi: gtk::TextView,
        multi_scroll: gtk::ScrolledWindow,
        error: gtk::Label,
        state: Rc<InputState>,
    },
    Checkbox {
        check: gtk::CheckButton,
        switch: gtk::Switch,
        switch_label: gtk::Label,
        switch_row: gtk::Box,
    },
    Select {
        label: gtk::Label,
        holder: gtk::Box,
        /// The variant and options the control was built for.
        built: RefCell<Option<SelectKey>>,
        control: RefCell<SelectControl>,
    },
    Slider {
        label: gtk::Label,
        scale: gtk::Scale,
        state: Rc<SliderState>,
    },
}

#[derive(Default)]
struct InputState {
    variant: RefCell<String>,
    /// An edit has not been reported with onChange yet.
    pending: Cell<bool>,
    idle: RefCell<Option<glib::SourceId>>,
}

#[derive(Default)]
struct SliderState {
    step: Cell<f64>,
    pending: Cell<bool>,
    pressed: Cell<bool>,
    idle: RefCell<Option<glib::SourceId>>,
}

/// A select's variant and its options as (value, label).
type SelectKey = (String, Vec<(String, String)>);

enum SelectControl {
    None,
    Dropdown(gtk::DropDown),
    /// Grouped check buttons (radio) or toggle buttons (segmented).
    Choices(Vec<gtk::Widget>),
}

impl Renderer {
    pub fn new(host: Weak<Tab>, overlay: gtk::Overlay, main: gtk::Box) -> Self {
        Renderer {
            host,
            overlay,
            main,
            widgets: HashMap::new(),
            highlighted: None,
        }
    }

    pub fn apply(&mut self, client: &Client, changes: Changes) {
        for id in &changes.removed {
            if let Some(w) = self.widgets.remove(id) {
                detach(&w.outer);
            }
        }
        let mut resync: BTreeSet<String> = changes.reordered.clone();
        for id in &changes.created {
            let Some(r) = client.get(id) else { continue };
            if let Some(old) = self.widgets.remove(id) {
                detach(&old.outer);
            }
            let w = self.build(id, r.component);
            self.widgets.insert(id.clone(), w);
            resync.insert(r.parent.unwrap_or_default());
        }
        for id in &resync {
            self.sync_children(client, id);
        }
        let updated: Vec<String> = if changes.connection || changes.reset {
            self.widgets.keys().cloned().collect()
        } else {
            changes.updated.union(&changes.created).cloned().collect()
        };
        for id in &updated {
            self.update(client, id);
        }
        for id in &resync {
            self.layout(client, id);
        }
    }

    /// Outlines the widget of element `id`; `None` clears the outline.
    pub fn highlight(&mut self, id: Option<&str>) {
        if let Some(w) = self
            .highlighted
            .take()
            .and_then(|old| self.widgets.get(&old))
        {
            w.outer.remove_css_class("hb-hl");
        }
        if let Some((id, w)) = id.and_then(|id| self.widgets.get(id).map(|w| (id, w))) {
            w.outer.add_css_class("hb-hl");
            self.highlighted = Some(id.to_string());
        }
    }

    /// The topmost open dialog, for Escape.
    pub fn top_dialog(&self, client: &Client) -> Option<String> {
        client
            .children(None)
            .iter()
            .rev()
            .find(|id| {
                self.widgets
                    .get(*id)
                    .is_some_and(|w| w.component == Component::Dialog && w.outer.is_visible())
            })
            .cloned()
    }

    // --- building ------------------------------------------------------------

    fn build(&self, id: &str, component: Component) -> W {
        let h = self.host.clone();
        let id = id.to_string();
        let (outer, container, kind): (gtk::Widget, Option<gtk::Box>, Kind) = match component {
            Component::Column | Component::Tab => {
                let b = vbox(0);
                if component == Component::Tab {
                    b.add_css_class("he-tab");
                }
                (b.clone().upcast(), Some(b), Kind::Plain)
            }
            Component::Row => {
                let outer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let inner = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                inner.set_hexpand(true);
                outer.append(&inner);
                (outer.upcast(), Some(inner), Kind::Plain)
            }
            Component::Card => {
                let outer = vbox(10);
                outer.add_css_class("he-card");
                let title = label("he-card-title");
                let inner = vbox(0);
                outer.append(&title);
                outer.append(&inner);
                (outer.upcast(), Some(inner), Kind::Card { title })
            }
            Component::List => {
                let outer = vbox(0);
                outer.add_css_class("he-list");
                let items = vbox(0);
                items.add_css_class("he-list-items");
                let empty = label("he-list-empty");
                outer.append(&items);
                outer.append(&empty);
                (outer.upcast(), Some(items), Kind::List { empty })
            }
            Component::Tabs => {
                let outer = vbox(10);
                let stack = gtk::Stack::new();
                stack.set_transition_type(gtk::StackTransitionType::Crossfade);
                stack.set_vhomogeneous(false);
                let switcher = gtk::StackSwitcher::new();
                switcher.set_stack(Some(&stack));
                switcher.set_halign(gtk::Align::Start);
                outer.append(&switcher);
                outer.append(&stack);
                stack.connect_visible_child_name_notify(move |st| {
                    let Some(name) = st.visible_child_name() else {
                        return;
                    };
                    with_host(&h, |t| t.select_tab(&id, &name));
                });
                (outer.upcast(), None, Kind::Tabs { stack })
            }
            Component::Dialog => {
                let backdrop = vbox(0);
                backdrop.add_css_class("he-backdrop");
                backdrop.set_hexpand(true);
                backdrop.set_vexpand(true);
                let card = vbox(12);
                card.add_css_class("he-dialog-card");
                card.set_halign(gtk::Align::Center);
                card.set_valign(gtk::Align::Center);
                card.set_vexpand(true);
                card.set_size_request(380, -1);
                let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let title = label("he-dialog-title");
                title.set_hexpand(true);
                let close = gtk::Button::from_icon_name("window-close-symbolic");
                close.add_css_class("flat");
                close.set_tooltip_text(Some("Close"));
                close.connect_clicked(move |_| {
                    with_host(&h, |t| t.input(&id, None, Some("onDismiss")))
                });
                header.append(&title);
                header.append(&close);
                let scroll = gtk::ScrolledWindow::new();
                scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
                scroll.set_propagate_natural_height(true);
                scroll.set_max_content_height(560);
                let content = vbox(0);
                scroll.set_child(Some(&content));
                card.append(&header);
                card.append(&scroll);
                backdrop.append(&card);
                (
                    backdrop.upcast(),
                    Some(content),
                    Kind::Dialog {
                        header,
                        title,
                        close,
                    },
                )
            }
            Component::Text => {
                let l = label("he-text");
                l.set_selectable(true);
                (l.clone().upcast(), None, Kind::Text(l))
            }
            Component::Notice => {
                let outer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                outer.add_css_class("he-notice");
                let icon = gtk::Image::new();
                icon.set_valign(gtk::Align::Start);
                let col = vbox(2);
                col.set_hexpand(true);
                let title = label("he-notice-title");
                let text = label("he-notice-text");
                col.append(&title);
                col.append(&text);
                outer.append(&icon);
                outer.append(&col);
                (outer.upcast(), None, Kind::Notice { icon, title, text })
            }
            Component::Badge => {
                let l = gtk::Label::new(None);
                l.add_css_class("he-badge");
                (l.clone().upcast(), None, Kind::Badge(l))
            }
            Component::Image => {
                let outer = vbox(0);
                let pic = gtk::Picture::new();
                pic.set_can_shrink(true);
                let alt = label("he-image-alt");
                outer.append(&pic);
                outer.append(&alt);
                let src = RefCell::new(None);
                (outer.upcast(), None, Kind::Image { pic, alt, src })
            }
            Component::Divider => {
                let s = gtk::Separator::new(gtk::Orientation::Horizontal);
                (s.clone().upcast(), None, Kind::Divider(s))
            }
            Component::Progress => {
                let outer = vbox(6);
                let label = label("he-label");
                let bar = gtk::ProgressBar::new();
                let spinner = gtk::Spinner::new();
                spinner.set_halign(gtk::Align::Start);
                outer.append(&label);
                outer.append(&bar);
                outer.append(&spinner);
                let kind = Kind::Progress {
                    label,
                    bar,
                    spinner,
                    pulse: Rc::new(Cell::new(0)),
                    pulsing: Cell::new(false),
                };
                (outer.upcast(), None, kind)
            }
            Component::Button => {
                let b = gtk::Button::new();
                b.connect_clicked(move |_| with_host(&h, |t| t.input(&id, None, Some("onPress"))));
                (b.clone().upcast(), None, Kind::Button(b))
            }
            Component::Input => self.build_input(id),
            Component::Checkbox => {
                let outer = vbox(0);
                let check = gtk::CheckButton::new();
                let switch_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let switch = gtk::Switch::new();
                switch.set_valign(gtk::Align::Center);
                let switch_label = label("he-switch-label");
                switch_row.append(&switch);
                switch_row.append(&switch_label);
                outer.append(&check);
                outer.append(&switch_row);
                {
                    let (h, id) = (h.clone(), id.clone());
                    check.connect_toggled(move |c| {
                        with_host(&h, |t| {
                            t.input(&id, Some(json!(c.is_active())), Some("onChange"))
                        })
                    });
                }
                switch.connect_active_notify(move |s| {
                    with_host(&h, |t| {
                        t.input(&id, Some(json!(s.is_active())), Some("onChange"))
                    })
                });
                let kind = Kind::Checkbox {
                    check,
                    switch,
                    switch_label,
                    switch_row,
                };
                (outer.upcast(), None, kind)
            }
            Component::Select => {
                let outer = vbox(6);
                let label = label("he-label");
                let holder = vbox(0);
                outer.append(&label);
                outer.append(&holder);
                let kind = Kind::Select {
                    label,
                    holder,
                    built: RefCell::new(None),
                    control: RefCell::new(SelectControl::None),
                };
                (outer.upcast(), None, kind)
            }
            Component::Slider => self.build_slider(id),
        };
        outer.add_css_class(&format!("he-{}", component.name()));
        W {
            component,
            outer,
            container,
            kind,
        }
    }

    fn build_input(&self, id: String) -> (gtk::Widget, Option<gtk::Box>, Kind) {
        let h = self.host.clone();
        let outer = vbox(6);
        let label = label("he-label");
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        let multi = gtk::TextView::new();
        multi.set_wrap_mode(gtk::WrapMode::WordChar);
        multi.set_top_margin(6);
        multi.set_bottom_margin(6);
        multi.set_left_margin(8);
        multi.set_right_margin(8);
        let multi_scroll = gtk::ScrolledWindow::new();
        multi_scroll.add_css_class("he-multiline");
        multi_scroll.set_min_content_height(84);
        multi_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        multi_scroll.set_child(Some(&multi));
        let error = label_with("he-error");
        outer.append(&label);
        outer.append(&entry);
        outer.append(&multi_scroll);
        outer.append(&error);
        let state = Rc::new(InputState::default());

        {
            let (h, id, st) = (h.clone(), id.clone(), state.clone());
            entry.connect_changed(move |e| {
                let text = e.text();
                let v = if *st.variant.borrow() == "number" {
                    parse_number(&text)
                } else {
                    Value::String(text.to_string())
                };
                input_edited(&h, &id, &st, v);
            });
        }
        {
            let (h, id, st) = (h.clone(), id.clone(), state.clone());
            multi.buffer().connect_changed(move |b| {
                let text = b.text(&b.start_iter(), &b.end_iter(), false);
                input_edited(&h, &id, &st, Value::String(text.to_string()));
            });
        }
        {
            let (h, id, st) = (h.clone(), id.clone(), state.clone());
            entry.connect_activate(move |_| {
                fire_change(&h, &id, &st);
                with_host(&h, |t| t.input(&id, None, Some("onSubmit")));
            });
        }
        for w in [entry.upcast_ref::<gtk::Widget>(), multi.upcast_ref()] {
            let (h, id, st) = (h.clone(), id.clone(), state.clone());
            let focus = gtk::EventControllerFocus::new();
            focus.connect_leave(move |_| fire_change(&h, &id, &st));
            w.add_controller(focus);
        }
        let kind = Kind::Input {
            label,
            entry,
            multi,
            multi_scroll,
            error,
            state,
        };
        (outer.upcast(), None, kind)
    }

    fn build_slider(&self, id: String) -> (gtk::Widget, Option<gtk::Box>, Kind) {
        let h = self.host.clone();
        let outer = vbox(4);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let label = label("he-label");
        label.set_hexpand(true);
        let out = label_with("he-slider-value");
        head.append(&label);
        head.append(&out);
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        scale.set_draw_value(false);
        outer.append(&head);
        outer.append(&scale);
        let state = Rc::new(SliderState::default());
        state.step.set(1.0);

        {
            let (h, id, st) = (h.clone(), id.clone(), state.clone());
            scale.connect_value_changed(move |sc| {
                let step = st.step.get();
                let min = sc.adjustment().lower();
                let raw = sc.value();
                let v = round_to(min + ((raw - min) / step).round() * step, step);
                if (v - raw).abs() > 1e-9 {
                    sc.set_value(v);
                    return;
                }
                out.set_text(&format_number(v, step));
                with_host(&h, |t| {
                    t.input(&id, Some(number(v)), None);
                    st.pending.set(true);
                    if !st.pressed.get() {
                        restart(&st.idle, SLIDER_IDLE, {
                            let (h, id, st) = (h.clone(), id.clone(), st.clone());
                            move || {
                                st.idle.take();
                                slider_change(&h, &id, &st);
                            }
                        });
                    }
                });
            });
        }
        // Run onChange when the pointer is released, not while dragging.
        let legacy = gtk::EventControllerLegacy::new();
        legacy.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let st = state.clone();
            legacy.connect_event(move |_, ev| {
                match ev.event_type() {
                    gdk::EventType::ButtonPress | gdk::EventType::TouchBegin => {
                        st.pressed.set(true)
                    }
                    gdk::EventType::ButtonRelease
                    | gdk::EventType::TouchEnd
                    | gdk::EventType::TouchCancel => {
                        st.pressed.set(false);
                        slider_change(&h, &id, &st);
                    }
                    _ => {}
                }
                glib::Propagation::Proceed
            });
        }
        scale.add_controller(legacy);
        (
            outer.upcast(),
            None,
            Kind::Slider {
                label,
                scale,
                state,
            },
        )
    }

    fn build_select(
        &self,
        id: &str,
        holder: &gtk::Box,
        variant: &str,
        opts: &[(String, String)],
    ) -> SelectControl {
        while let Some(c) = holder.first_child() {
            holder.remove(&c);
        }
        let values: Rc<Vec<String>> = Rc::new(opts.iter().map(|o| o.0.clone()).collect());
        let choose = {
            let (h, id) = (self.host.clone(), id.to_string());
            move |v: &str| with_host(&h, |t| t.input(&id, Some(json!(v)), Some("onChange")))
        };
        match variant {
            "radio" | "segmented" => {
                let radio = variant == "radio";
                let group = gtk::Box::new(
                    if radio {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    },
                    if radio { 2 } else { 0 },
                );
                if !radio {
                    group.add_css_class("linked");
                    group.set_halign(gtk::Align::Start);
                }
                let mut choices: Vec<gtk::Widget> = Vec::new();
                for (value, label) in opts {
                    let w: gtk::Widget = if radio {
                        let cb = gtk::CheckButton::with_label(label);
                        if let Some(first) = choices.first() {
                            cb.set_group(first.downcast_ref::<gtk::CheckButton>());
                        }
                        let (choose, value) = (choose.clone(), value.clone());
                        cb.connect_toggled(move |b| {
                            if b.is_active() {
                                choose(&value)
                            }
                        });
                        cb.upcast()
                    } else {
                        let tb = gtk::ToggleButton::with_label(label);
                        if let Some(first) = choices.first() {
                            tb.set_group(first.downcast_ref::<gtk::ToggleButton>());
                        }
                        let (choose, value) = (choose.clone(), value.clone());
                        tb.connect_toggled(move |b| {
                            if b.is_active() {
                                choose(&value)
                            }
                        });
                        tb.upcast()
                    };
                    group.append(&w);
                    choices.push(w);
                }
                holder.append(&group);
                SelectControl::Choices(choices)
            }
            _ => {
                let labels: Vec<&str> = opts.iter().map(|o| o.1.as_str()).collect();
                let dd = gtk::DropDown::from_strings(&labels);
                dd.set_selected(gtk::INVALID_LIST_POSITION);
                dd.set_halign(gtk::Align::Start);
                dd.connect_selected_notify(move |dd| {
                    if let Some(v) = values.get(dd.selected() as usize) {
                        choose(v)
                    }
                });
                holder.append(&dd);
                SelectControl::Dropdown(dd)
            }
        }
    }

    // --- tree structure ------------------------------------------------------

    fn sync_children(&self, client: &Client, id: &str) {
        if id.is_empty() {
            let (dialogs, others): (Vec<&String>, Vec<&String>) =
                client.children(None).iter().partition(|c| {
                    client
                        .get(c)
                        .is_some_and(|r| r.component == Component::Dialog)
                });
            sync_box(&self.main, &self.outers(&others));
            let dialogs = self.outers(&dialogs);
            let ov: &gtk::Widget = self.overlay.upcast_ref();
            for d in &dialogs {
                if d.parent().as_ref() != Some(ov) {
                    detach(d);
                    self.overlay.add_overlay(d);
                }
            }
            let main_child = self.overlay.child();
            let mut c = self.overlay.first_child();
            while let Some(w) = c {
                c = w.next_sibling();
                if Some(&w) != main_child.as_ref() && !dialogs.contains(&w) {
                    self.overlay.remove_overlay(&w);
                }
            }
            return;
        }
        let Some(w) = self.widgets.get(id) else {
            return;
        };
        let kids: Vec<&String> = client.children(Some(id)).iter().collect();
        let outers = self.outers(&kids);
        match (&w.kind, &w.container) {
            (Kind::Tabs { stack }, _) => {
                let mut current = Vec::new();
                let mut c = stack.first_child();
                while let Some(x) = c {
                    c = x.next_sibling();
                    current.push(x);
                }
                if current == outers {
                    return;
                }
                let selected = stack.visible_child_name();
                for x in current {
                    stack.remove(&x);
                }
                for (cid, o) in kids.iter().zip(&outers) {
                    detach(o);
                    stack.add_titled(o, Some(cid.as_str()), cid.as_str());
                }
                if let Some(name) = selected {
                    if stack.child_by_name(&name).is_some() {
                        stack.set_visible_child_name(&name);
                    }
                }
            }
            (_, Some(b)) => sync_box(b, &outers),
            _ => {}
        }
    }

    fn outers(&self, ids: &[&String]) -> Vec<gtk::Widget> {
        ids.iter()
            .filter_map(|id| self.widgets.get(*id).map(|w| w.outer.clone()))
            .collect()
    }

    /// Spacing and child alignment of a container.
    fn layout(&self, client: &Client, id: &str) {
        let (container, attrs, component) = if id.is_empty() {
            (self.main.clone(), Map::new(), Component::Column)
        } else {
            let Some(w) = self.widgets.get(id) else {
                return;
            };
            let Some(c) = &w.container else { return };
            let Some(r) = client.get(id) else { return };
            (c.clone(), r.attrs, w.component)
        };
        let gap = st(&attrs, "gap").unwrap_or(match component {
            Component::Row => "small",
            Component::List => "none",
            _ => "medium",
        });
        container.set_spacing(match gap {
            "none" => 0,
            "small" => 6,
            "large" => 24,
            _ => 12,
        });
        let row = component == Component::Row;
        let align = st(&attrs, "align").unwrap_or(if row { "center" } else { "stretch" });
        let justify = st(&attrs, "justify").unwrap_or("start");
        let kids: Vec<(&W, String)> = client
            .children(if id.is_empty() { None } else { Some(id) })
            .iter()
            .filter_map(|c| self.widgets.get(c).map(|w| (w, c.clone())))
            .filter(|(w, _)| w.component != Component::Dialog)
            .collect();
        if row {
            container.set_halign(match justify {
                "center" => gtk::Align::Center,
                "end" => gtk::Align::End,
                _ => gtk::Align::Fill,
            });
        }
        let n = kids.len();
        for (i, (w, cid)) in kids.iter().enumerate() {
            let o = &w.outer;
            if let Kind::Divider(s) = &w.kind {
                s.set_orientation(if row {
                    gtk::Orientation::Vertical
                } else {
                    gtk::Orientation::Horizontal
                });
            }
            if row {
                o.set_valign(to_align(align));
                let grows = matches!(w.component, Component::Input | Component::Slider);
                if justify == "between" {
                    o.set_hexpand(true);
                    o.set_halign(if i == 0 {
                        gtk::Align::Start
                    } else if i + 1 == n {
                        gtk::Align::End
                    } else {
                        gtk::Align::Center
                    });
                } else {
                    o.set_hexpand(grows);
                    o.set_halign(gtk::Align::Fill);
                }
            } else {
                o.set_valign(gtk::Align::Fill);
                o.set_hexpand(false);
                let shrink = match w.component {
                    Component::Badge => true,
                    Component::Image => client
                        .attr(cid, "variant")
                        .is_some_and(|v| v.as_str() != Some("full")),
                    _ => false,
                };
                o.set_halign(match align {
                    "stretch" if shrink => gtk::Align::Start,
                    _ if w.component == Component::Divider => gtk::Align::Fill,
                    a => to_align(a),
                });
            }
        }
    }

    // --- attributes ----------------------------------------------------------

    fn update(&self, client: &Client, id: &str) {
        let Some(w) = self.widgets.get(id) else {
            return;
        };
        let Some(r) = client.get(id) else { return };
        let a = &r.attrs;
        let mut visible = bl(a, "visible");
        if w.component.is_control() {
            w.outer
                .set_sensitive(!bl(a, "disabled") && client.is_connected());
        }
        match &w.kind {
            Kind::Plain | Kind::Divider(_) => {}
            Kind::Card { title } => set_opt_text(title, st(a, "title")),
            Kind::List { empty } => {
                let any = r
                    .children
                    .iter()
                    .any(|c| client.attr(c, "visible") == Some(Value::Bool(true)));
                let text = st(a, "empty").filter(|_| !any);
                set_opt_text(empty, text);
            }
            Kind::Tabs { stack } => self.update_tabs(client, &r, stack),
            Kind::Dialog {
                header,
                title,
                close,
            } => {
                visible = visible && bl(a, "open");
                set_opt_text(title, st(a, "title"));
                let dismissable = !a.get("onDismiss").is_none_or(Value::is_null);
                close.set_visible(dismissable);
                header.set_visible(dismissable || title.is_visible());
            }
            Kind::Text(l) => {
                l.set_text(st(a, "text").unwrap_or(""));
                set_variant(
                    l,
                    &["h1", "h2", "h3", "body", "caption", "code"],
                    st(a, "variant"),
                );
                l.set_accessible_role(match st(a, "variant") {
                    Some("h1" | "h2" | "h3") => gtk::AccessibleRole::Heading,
                    _ => gtk::AccessibleRole::Label,
                });
            }
            Kind::Notice { icon, title, text } => {
                let v = st(a, "variant").unwrap_or("info");
                set_variant(
                    &w.outer,
                    &["info", "success", "warning", "failure"],
                    Some(v),
                );
                icon.set_icon_name(Some(match v {
                    "success" => "object-select-symbolic",
                    "warning" => "dialog-warning-symbolic",
                    "failure" => "dialog-error-symbolic",
                    _ => "dialog-information-symbolic",
                }));
                set_opt_text(title, st(a, "title"));
                set_opt_text(text, st(a, "text"));
            }
            Kind::Badge(l) => {
                l.set_text(st(a, "text").unwrap_or(""));
                set_variant(
                    l,
                    &["neutral", "info", "success", "warning", "failure"],
                    st(a, "variant"),
                );
            }
            Kind::Image { pic, alt, src } => {
                let s = st(a, "src").unwrap_or("");
                if src.borrow().as_deref() != Some(s) {
                    *src.borrow_mut() = Some(s.to_string());
                    let tex = load_image(s);
                    pic.set_paintable(tex.as_ref());
                    pic.set_visible(tex.is_some());
                }
                alt.set_text(st(a, "alt").unwrap_or(""));
                alt.set_visible(!pic.is_visible());
                pic.set_alternative_text(st(a, "alt"));
                let v = st(a, "variant").unwrap_or("inline");
                set_variant(pic, &["inline", "thumbnail", "avatar", "full"], Some(v));
                let (size, fit) = match v {
                    "thumbnail" => (64, gtk::ContentFit::Cover),
                    "avatar" => (40, gtk::ContentFit::Cover),
                    _ => (-1, gtk::ContentFit::Contain),
                };
                pic.set_size_request(size, size);
                pic.set_content_fit(fit);
                pic.set_overflow(gtk::Overflow::Hidden);
            }
            Kind::Progress {
                label,
                bar,
                spinner,
                pulse,
                pulsing,
            } => {
                set_opt_text(label, st(a, "label"));
                let spin = st(a, "variant") == Some("spinner");
                spinner.set_visible(spin);
                spinner.set_spinning(spin);
                bar.set_visible(!spin);
                let value = a.get("value").and_then(Value::as_f64);
                if !spin && value.is_none() {
                    if !pulsing.replace(true) {
                        let g = pulse.get() + 1;
                        pulse.set(g);
                        let (bar, pulse) = (bar.downgrade(), pulse.clone());
                        glib::timeout_add_local(Duration::from_millis(120), move || {
                            match bar.upgrade() {
                                Some(b) if pulse.get() == g => {
                                    b.pulse();
                                    glib::ControlFlow::Continue
                                }
                                _ => glib::ControlFlow::Break,
                            }
                        });
                    }
                } else {
                    if pulsing.replace(false) {
                        pulse.set(pulse.get() + 1);
                    }
                    bar.set_fraction(value.unwrap_or(0.0).clamp(0.0, 1.0));
                }
            }
            Kind::Button(b) => {
                b.set_label(st(a, "label").unwrap_or(""));
                for c in [
                    "suggested-action",
                    "destructive-action",
                    "flat",
                    "he-success",
                ] {
                    b.remove_css_class(c);
                }
                match st(a, "variant") {
                    Some("primary") => b.add_css_class("suggested-action"),
                    Some("failure") => b.add_css_class("destructive-action"),
                    Some("plain") => b.add_css_class("flat"),
                    Some("success") => b.add_css_class("he-success"),
                    _ => {}
                }
            }
            Kind::Input {
                label,
                entry,
                multi,
                multi_scroll,
                error,
                state,
            } => {
                set_opt_text(label, st(a, "label"));
                let variant = st(a, "variant").unwrap_or("text");
                *state.variant.borrow_mut() = variant.to_string();
                let is_multi = variant == "multiline";
                entry.set_visible(!is_multi);
                multi_scroll.set_visible(is_multi);
                entry.set_visibility(variant != "password");
                entry.set_input_purpose(match variant {
                    "password" => gtk::InputPurpose::Password,
                    "number" => gtk::InputPurpose::Number,
                    "email" => gtk::InputPurpose::Email,
                    _ => gtk::InputPurpose::FreeForm,
                });
                entry.set_placeholder_text(st(a, "placeholder"));
                let err = st(a, "error").unwrap_or("");
                set_opt_text(error, Some(err));
                for w in [entry.upcast_ref::<gtk::Widget>(), multi_scroll.upcast_ref()] {
                    if err.is_empty() {
                        w.remove_css_class("error");
                    } else {
                        w.add_css_class("error");
                    }
                }
                if let Some(path) = st(a, "bind") {
                    let v = client.value(path);
                    if is_multi {
                        let buf = multi.buffer();
                        let cur = buf.text(&buf.start_iter(), &buf.end_iter(), false);
                        let want = v.and_then(Value::as_str).unwrap_or("");
                        if cur != want {
                            buf.set_text(want);
                        }
                    } else if variant == "number" {
                        let want = v.filter(|v| v.is_number());
                        let cur = parse_number(&entry.text());
                        if cur.as_f64() != want.and_then(Value::as_f64) {
                            let text = want.and_then(Value::as_f64).map(|f| format_number(f, 0.0));
                            entry.set_text(&text.unwrap_or_default());
                        }
                    } else {
                        let want = v.and_then(Value::as_str).unwrap_or("");
                        if entry.text() != want {
                            entry.set_text(want);
                        }
                    }
                }
            }
            Kind::Checkbox {
                check,
                switch,
                switch_label,
                switch_row,
            } => {
                let sw = st(a, "variant") == Some("switch");
                check.set_visible(!sw);
                switch_row.set_visible(sw);
                let text = st(a, "label").unwrap_or("");
                check.set_label(Some(text));
                switch_label.set_text(text);
                if let Some(path) = st(a, "bind") {
                    let want = client.value(path).and_then(Value::as_bool).unwrap_or(false);
                    if check.is_active() != want {
                        check.set_active(want);
                    }
                    if switch.is_active() != want {
                        switch.set_active(want);
                    }
                }
            }
            Kind::Select {
                label,
                holder,
                built,
                control,
            } => {
                set_opt_text(label, st(a, "label"));
                let variant = st(a, "variant").unwrap_or("dropdown").to_string();
                let opts: Vec<(String, String)> = a
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(|o| {
                                Some((
                                    st(o.as_object()?, "value")?.into(),
                                    st(o.as_object()?, "label")?.into(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let key = (variant.clone(), opts.clone());
                if built.borrow().as_ref() != Some(&key) {
                    *control.borrow_mut() = self.build_select(id, holder, &variant, &opts);
                    *built.borrow_mut() = Some(key);
                }
                if let Some(path) = st(a, "bind") {
                    let want = client.value(path).and_then(Value::as_str);
                    let idx = want.and_then(|w| opts.iter().position(|o| o.0 == w));
                    match &*control.borrow() {
                        SelectControl::Dropdown(dd) => {
                            let sel = idx.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32);
                            if dd.selected() != sel {
                                dd.set_selected(sel);
                            }
                        }
                        SelectControl::Choices(cs) => {
                            for (i, c) in cs.iter().enumerate() {
                                let on = Some(i) == idx;
                                if c.property::<bool>("active") != on {
                                    c.set_property("active", on);
                                }
                            }
                        }
                        SelectControl::None => {}
                    }
                }
            }
            Kind::Slider {
                label,
                scale,
                state,
            } => {
                set_opt_text(label, st(a, "label"));
                let min = a.get("min").and_then(Value::as_f64).unwrap_or(0.0);
                let max = a
                    .get("max")
                    .and_then(Value::as_f64)
                    .unwrap_or(100.0)
                    .max(min);
                let step = a
                    .get("step")
                    .and_then(Value::as_f64)
                    .filter(|s| *s > 0.0)
                    .unwrap_or(1.0);
                state.step.set(step);
                let adj = scale.adjustment();
                if adj.lower() != min || adj.upper() != max {
                    scale.set_range(min, max);
                }
                scale.set_increments(step, step * 10.0);
                let want = st(a, "bind")
                    .and_then(|p| client.value(p))
                    .and_then(Value::as_f64)
                    .unwrap_or(min)
                    .clamp(min, max);
                if st(a, "bind").is_some() && (scale.value() - want).abs() > 1e-9 {
                    scale.set_value(want);
                }
                // Keeps the readout right when only the step changed.
                scale.emit_by_name::<()>("value-changed", &[]);
            }
        }
        w.outer.set_visible(visible);
        if w.container.is_some() {
            self.layout(client, id);
        }
    }

    fn update_tabs(&self, client: &Client, r: &Resolved, stack: &gtk::Stack) {
        let shown: Vec<&String> = r
            .children
            .iter()
            .filter(|c| client.attr(c, "visible") == Some(Value::Bool(true)))
            .collect();
        for c in &r.children {
            let Some(w) = self.widgets.get(c) else {
                continue;
            };
            if w.outer.parent().as_ref() != Some(stack.upcast_ref()) {
                continue;
            }
            w.outer.set_visible(shown.contains(&c));
            let label = client.attr(c, "label");
            stack
                .page(&w.outer)
                .set_title(label.as_ref().and_then(Value::as_str).unwrap_or(c));
        }
        let current = stack.visible_child_name().map(|s| s.to_string());
        let wanted = match st(&r.attrs, "bind") {
            Some(path) => client
                .value(path)
                .and_then(Value::as_str)
                .map(str::to_string),
            None => current.clone(),
        };
        let selected = wanted
            .filter(|w| shown.contains(&w))
            .or_else(|| shown.first().map(|s| s.to_string()));
        if let Some(sel) = selected {
            if current.as_deref() != Some(&sel) {
                stack.set_visible_child_name(&sel);
            }
        }
    }
}

// --- callbacks ---------------------------------------------------------------

/// Runs `f` with the tab, unless it is gone or applying changes.
fn with_host(h: &Weak<Tab>, f: impl FnOnce(&Rc<Tab>)) {
    if let Some(t) = h.upgrade() {
        if !t.is_quiet() {
            f(&t)
        }
    }
}

fn input_edited(h: &Weak<Tab>, id: &str, st: &Rc<InputState>, v: Value) {
    with_host(h, |t| {
        t.input(id, Some(v), None);
        st.pending.set(true);
        let (h, id, st2) = (h.clone(), id.to_string(), st.clone());
        restart(&st.idle, CHANGE_IDLE, move || {
            st2.idle.take();
            fire_change(&h, &id, &st2);
        });
    });
}

fn fire_change(h: &Weak<Tab>, id: &str, st: &InputState) {
    if let Some(s) = st.idle.take() {
        s.remove();
    }
    if st.pending.replace(false) {
        with_host(h, |t| t.input(id, None, Some("onChange")));
    }
}

fn slider_change(h: &Weak<Tab>, id: &str, st: &SliderState) {
    if let Some(s) = st.idle.take() {
        s.remove();
    }
    if st.pending.replace(false) {
        with_host(h, |t| t.input(id, None, Some("onChange")));
    }
}

/// Replaces a pending timeout. The callback must take its own id out of
/// `slot` so it is not removed twice.
fn restart(slot: &RefCell<Option<glib::SourceId>>, after: Duration, f: impl FnOnce() + 'static) {
    if let Some(s) = slot.take() {
        s.remove();
    }
    *slot.borrow_mut() = Some(glib::timeout_add_local_once(after, f));
}

// --- helpers -----------------------------------------------------------------

fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

fn label(class: &str) -> gtk::Label {
    let l = label_with(class);
    l.set_visible(true);
    l
}

fn label_with(class: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.add_css_class(class);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(pango::WrapMode::WordChar);
    l
}

fn set_opt_text(l: &gtk::Label, text: Option<&str>) {
    let text = text.unwrap_or("");
    l.set_text(text);
    l.set_visible(!text.is_empty());
}

fn set_variant(w: &impl IsA<gtk::Widget>, all: &[&str], v: Option<&str>) {
    for x in all {
        w.remove_css_class(&format!("he-v-{x}"));
    }
    if let Some(v) = v {
        w.add_css_class(&format!("he-v-{v}"));
    }
}

fn to_align(a: &str) -> gtk::Align {
    match a {
        "start" => gtk::Align::Start,
        "center" => gtk::Align::Center,
        "end" => gtk::Align::End,
        _ => gtk::Align::Fill,
    }
}

fn st<'a>(a: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

fn bl(a: &Map<String, Value>, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// Places `desired` as the children of `b`, in order, moving rather than
/// re-adding widgets that are already there so focus is kept.
fn sync_box(b: &gtk::Box, desired: &[gtk::Widget]) {
    let bw: &gtk::Widget = b.upcast_ref();
    let mut prev: Option<gtk::Widget> = None;
    for w in desired {
        if w.parent().as_ref() == Some(bw) {
            b.reorder_child_after(w, prev.as_ref());
        } else {
            detach(w);
            b.insert_child_after(w, prev.as_ref());
        }
        prev = Some(w.clone());
    }
    let mut extra = match &prev {
        Some(p) => p.next_sibling(),
        None => b.first_child(),
    };
    while let Some(x) = extra {
        extra = x.next_sibling();
        b.remove(&x);
    }
}

fn detach(w: &gtk::Widget) {
    let Some(p) = w.parent() else { return };
    if let Some(b) = p.downcast_ref::<gtk::Box>() {
        b.remove(w);
    } else if let Some(s) = p.downcast_ref::<gtk::Stack>() {
        s.remove(w);
    } else if let Some(o) = p.downcast_ref::<gtk::Overlay>() {
        o.remove_overlay(w);
    } else {
        w.unparent();
    }
}

/// The value of a number input: a number, or null if empty or invalid.
fn parse_number(text: &str) -> Value {
    let t = text.trim();
    if let Ok(i) = t.parse::<i64>() {
        return json!(i);
    }
    match t.parse::<f64>() {
        Ok(f) if f.is_finite() => json!(f),
        _ => Value::Null,
    }
}

fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

fn decimals(step: f64) -> usize {
    let s = format!("{step}");
    s.split_once('.').map_or(0, |(_, f)| f.len())
}

fn round_to(v: f64, step: f64) -> f64 {
    let p = 10f64.powi(decimals(step) as i32);
    (v * p).round() / p
}

fn format_number(v: f64, step: f64) -> String {
    if step > 0.0 {
        format!("{:.*}", decimals(step), v)
    } else {
        format!("{v}")
    }
}

/// Loads `data:` images. Other schemes are not fetched; the alt text is shown.
fn load_image(src: &str) -> Option<gdk::Texture> {
    let rest = src.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let bytes = if meta.ends_with(";base64") {
        let clean: String = data.chars().filter(|c| !c.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(clean)
            .ok()?
    } else {
        percent_encoding::percent_decode_str(data).collect()
    };
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
}
