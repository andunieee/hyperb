//! Draws a hypereffect element tree in the terminal, and turns keys and mouse
//! clicks into input for the session.
//!
//! Unlike the desktop renderer this keeps no widgets: every frame lays the
//! resolved tree out into an offscreen buffer as tall as the page, notes where
//! the controls ended up, and copies the visible part to the screen. Open
//! dialogs are drawn the same way over the page, and an open dropdown over
//! both. Between frames the page only keeps what the user is in the middle
//! of: focus, scroll positions, the text being edited, pending onChange
//! events and the selected tab of tabs without `bind`.

use crate::edit::{Edit, Editor};
use hyperb_core::hypereffect::client::{Client, ResolvedTree};
use hyperb_core::hypereffect::{Component, Map, Value, json};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{
    Block, BorderType, Clear, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
    Widget,
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

/// How long an input waits after the last keystroke before running onChange.
const CHANGE_IDLE: Duration = Duration::from_millis(700);
/// The same, for slider changes made with the keyboard.
const SLIDER_IDLE: Duration = Duration::from_millis(400);
/// Rows of a multiline input, before and after it grows.
const MULTI_MIN: usize = 3;
const MULTI_MAX: usize = 8;
const POPUP_MAX: usize = 10;

pub const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const FIELD_BG: Color = Color::Indexed(236);
const FIELD_FOCUS_BG: Color = Color::Indexed(238);
pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Input for the session: write `value` to the element's bound signal, if
/// it has one, then run the handler in attribute `event`, if any.
pub struct Action {
    pub id: String,
    pub value: Option<Value>,
    pub event: Option<&'static str>,
}

fn act(id: &str, value: Option<Value>, event: Option<&'static str>) -> Action {
    Action {
        id: id.to_string(),
        value,
        event,
    }
}

pub struct Page {
    focus: Option<String>,
    /// Scroll the focused control into view on the next frame.
    reveal: bool,
    scroll: u16,
    dialog_scroll: u16,
    inputs: HashMap<String, Editor>,
    /// Controls whose edits have not been reported with onChange yet, and
    /// when to report them.
    pending: HashMap<String, Instant>,
    /// A slider being dragged, and whether it moved.
    dragging: Option<(String, bool)>,
    popup: Option<Popup>,
    tab_sel: HashMap<String, String>,
    epoch: Instant,
    /// Whether the last frame shows something that moves.
    pub animating: bool,
    // Where things were on the last frame.
    doc: Layer,
    dialog: Option<DialogFrame>,
}

struct Popup {
    id: String,
    highlight: usize,
    offset: usize,
    /// Where the popup was drawn, on screen.
    rect: Rect,
}

struct DialogFrame {
    id: String,
    layer: Layer,
    rect: Rect,
    close: Option<Rect>,
}

/// A laid-out page or dialog: what was drawn where, in its own coordinates,
/// and where that shows on screen.
#[derive(Default)]
struct Layer {
    /// The screen position of the layer's (0, 0).
    origin: (i32, i32),
    /// The part of the screen the layer shows in.
    clip: Rect,
    hits: Vec<Hit>,
    focusables: Vec<(String, Rect)>,
    /// The anchor of the open dropdown, if it is in this layer.
    anchor: Option<Rect>,
    cursor: Option<Position>,
    /// Something on it moves, so it must be redrawn.
    animating: bool,
}

#[derive(Clone)]
struct Hit {
    id: String,
    rect: Rect,
    part: Part,
}

#[derive(Clone, PartialEq)]
enum Part {
    Control,
    Option(usize),
    TabHead(String),
    SliderBar,
}

impl Layer {
    fn to_screen(&self, r: Rect) -> Rect {
        let x = (r.x as i32 + self.origin.0).max(0) as u16;
        let y = (r.y as i32 + self.origin.1).max(0) as u16;
        Rect::new(x, y, r.width, r.height)
    }

    fn hit(&self, p: Position) -> Option<&Hit> {
        if !self.clip.contains(p) {
            return None;
        }
        let x = p.x as i32 - self.origin.0;
        let y = p.y as i32 - self.origin.1;
        if x < 0 || y < 0 {
            return None;
        }
        let p = Position::new(x as u16, y as u16);
        self.hits.iter().rev().find(|h| h.rect.contains(p))
    }

    fn focus_rect(&self, id: &str) -> Option<Rect> {
        self.focusables
            .iter()
            .find(|(f, _)| f == id)
            .map(|(_, r)| *r)
    }
}

impl Page {
    pub fn new() -> Self {
        Page {
            focus: None,
            reveal: false,
            scroll: 0,
            dialog_scroll: 0,
            inputs: HashMap::new(),
            pending: HashMap::new(),
            dragging: None,
            popup: None,
            tab_sel: HashMap::new(),
            epoch: Instant::now(),
            animating: false,
            doc: Layer::default(),
            dialog: None,
        }
    }

    /// Brings the text of inputs in line with their bound signals, after the
    /// tree changed.
    pub fn sync(&mut self, tree: &[ResolvedTree], client: &Client) {
        let mut seen = HashSet::new();
        walk(tree, &mut |n| {
            seen.insert(n.id.clone());
            if n.component != Component::Input {
                return;
            }
            let ed = self.inputs.entry(n.id.clone()).or_default();
            let Some(path) = st(&n.attrs, "bind") else {
                return;
            };
            let v = client.value(path);
            if st(&n.attrs, "variant") == Some("number") {
                let want = v.filter(|v| v.is_number()).and_then(Value::as_f64);
                if parse_number(ed.text()).as_f64() != want {
                    ed.set_text(&want.map(|f| format_number(f, 0.0)).unwrap_or_default());
                }
            } else {
                ed.set_text(v.and_then(Value::as_str).unwrap_or(""));
            }
        });
        self.inputs.retain(|id, _| seen.contains(id));
        self.pending.retain(|id, _| seen.contains(id));
        if self.popup.as_ref().is_some_and(|p| !seen.contains(&p.id)) {
            self.popup = None;
        }
    }

    // --- timers --------------------------------------------------------------

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.values().min().copied()
    }

    /// onChange for edits that have been idle long enough.
    pub fn due(&mut self, now: Instant) -> Vec<Action> {
        let ids: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter().filter_map(|id| self.flush(id)).collect()
    }

    fn flush(&mut self, id: &str) -> Option<Action> {
        self.pending
            .remove(id)
            .map(|_| act(id, None, Some("onChange")))
    }

    // --- focus ---------------------------------------------------------------

    fn layer(&self) -> &Layer {
        self.dialog.as_ref().map_or(&self.doc, |d| &d.layer)
    }

    fn set_focus(&mut self, id: Option<String>) -> Vec<Action> {
        if self.focus == id {
            return Vec::new();
        }
        let flushed = self.focus.take().and_then(|old| self.flush(&old));
        if let Some(id) = &id
            && let Some(ed) = self.inputs.get_mut(id)
        {
            ed.cursor_to_end();
        }
        self.focus = id;
        self.reveal = true;
        self.popup = None;
        flushed.into_iter().collect()
    }

    fn move_focus(&mut self, delta: isize) -> Vec<Action> {
        let ids: Vec<String> = self
            .layer()
            .focusables
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        if ids.is_empty() {
            self.scroll_by(delta as i32);
            return Vec::new();
        }
        let n = ids.len() as isize;
        let at = self
            .focus
            .as_ref()
            .and_then(|f| ids.iter().position(|i| i == f));
        let next = match at {
            Some(i) => (i as isize + delta).rem_euclid(n),
            None if delta > 0 => 0,
            None => n - 1,
        };
        self.set_focus(Some(ids[next as usize].clone()))
    }

    fn scroll_by(&mut self, d: i32) {
        let s = match &self.dialog {
            Some(_) => &mut self.dialog_scroll,
            None => &mut self.scroll,
        };
        *s = (*s as i32 + d).clamp(0, u16::MAX as i32) as u16;
    }

    // --- keys ----------------------------------------------------------------

    pub fn key(&mut self, k: KeyEvent, tree: &[ResolvedTree], client: &Client) -> Vec<Action> {
        if self.popup.is_some() {
            return self.popup_key(k, tree, client);
        }
        let focused = self
            .focus
            .as_deref()
            .filter(|id| self.layer().focus_rect(id).is_some())
            .and_then(|id| find(tree, id));
        if let Some(n) = focused
            && let Some(acts) = self.control_key(n, k, client)
        {
            return acts;
        }
        let page = self.layer().clip.height.max(2) as i32 - 1;
        match k.code {
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(-1),
            KeyCode::PageDown => {
                self.scroll_by(page);
                Vec::new()
            }
            KeyCode::PageUp => {
                self.scroll_by(-page);
                Vec::new()
            }
            KeyCode::Home => {
                self.scroll_by(-(u16::MAX as i32));
                Vec::new()
            }
            KeyCode::End => {
                self.scroll_by(u16::MAX as i32);
                Vec::new()
            }
            KeyCode::Esc => match &self.dialog {
                Some(d) => vec![act(&d.id, None, Some("onDismiss"))],
                None => self.set_focus(None),
            },
            _ => Vec::new(),
        }
    }

    /// Keys for the focused control; `None` lets the page have them.
    fn control_key(
        &mut self,
        n: &ResolvedTree,
        k: KeyEvent,
        client: &Client,
    ) -> Option<Vec<Action>> {
        let id = n.id.as_str();
        let a = &n.attrs;
        let enabled = client.is_connected() && !bl(a, "disabled");
        let activate = matches!(k.code, KeyCode::Enter | KeyCode::Char(' '));
        match n.component {
            Component::Button | Component::Checkbox if activate => Some(self.activate(n, client)),
            Component::Input if enabled => {
                let multi = st(a, "variant") == Some("multiline");
                if k.code == KeyCode::Enter && !multi {
                    let mut acts: Vec<Action> = self.flush(id).into_iter().collect();
                    acts.push(act(id, None, Some("onSubmit")));
                    return Some(acts);
                }
                let ed = self.inputs.entry(id.to_string()).or_default();
                match ed.key(&k, multi) {
                    Edit::Changed => Some(self.edited(n)),
                    Edit::Moved => {
                        self.reveal = true;
                        Some(Vec::new())
                    }
                    Edit::Ignored => None,
                }
            }
            Component::Select => {
                let opts = options(a);
                let cur = selected_option(n, client, &opts);
                let step = |d: isize| {
                    let len = opts.len() as isize;
                    cur.map_or(0, |c| (c as isize + d).clamp(0, len - 1) as usize)
                };
                let variant = st(a, "variant").unwrap_or("dropdown");
                let i = match (variant, k.code) {
                    (_, KeyCode::Left) | ("radio", KeyCode::Up) => step(-1),
                    (_, KeyCode::Right) | ("radio", KeyCode::Down) => step(1),
                    ("dropdown", _) if activate => return Some(self.activate(n, client)),
                    _ => return None,
                };
                Some(self.choose(n, client, i))
            }
            Component::Slider => {
                let (min, max, step) = slider_range(a);
                let v = slider_value(n, client);
                let v = match k.code {
                    KeyCode::Left => v - step,
                    KeyCode::Right => v + step,
                    KeyCode::PageDown => v - step * 10.0,
                    KeyCode::PageUp => v + step * 10.0,
                    KeyCode::Home => min,
                    KeyCode::End => max,
                    _ => return None,
                };
                Some(self.slide(n, client, v))
            }
            Component::Tabs => {
                let shown: Vec<&ResolvedTree> = shown(&n.children).collect();
                let cur = self
                    .selected_tab(n, client)
                    .and_then(|t| shown.iter().position(|s| s.id == t.id));
                let i = match (k.code, cur) {
                    (KeyCode::Left, Some(c)) => c.saturating_sub(1),
                    (KeyCode::Right, Some(c)) => (c + 1).min(shown.len().saturating_sub(1)),
                    _ => return None,
                };
                let tab = shown.get(i)?.id.clone();
                Some(self.select_tab(n, &tab))
            }
            _ => None,
        }
    }

    fn popup_key(&mut self, k: KeyEvent, tree: &[ResolvedTree], client: &Client) -> Vec<Action> {
        let Some(p) = &mut self.popup else {
            return Vec::new();
        };
        let Some(n) = find(tree, &p.id) else {
            self.popup = None;
            return Vec::new();
        };
        let len = options(&n.attrs).len();
        match k.code {
            KeyCode::Up => p.highlight = p.highlight.saturating_sub(1),
            KeyCode::Down => p.highlight = (p.highlight + 1).min(len.saturating_sub(1)),
            KeyCode::PageUp | KeyCode::Home => p.highlight = 0,
            KeyCode::PageDown | KeyCode::End => p.highlight = len.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let i = p.highlight;
                self.popup = None;
                return self.choose(n, client, i);
            }
            _ => self.popup = None,
        }
        Vec::new()
    }

    pub fn paste(&mut self, text: &str, tree: &[ResolvedTree], client: &Client) -> Vec<Action> {
        let Some(n) = self.focus.as_deref().and_then(|id| find(tree, id)) else {
            return Vec::new();
        };
        if n.component != Component::Input || !client.is_connected() || bl(&n.attrs, "disabled") {
            return Vec::new();
        }
        let text = if st(&n.attrs, "variant") == Some("multiline") {
            text.replace("\r\n", "\n")
        } else {
            text.replace(['\r', '\n'], " ")
        };
        self.inputs.entry(n.id.clone()).or_default().insert(&text);
        self.edited(n)
    }

    /// The input's text changed: write it to the bound signal and report it
    /// with onChange once the user pauses.
    fn edited(&mut self, n: &ResolvedTree) -> Vec<Action> {
        let text = self.inputs.get(&n.id).map(Editor::text).unwrap_or("");
        let v = if st(&n.attrs, "variant") == Some("number") {
            parse_number(text)
        } else {
            Value::String(text.to_string())
        };
        self.pending
            .insert(n.id.clone(), Instant::now() + CHANGE_IDLE);
        self.reveal = true;
        vec![act(&n.id, Some(v), None)]
    }

    /// Enter, Space or a click on a control.
    fn activate(&mut self, n: &ResolvedTree, client: &Client) -> Vec<Action> {
        if !client.is_connected() || bl(&n.attrs, "disabled") {
            return Vec::new();
        }
        match n.component {
            Component::Button => vec![act(&n.id, None, Some("onPress"))],
            Component::Checkbox => {
                let on = st(&n.attrs, "bind")
                    .and_then(|p| client.value(p))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                vec![act(&n.id, Some(json!(!on)), Some("onChange"))]
            }
            Component::Select if st(&n.attrs, "variant").unwrap_or("dropdown") == "dropdown" => {
                let opts = options(&n.attrs);
                if !opts.is_empty() {
                    self.popup = Some(Popup {
                        id: n.id.clone(),
                        highlight: selected_option(n, client, &opts).unwrap_or(0),
                        offset: 0,
                        rect: Rect::default(),
                    });
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn choose(&mut self, n: &ResolvedTree, client: &Client, i: usize) -> Vec<Action> {
        let opts = options(&n.attrs);
        if !client.is_connected()
            || bl(&n.attrs, "disabled")
            || selected_option(n, client, &opts) == Some(i)
        {
            return Vec::new();
        }
        match opts.get(i) {
            Some((v, _)) => vec![act(&n.id, Some(json!(v)), Some("onChange"))],
            None => Vec::new(),
        }
    }

    fn slide(&mut self, n: &ResolvedTree, client: &Client, v: f64) -> Vec<Action> {
        if !client.is_connected() || bl(&n.attrs, "disabled") {
            return Vec::new();
        }
        let (min, max, step) = slider_range(&n.attrs);
        let v = round_to(
            min + ((v.clamp(min, max) - min) / step).round() * step,
            step,
        )
        .clamp(min, max);
        if (v - slider_value(n, client)).abs() < 1e-9 {
            return Vec::new();
        }
        match &mut self.dragging {
            Some((id, moved)) if *id == n.id => *moved = true,
            _ => {
                self.pending
                    .insert(n.id.clone(), Instant::now() + SLIDER_IDLE);
            }
        }
        vec![act(&n.id, Some(number(v)), None)]
    }

    fn select_tab(&mut self, n: &ResolvedTree, tab: &str) -> Vec<Action> {
        self.tab_sel.insert(n.id.clone(), tab.to_string());
        vec![act(&n.id, Some(json!(tab)), Some("onChange"))]
    }

    fn selected_tab<'a>(&self, n: &'a ResolvedTree, client: &Client) -> Option<&'a ResolvedTree> {
        let wanted = match st(&n.attrs, "bind") {
            Some(path) => client.value(path).and_then(Value::as_str),
            None => self.tab_sel.get(&n.id).map(String::as_str),
        };
        shown(&n.children)
            .find(|t| Some(t.id.as_str()) == wanted)
            .or_else(|| shown(&n.children).next())
    }

    // --- mouse ---------------------------------------------------------------

    pub fn mouse(&mut self, m: MouseEvent, tree: &[ResolvedTree], client: &Client) -> Vec<Action> {
        let p = Position::new(m.column, m.row);
        match m.kind {
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::Down(MouseButton::Left) => return self.click(p, tree, client),
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(n) = self.dragging.as_ref().and_then(|(id, _)| find(tree, id)) {
                    return self.slide_to(n, client, p.x);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some((id, true)) = self.dragging.take() {
                    return vec![act(&id, None, Some("onChange"))];
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn click(&mut self, p: Position, tree: &[ResolvedTree], client: &Client) -> Vec<Action> {
        if let Some(pop) = self.popup.take() {
            if pop.rect.contains(p) {
                let row = p.y.saturating_sub(pop.rect.y + 1) as usize + pop.offset;
                if let Some(n) = find(tree, &pop.id) {
                    return self.choose(n, client, row);
                }
            }
            return Vec::new();
        }
        if let Some(d) = &self.dialog {
            if d.close.is_some_and(|c| c.contains(p)) {
                return vec![act(&d.id, None, Some("onDismiss"))];
            }
            if !d.rect.contains(p) {
                return Vec::new();
            }
        }
        let Some(hit) = self.layer().hit(p).cloned() else {
            return self.set_focus(None);
        };
        let mut acts = self.set_focus(Some(hit.id.clone()));
        let Some(n) = find(tree, &hit.id) else {
            return acts;
        };
        acts.extend(match hit.part {
            Part::Control => self.activate(n, client),
            Part::Option(i) => self.choose(n, client, i),
            Part::TabHead(t) => self.select_tab(n, &t),
            Part::SliderBar => {
                self.dragging = Some((n.id.clone(), false));
                let mut a = self.slide_to(n, client, p.x);
                if !a.is_empty() {
                    a.push(act(&n.id, None, Some("onChange")));
                }
                a
            }
        });
        acts
    }

    fn slide_to(&mut self, n: &ResolvedTree, client: &Client, x: u16) -> Vec<Action> {
        let layer = self.layer();
        let Some(bar) = layer
            .hits
            .iter()
            .find(|h| h.id == n.id && h.part == Part::SliderBar)
        else {
            return Vec::new();
        };
        let bar = layer.to_screen(bar.rect);
        let (min, max, _) = slider_range(&n.attrs);
        let f = (x.saturating_sub(bar.x) as f64 / bar.width.saturating_sub(1).max(1) as f64)
            .clamp(0.0, 1.0);
        self.slide(n, client, min + f * (max - min))
    }

    // --- drawing -------------------------------------------------------------

    /// Draws the page into `area` and returns where the cursor goes, if a
    /// text field has focus.
    pub fn draw(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        tree: &[ResolvedTree],
        client: &Client,
    ) -> Option<Position> {
        let tick = self.epoch.elapsed().as_millis() as u64;
        let enabled = client.is_connected();
        let (dialogs, body): (Vec<&ResolvedTree>, Vec<&ResolvedTree>) =
            shown(tree).partition(|n| n.component == Component::Dialog);
        let dialogs: Vec<&ResolvedTree> = dialogs
            .into_iter()
            .filter(|d| bl(&d.attrs, "open"))
            .collect();
        self.animating = false;

        // The page, scrolled.
        let doc_w = area.width.saturating_sub(3).max(1);
        let (doc, mut layer) = {
            let env = Env {
                page: self,
                client,
                enabled,
                tick,
            };
            let h = column_height(&env, &body, doc_w, 1, "stretch");
            let mut doc = Buffer::empty(Rect::new(0, 0, doc_w, h + 2));
            let mut out = Layer::default();
            draw_column(
                &env,
                &mut out,
                &mut doc,
                &body,
                Rect::new(0, 1, doc_w, h),
                1,
                "stretch",
            );
            (doc, out)
        };
        self.animating |= layer.animating;
        if dialogs.is_empty() {
            self.scroll = self.reveal_in(&layer, self.scroll, area.height);
        }
        self.scroll = self.scroll.min(doc.area.height.saturating_sub(area.height));
        layer.origin = (area.x as i32 + 1, area.y as i32 - self.scroll as i32);
        layer.clip = Rect::new(area.x + 1, area.y, doc_w, area.height);
        blit(&doc, self.scroll, buf, layer.clip);
        if doc.area.height > area.height {
            let mut sb = ScrollbarState::new(doc.area.height.saturating_sub(area.height) as usize)
                .position(self.scroll as usize);
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_symbol("┃")
                .track_symbol(Some("│"))
                .thumb_style(Style::new().fg(MUTED))
                .track_style(Style::new().fg(Color::Indexed(236)))
                .render(area, buf, &mut sb);
        }
        self.doc = layer;

        // Dialogs, the last one on top and the only one taking input.
        self.dialog = None;
        if !dialogs.is_empty() {
            buf.set_style(area, Style::new().fg(MUTED).add_modifier(Modifier::DIM));
        }
        for (i, d) in dialogs.iter().enumerate() {
            let top = i + 1 == dialogs.len();
            let frame = self.draw_dialog(area, buf, d, client, tick, top);
            if top {
                self.dialog = Some(frame);
            }
        }
        self.reveal = false;

        let cursor = {
            let l = self.layer();
            l.cursor
                .map(|c| {
                    let s = l.to_screen(Rect::new(c.x, c.y, 1, 1));
                    Position::new(s.x, s.y)
                })
                .filter(|c| l.clip.contains(*c))
        };
        self.draw_popup(area, buf, tree, client);
        if self.popup.is_some() {
            return None;
        }
        cursor
    }

    /// The scroll position that shows the focused control, if it should be revealed.
    fn reveal_in(&self, layer: &Layer, scroll: u16, view: u16) -> u16 {
        let Some(r) = self
            .focus
            .as_deref()
            .and_then(|f| layer.focus_rect(f))
            .filter(|_| self.reveal)
        else {
            return scroll;
        };
        // Include the line above, which is usually the control's label or a gap.
        let top = r.y.saturating_sub(1);
        if top < scroll {
            top
        } else if r.bottom() + 1 > scroll + view {
            (r.bottom() + 1).saturating_sub(view).min(top)
        } else {
            scroll
        }
    }

    fn draw_dialog(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        d: &ResolvedTree,
        client: &Client,
        tick: u64,
        top: bool,
    ) -> DialogFrame {
        let kids: Vec<&ResolvedTree> = shown(&d.children).collect();
        let max_w = area.width.saturating_sub(4).max(10);
        let (content, mut layer) = {
            let env = Env {
                page: self,
                client,
                enabled: client.is_connected(),
                tick,
            };
            let nat = kids
                .iter()
                .map(|k| natural(&env, k, max_w))
                .max()
                .unwrap_or(0);
            let title = st(&d.attrs, "title").map_or(0, |t| t.width() as u16 + 8);
            let w = (nat.max(title) + 4).clamp(44.min(max_w), max_w);
            let inner_w = w.saturating_sub(4).max(1);
            let h = column_height(&env, &kids, inner_w, 1, "stretch");
            let mut content = Buffer::empty(Rect::new(0, 0, inner_w, h + 2));
            let mut out = Layer::default();
            draw_column(
                &env,
                &mut out,
                &mut content,
                &kids,
                Rect::new(0, 1, inner_w, h),
                1,
                "stretch",
            );
            (content, out)
        };
        self.animating |= layer.animating;
        let w = content.area.width + 4;
        let h = (content.area.height + 2).min(area.height.saturating_sub(2).max(3));
        let rect = Rect::new(
            area.x + area.width.saturating_sub(w) / 2,
            area.y + area.height.saturating_sub(h) / 2,
            w.min(area.width),
            h,
        );
        let view = h.saturating_sub(2);
        if top {
            self.dialog_scroll = self.reveal_in(&layer, self.dialog_scroll, view);
            self.dialog_scroll = self
                .dialog_scroll
                .min(content.area.height.saturating_sub(view));
        }
        let scroll = if top { self.dialog_scroll } else { 0 };
        Clear.render(rect, buf);
        let mut block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(if top { ACCENT } else { MUTED }));
        if let Some(t) = st(&d.attrs, "title") {
            block = block.title_top(Line::styled(
                format!(" {t} "),
                Style::new().add_modifier(Modifier::BOLD),
            ));
        }
        block.render(rect, buf);
        layer.origin = (rect.x as i32 + 2, rect.y as i32 + 1 - scroll as i32);
        layer.clip = Rect::new(rect.x + 2, rect.y + 1, content.area.width, view);
        blit(&content, scroll, buf, layer.clip);
        let dismissable = d.attrs.get("onDismiss").is_some_and(|h| !h.is_null());
        let close = dismissable.then(|| {
            let c = Rect::new(rect.right().saturating_sub(5), rect.y, 3, 1);
            buf.set_stringn(c.x, c.y, " ✕ ", 3, Style::new().fg(ACCENT));
            c
        });
        DialogFrame {
            id: d.id.clone(),
            layer,
            rect,
            close,
        }
    }

    fn draw_popup(&mut self, area: Rect, buf: &mut Buffer, tree: &[ResolvedTree], client: &Client) {
        let Some(anchor) = self.layer().anchor.map(|a| self.layer().to_screen(a)) else {
            self.popup = None;
            return;
        };
        let Some(p) = &mut self.popup else { return };
        let Some(n) = find(tree, &p.id) else { return };
        let opts = options(&n.attrs);
        let current = selected_option(n, client, &opts);
        let rows = opts.len().min(POPUP_MAX);
        let widest = opts.iter().map(|o| o.1.width()).max().unwrap_or(0) as u16;
        let w = (widest + 6).max(anchor.width).min(area.width);
        let h = rows as u16 + 2;
        let below = area.bottom().saturating_sub(anchor.bottom());
        let y = if below >= h || anchor.y.saturating_sub(area.y) < h {
            anchor.bottom()
        } else {
            anchor.y - h
        };
        let x = anchor.x.min(area.right().saturating_sub(w));
        let rect = Rect::new(x, y, w, h).intersection(area);
        if p.highlight < p.offset {
            p.offset = p.highlight;
        } else if p.highlight >= p.offset + rows {
            p.offset = p.highlight + 1 - rows;
        }
        p.rect = rect;
        Clear.render(rect, buf);
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(ACCENT))
            .render(rect, buf);
        for (row, (i, (_, label))) in opts
            .iter()
            .enumerate()
            .skip(p.offset)
            .take(rows)
            .enumerate()
        {
            let y = rect.y + 1 + row as u16;
            if y + 1 >= rect.bottom() {
                break;
            }
            let mut style = Style::new();
            if i == p.highlight {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let mark = if Some(i) == current { "• " } else { "  " };
            let line = format!(
                " {mark}{label:<width$} ",
                width = (w as usize).saturating_sub(6)
            );
            buf.set_stringn(rect.x + 1, y, line, w.saturating_sub(2) as usize, style);
        }
    }
}

/// What layout and drawing read.
struct Env<'a> {
    page: &'a Page,
    client: &'a Client,
    enabled: bool,
    tick: u64,
}

impl Env<'_> {
    fn focused(&self, id: &str) -> bool {
        self.page.focus.as_deref() == Some(id)
    }

    fn enabled(&self, n: &ResolvedTree) -> bool {
        self.enabled && !bl(&n.attrs, "disabled")
    }

    fn editor(&self, id: &str) -> Editor {
        self.page.inputs.get(id).cloned().unwrap_or_default()
    }

    fn spinner(&self) -> &'static str {
        SPINNER[(self.tick / 80) as usize % SPINNER.len()]
    }
}

// --- layout ------------------------------------------------------------------

fn height(env: &Env, n: &ResolvedTree, w: u16) -> u16 {
    let a = &n.attrs;
    let w = w.max(1);
    let lines = |s: &str, w: u16| wrap(s, w).len() as u16;
    match n.component {
        Component::Column | Component::Tab => {
            let kids: Vec<&ResolvedTree> = shown(&n.children).collect();
            column_height(
                env,
                &kids,
                w,
                col_gap(a),
                st(a, "align").unwrap_or("stretch"),
            )
        }
        Component::Row => row_layout(env, n, w).iter().map(|l| l.height).sum(),
        Component::Card => {
            let kids: Vec<&ResolvedTree> = shown(&n.children).collect();
            2 + column_height(env, &kids, w.saturating_sub(4), col_gap(a), "stretch")
        }
        Component::List => {
            let kids: Vec<&ResolvedTree> = shown(&n.children).collect();
            if kids.is_empty() {
                2 + st(a, "empty").map_or(0, |e| lines(e, w.saturating_sub(4)))
            } else {
                let inner = w.saturating_sub(4);
                2 + kids.iter().map(|k| height(env, k, inner)).sum::<u16>() + kids.len() as u16 - 1
            }
        }
        Component::Tabs => {
            let content = env.page.selected_tab(n, env.client).map_or(0, |t| {
                let kids: Vec<&ResolvedTree> = shown(&t.children).collect();
                column_height(env, &kids, w, col_gap(&t.attrs), "stretch")
            });
            3 + content
        }
        Component::Dialog => 0,
        Component::Text => lines(st(a, "text").unwrap_or(""), w),
        Component::Notice => {
            let inner = w.saturating_sub(4);
            let title = st(a, "title").map_or(0, |t| lines(t, inner));
            let text = st(a, "text")
                .filter(|t| !t.is_empty())
                .map_or(0, |t| lines(t, inner));
            (title + text).max(1)
        }
        Component::Badge | Component::Button | Component::Divider => 1,
        Component::Image => lines(&image_text(a), w),
        Component::Progress => {
            if st(a, "variant") == Some("spinner") {
                1
            } else {
                1 + st(a, "label").is_some() as u16
            }
        }
        Component::Input => {
            let label = st(a, "label").is_some() as u16;
            let field = if st(a, "variant") == Some("multiline") {
                let (l, _) = env.editor(&n.id).wrapped(w.saturating_sub(2));
                l.len().clamp(MULTI_MIN, MULTI_MAX) as u16 + 2
            } else {
                1
            };
            let error = st(a, "error")
                .filter(|e| !e.is_empty())
                .map_or(0, |e| lines(e, w));
            label + field + error
        }
        Component::Checkbox => lines(st(a, "label").unwrap_or(""), w.saturating_sub(4)).max(1),
        Component::Select => {
            let label = st(a, "label").is_some() as u16;
            let body = match st(a, "variant") {
                Some("radio") => options(a).len().max(1) as u16,
                _ => 1,
            };
            label + body
        }
        Component::Slider => 2,
    }
}

/// The width an element would like, up to `max`.
fn natural(env: &Env, n: &ResolvedTree, max: u16) -> u16 {
    let a = &n.attrs;
    let tw = |k: &str| {
        st(a, k).map_or(0, |s| {
            s.lines().map(UnicodeWidthStr::width).max().unwrap_or(0)
        }) as u16
    };
    let kids_max =
        |kids: &[ResolvedTree], m: u16| shown(kids).map(|k| natural(env, k, m)).max().unwrap_or(0);
    let w = match n.component {
        Component::Column | Component::Tab => kids_max(&n.children, max),
        Component::Row => {
            let kids: Vec<u16> = shown(&n.children).map(|k| natural(env, k, max)).collect();
            kids.iter().sum::<u16>() + row_gap(a) * kids.len().saturating_sub(1) as u16
        }
        Component::Card => kids_max(&n.children, max.saturating_sub(4)).max(tw("title") + 4) + 4,
        Component::List => kids_max(&n.children, max.saturating_sub(4)).max(tw("empty")) + 4,
        Component::Tabs => {
            let head: u16 = shown(&n.children)
                .map(|t| tab_label(t).width() as u16 + 3)
                .sum();
            let body = env
                .page
                .selected_tab(n, env.client)
                .map_or(0, |t| kids_max(&t.children, max));
            head.max(body)
        }
        Component::Dialog => 0,
        Component::Text => tw("text"),
        Component::Notice => tw("title").max(tw("text")) + 4,
        Component::Badge => tw("text") + 2,
        Component::Image => image_text(a).width() as u16,
        Component::Divider => 1,
        Component::Progress => tw("label").max(20),
        Component::Button => tw("label") + 4,
        Component::Input => tw("label").max(20),
        Component::Checkbox => tw("label") + 4,
        Component::Select => {
            let opts = options(a);
            let widest = opts.iter().map(|o| o.1.width() as u16).max().unwrap_or(0);
            let body = match st(a, "variant") {
                Some("segmented") => opts.iter().map(|o| o.1.width() as u16 + 3).sum::<u16>(),
                _ => widest + 4,
            };
            body.max(tw("label"))
        }
        Component::Slider => (tw("label") + 8).max(20),
    };
    w.min(max)
}

/// Elements that take the room left over in a row.
fn grows(n: &ResolvedTree) -> bool {
    match n.component {
        Component::Input
        | Component::Slider
        | Component::Progress
        | Component::Column
        | Component::Card
        | Component::List
        | Component::Tabs
        | Component::Notice => true,
        Component::Row => shown(&n.children).any(grows),
        _ => false,
    }
}

/// Elements that keep their natural width in a stretched column.
fn shrinks(n: &ResolvedTree) -> bool {
    match n.component {
        Component::Badge | Component::Button | Component::Checkbox => true,
        Component::Image => st(&n.attrs, "variant") != Some("full"),
        _ => false,
    }
}

fn col_gap(a: &Map<String, Value>) -> u16 {
    match st(a, "gap").unwrap_or("medium") {
        "none" | "small" => 0,
        "large" => 2,
        _ => 1,
    }
}

fn row_gap(a: &Map<String, Value>) -> u16 {
    match st(a, "gap").unwrap_or("small") {
        "none" => 0,
        "medium" => 2,
        "large" => 4,
        _ => 1,
    }
}

fn child_width(env: &Env, k: &ResolvedTree, w: u16, align: &str) -> u16 {
    if align == "stretch" && !shrinks(k) {
        w
    } else {
        natural(env, k, w).clamp(1, w.max(1))
    }
}

fn column_height(env: &Env, kids: &[&ResolvedTree], w: u16, gap: u16, align: &str) -> u16 {
    let n = kids.len() as u16;
    kids.iter()
        .map(|k| height(env, k, child_width(env, k, w, align)))
        .sum::<u16>()
        + gap * n.saturating_sub(1)
}

struct RowLine<'a> {
    /// Element, x, width, height.
    items: Vec<(&'a ResolvedTree, u16, u16, u16)>,
    height: u16,
}

/// Places a row's children: on one line, or over several if `wrap` is set.
/// Text shrinks to make room; if that is not enough, the children are
/// stacked as in a column.
fn row_layout<'a>(env: &Env, n: &'a ResolvedTree, w: u16) -> Vec<RowLine<'a>> {
    let a = &n.attrs;
    let kids: Vec<&ResolvedTree> = shown(&n.children).collect();
    let gap = row_gap(a);
    let justify = st(a, "justify").unwrap_or("start");
    let nat: Vec<u16> = kids
        .iter()
        .map(|k| {
            let min = if grows(k) { 12 } else { 1 };
            natural(env, k, w).max(min.min(w))
        })
        .collect();

    let mut groups: Vec<Vec<usize>> = vec![Vec::new()];
    let mut used = 0;
    for (i, &nw) in nat.iter().enumerate() {
        let g = groups.last_mut().unwrap();
        if bl(a, "wrap") && !g.is_empty() && used + gap + nw > w {
            groups.push(vec![i]);
            used = nw;
        } else {
            used += if g.is_empty() { 0 } else { gap } + nw;
            g.push(i);
        }
    }

    let mut lines = Vec::new();
    for g in groups.into_iter().filter(|g| !g.is_empty()) {
        let gaps = gap * (g.len() as u16 - 1);
        let mut widths: Vec<u16> = g.iter().map(|&i| nat[i]).collect();
        if widths.iter().sum::<u16>() + gaps > w {
            let texts: Vec<usize> = (0..g.len())
                .filter(|&j| kids[g[j]].component == Component::Text)
                .collect();
            let fixed: u16 = (0..g.len())
                .filter(|j| !texts.contains(j))
                .map(|j| widths[j])
                .sum::<u16>()
                + gaps;
            let avail = w.saturating_sub(fixed);
            if texts.is_empty() || avail < 8 * texts.len() as u16 {
                for &i in &g {
                    let k = kids[i];
                    let cw = if grows(k) || k.component == Component::Text {
                        w
                    } else {
                        nat[i].min(w)
                    };
                    let h = height(env, k, cw);
                    lines.push(RowLine {
                        items: vec![(k, 0, cw, h)],
                        height: h,
                    });
                }
                continue;
            }
            // Share what is left between the texts, narrowest first.
            let mut order = texts.clone();
            order.sort_by_key(|&j| widths[j]);
            let mut rem = avail;
            for (c, &j) in order.iter().enumerate() {
                let share = rem / (order.len() - c) as u16;
                widths[j] = widths[j].min(share);
                rem -= widths[j];
            }
        }
        let extra = w.saturating_sub(widths.iter().sum::<u16>() + gaps);
        let growers: Vec<usize> = (0..g.len()).filter(|&j| grows(kids[g[j]])).collect();
        let (mut x, mut spread) = (0, 0);
        if !growers.is_empty() {
            let each = extra / growers.len() as u16;
            for (c, &j) in growers.iter().enumerate() {
                widths[j] += each + ((c as u16) < extra % growers.len() as u16) as u16;
            }
        } else {
            match justify {
                "center" => x = extra / 2,
                "end" => x = extra,
                "between" if g.len() > 1 => spread = extra,
                _ => {}
            }
        }
        let mut items = Vec::new();
        for (j, &i) in g.iter().enumerate() {
            if j > 0 {
                let slots = g.len() as u16 - 1;
                x += gap + spread / slots + ((j as u16) <= spread % slots) as u16;
            }
            let h = height(env, kids[i], widths[j]);
            items.push((kids[i], x, widths[j], h));
            x += widths[j];
        }
        let height = items.iter().map(|it| it.3).max().unwrap_or(0);
        lines.push(RowLine { items, height });
    }
    lines
}

// --- drawing -----------------------------------------------------------------

fn draw_column(
    env: &Env,
    out: &mut Layer,
    buf: &mut Buffer,
    kids: &[&ResolvedTree],
    r: Rect,
    gap: u16,
    align: &str,
) {
    let mut y = r.y;
    for k in kids {
        let cw = child_width(env, k, r.width, align);
        let x = match align {
            "center" => (r.width - cw) / 2,
            "end" => r.width - cw,
            _ => 0,
        };
        let h = height(env, k, cw);
        draw(env, out, buf, k, Rect::new(r.x + x, y, cw, h));
        y += h + gap;
    }
}

fn draw(env: &Env, out: &mut Layer, buf: &mut Buffer, n: &ResolvedTree, r: Rect) {
    let a = &n.attrs;
    let id = n.id.as_str();
    let focused = env.focused(id);
    let enabled = env.enabled(n);
    if r.width == 0 || r.height == 0 {
        return;
    }
    match n.component {
        Component::Column | Component::Tab => {
            let kids: Vec<&ResolvedTree> = shown(&n.children).collect();
            draw_column(
                env,
                out,
                buf,
                &kids,
                r,
                col_gap(a),
                st(a, "align").unwrap_or("stretch"),
            );
        }
        Component::Row => {
            let align = st(a, "align").unwrap_or("center");
            let mut y = r.y;
            for line in row_layout(env, n, r.width) {
                for (k, x, w, h) in line.items {
                    let (dy, h) = match (k.component, align) {
                        (Component::Divider, _) => (0, line.height),
                        (_, "center") => ((line.height - h) / 2, h),
                        (_, "end") => (line.height - h, h),
                        _ => (0, h),
                    };
                    draw(env, out, buf, k, Rect::new(r.x + x, y + dy, w, h));
                }
                y += line.height;
            }
        }
        Component::Card => {
            let mut block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(MUTED));
            if let Some(t) = st(a, "title") {
                block = block.title_top(Line::styled(
                    format!(" {t} "),
                    Style::new().add_modifier(Modifier::BOLD),
                ));
            }
            block.render(r, buf);
            let kids: Vec<&ResolvedTree> = shown(&n.children).collect();
            let inner = Rect::new(
                r.x + 2,
                r.y + 1,
                r.width.saturating_sub(4),
                r.height.saturating_sub(2),
            );
            draw_column(env, out, buf, &kids, inner, col_gap(a), "stretch");
        }
        Component::List => {
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(MUTED))
                .render(r, buf);
            let kids: Vec<&ResolvedTree> = shown(&n.children).collect();
            let inner_w = r.width.saturating_sub(4);
            if kids.is_empty() {
                let style = Style::new().fg(MUTED).add_modifier(Modifier::ITALIC);
                put_lines(
                    buf,
                    r.x + 2,
                    r.y + 1,
                    &wrap(st(a, "empty").unwrap_or(""), inner_w),
                    inner_w,
                    style,
                );
            }
            let mut y = r.y + 1;
            for (i, k) in kids.iter().enumerate() {
                if i > 0 {
                    let sep = format!("├{}┤", "─".repeat(r.width.saturating_sub(2) as usize));
                    put(buf, r.x, y, &sep, r.width, Style::new().fg(MUTED));
                    y += 1;
                }
                let h = height(env, k, inner_w);
                draw(env, out, buf, k, Rect::new(r.x + 2, y, inner_w, h));
                y += h;
            }
        }
        Component::Tabs => {
            let sel = env.page.selected_tab(n, env.client);
            let mut x = r.x;
            for t in shown(&n.children) {
                let text = format!(" {} ", tab_label(t));
                let w = text.width() as u16;
                let on = sel.is_some_and(|s| s.id == t.id);
                let mut style = Style::new();
                if on {
                    style = style
                        .fg(ACCENT)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                    if focused {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                }
                put(buf, x, r.y, &text, r.right().saturating_sub(x), style);
                out.hits.push(Hit {
                    id: id.to_string(),
                    rect: Rect::new(x, r.y, w, 1),
                    part: Part::TabHead(t.id.clone()),
                });
                x = x.saturating_add(w + 1);
            }
            out.focusables
                .push((id.to_string(), Rect::new(r.x, r.y, r.width, 1)));
            put(
                buf,
                r.x,
                r.y + 1,
                &"─".repeat(r.width as usize),
                r.width,
                Style::new().fg(MUTED),
            );
            if let Some(t) = sel {
                let kids: Vec<&ResolvedTree> = shown(&t.children).collect();
                let inner = Rect::new(r.x, r.y + 3, r.width, r.height.saturating_sub(3));
                draw_column(env, out, buf, &kids, inner, col_gap(&t.attrs), "stretch");
            }
        }
        Component::Dialog => {}
        Component::Text => {
            let style = match st(a, "variant") {
                Some("h1") => Style::new()
                    .fg(ACCENT)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                Some("h2") => Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                Some("h3") => Style::new().add_modifier(Modifier::BOLD),
                Some("caption") => Style::new().fg(Color::Gray).add_modifier(Modifier::DIM),
                Some("code") => Style::new().fg(Color::Yellow),
                _ => Style::new(),
            };
            put_lines(
                buf,
                r.x,
                r.y,
                &wrap(st(a, "text").unwrap_or(""), r.width),
                r.width,
                style,
            );
        }
        Component::Notice => {
            let v = st(a, "variant").unwrap_or("info");
            let color = variant_color(v);
            for y in r.y..r.bottom() {
                put(buf, r.x, y, "▌", 1, Style::new().fg(color));
            }
            let icon = match v {
                "success" => "✓",
                "warning" => "!",
                "failure" => "✗",
                _ => "i",
            };
            put(
                buf,
                r.x + 2,
                r.y,
                icon,
                1,
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            );
            let inner = r.width.saturating_sub(4);
            let mut y = r.y;
            if let Some(t) = st(a, "title") {
                let l = wrap(t, inner);
                put_lines(
                    buf,
                    r.x + 4,
                    y,
                    &l,
                    inner,
                    Style::new().add_modifier(Modifier::BOLD),
                );
                y += l.len() as u16;
            }
            put_lines(
                buf,
                r.x + 4,
                y,
                &wrap(st(a, "text").unwrap_or(""), inner),
                inner,
                Style::new(),
            );
        }
        Component::Badge => {
            let v = st(a, "variant").unwrap_or("neutral");
            let fg = if matches!(v, "info" | "failure") {
                Color::White
            } else {
                Color::Black
            };
            let text = format!(" {} ", st(a, "text").unwrap_or(""));
            put(
                buf,
                r.x,
                r.y,
                &text,
                r.width,
                Style::new().fg(fg).bg(variant_color(v)),
            );
        }
        Component::Image => {
            let style = Style::new().fg(MUTED).add_modifier(Modifier::ITALIC);
            put_lines(
                buf,
                r.x,
                r.y,
                &wrap(&image_text(a), r.width),
                r.width,
                style,
            );
        }
        Component::Divider => {
            let style = Style::new().fg(MUTED);
            if r.width == 1 && r.height >= 1 {
                for y in r.y..r.bottom() {
                    put(buf, r.x, y, "│", 1, style);
                }
            } else {
                put(buf, r.x, r.y, &"─".repeat(r.width as usize), r.width, style);
            }
        }
        Component::Progress => draw_progress(env, out, buf, n, r),
        Component::Button => {
            let label = st(a, "label").unwrap_or("");
            let v = st(a, "variant").unwrap_or("secondary");
            let (text, mut style) = match v {
                "primary" => (
                    format!("  {label}  "),
                    Style::new()
                        .fg(Color::White)
                        .bg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                ),
                "success" => (
                    format!("  {label}  "),
                    Style::new().fg(Color::Black).bg(Color::Green),
                ),
                "failure" => (
                    format!("  {label}  "),
                    Style::new().fg(Color::White).bg(Color::Red),
                ),
                "plain" => (
                    format!("  {label}  "),
                    Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED),
                ),
                _ => (
                    format!("[ {label} ]"),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
            };
            if !enabled {
                style = Style::new().fg(MUTED);
            }
            if focused {
                style = style.add_modifier(Modifier::REVERSED);
            }
            put(buf, r.x, r.y, &text, r.width, style);
            control(out, id, r, enabled);
        }
        Component::Input => draw_input(env, out, buf, n, r),
        Component::Checkbox => {
            let on = st(a, "bind")
                .and_then(|p| env.client.value(p))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let (mark, color) = match (st(a, "variant") == Some("switch"), on) {
                (true, true) => ("━━●", Color::Green),
                (true, false) => ("●━━", MUTED),
                (false, true) => ("[x]", ACCENT),
                (false, false) => ("[ ]", Color::Reset),
            };
            let mut style = Style::new().fg(if enabled { color } else { MUTED });
            if focused {
                style = style.add_modifier(Modifier::REVERSED);
            }
            put(buf, r.x, r.y, mark, 3, style);
            let inner = r.width.saturating_sub(4);
            let lstyle = text_style(enabled);
            put_lines(
                buf,
                r.x + 4,
                r.y,
                &wrap(st(a, "label").unwrap_or(""), inner),
                inner,
                lstyle,
            );
            control(out, id, r, enabled);
        }
        Component::Select => draw_select(env, out, buf, n, r),
        Component::Slider => {
            let (min, max, step) = slider_range(a);
            let v = slider_value(n, env.client);
            put(
                buf,
                r.x,
                r.y,
                st(a, "label").unwrap_or(""),
                r.width,
                label_style(focused, enabled),
            );
            let value = format_number(v, step);
            let vw = value.width() as u16;
            put(
                buf,
                r.right().saturating_sub(vw),
                r.y,
                &value,
                vw,
                text_style(enabled),
            );
            let bw = r.width.max(1);
            let f = if max > min {
                (v - min) / (max - min)
            } else {
                0.0
            };
            let knob = (f * (bw - 1) as f64).round() as u16;
            let fill = Style::new().fg(if enabled { ACCENT } else { MUTED });
            put(buf, r.x, r.y + 1, &"━".repeat(knob as usize), knob, fill);
            let mut ks = fill.add_modifier(Modifier::BOLD);
            if focused {
                ks = ks.add_modifier(Modifier::REVERSED);
            }
            put(buf, r.x + knob, r.y + 1, "●", 1, ks);
            let rest = bw - knob - 1;
            put(
                buf,
                r.x + knob + 1,
                r.y + 1,
                &"─".repeat(rest as usize),
                rest,
                Style::new().fg(MUTED),
            );
            control(out, id, r, enabled);
            if enabled {
                out.hits.push(Hit {
                    id: id.to_string(),
                    rect: Rect::new(r.x, r.y + 1, bw, 1),
                    part: Part::SliderBar,
                });
            }
        }
    }
}

fn draw_progress(env: &Env, out: &mut Layer, buf: &mut Buffer, n: &ResolvedTree, r: Rect) {
    let a = &n.attrs;
    let label = st(a, "label");
    if st(a, "variant") == Some("spinner") {
        out.animating = true;
        put(buf, r.x, r.y, env.spinner(), 1, Style::new().fg(ACCENT));
        put(
            buf,
            r.x + 2,
            r.y,
            label.unwrap_or(""),
            r.width.saturating_sub(2),
            Style::new(),
        );
        return;
    }
    let mut y = r.y;
    if let Some(l) = label {
        put(buf, r.x, y, l, r.width, Style::new());
        y += 1;
    }
    let track = Style::new().fg(Color::Indexed(238));
    let fill = Style::new().fg(ACCENT);
    match a.get("value").and_then(Value::as_f64) {
        Some(v) => {
            let v = v.clamp(0.0, 1.0);
            let pct = format!(" {:>3.0}%", v * 100.0);
            let bw = r.width.saturating_sub(pct.len() as u16);
            let filled = (v * bw as f64).round() as u16;
            put(buf, r.x, y, &"█".repeat(filled as usize), filled, fill);
            put(
                buf,
                r.x + filled,
                y,
                &"░".repeat((bw - filled) as usize),
                bw - filled,
                track,
            );
            put(buf, r.x + bw, y, &pct, pct.len() as u16, Style::new());
        }
        None => {
            let bw = r.width;
            let seg = (bw / 4).max(3).min(bw);
            let span = bw - seg;
            let t = (env.tick / 40) as u16 % (2 * span).max(1);
            let at = if t < span { t } else { 2 * span - t };
            out.animating = true;
            put(buf, r.x, y, &"░".repeat(bw as usize), bw, track);
            put(buf, r.x + at, y, &"█".repeat(seg as usize), seg, fill);
        }
    }
}

fn draw_input(env: &Env, out: &mut Layer, buf: &mut Buffer, n: &ResolvedTree, r: Rect) {
    let a = &n.attrs;
    let id = n.id.as_str();
    let focused = env.focused(id);
    let enabled = env.enabled(n);
    let ed = env.editor(id);
    let mut y = r.y;
    if let Some(l) = st(a, "label") {
        put(buf, r.x, y, l, r.width, label_style(focused, enabled));
        y += 1;
    }
    let placeholder = st(a, "placeholder").filter(|_| ed.text().is_empty());
    let ph_style = Style::new().fg(Color::Gray).add_modifier(Modifier::DIM);
    let error = st(a, "error").filter(|e| !e.is_empty());
    if st(a, "variant") == Some("multiline") {
        let inner = r.width.saturating_sub(2);
        let (lines, (cr, cc)) = ed.wrapped(inner);
        let rows = lines.len().clamp(MULTI_MIN, MULTI_MAX) as u16;
        let offset = if focused {
            (cr + 1).saturating_sub(rows)
        } else {
            0
        };
        let border = match (error, focused) {
            (Some(_), _) => Color::Red,
            (None, true) => ACCENT,
            _ => MUTED,
        };
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border))
            .render(Rect::new(r.x, y, r.width, rows + 2), buf);
        let visible: Vec<String> = lines
            .iter()
            .skip(offset as usize)
            .take(rows as usize)
            .cloned()
            .collect();
        put_lines(buf, r.x + 1, y + 1, &visible, inner, text_style(enabled));
        if let Some(p) = placeholder {
            put(buf, r.x + 1, y + 1, p, inner, ph_style);
        }
        if focused && enabled {
            out.cursor = Some(Position::new(r.x + 1 + cc, y + 1 + cr - offset));
        }
        y += rows + 2;
    } else {
        let style = Style::new()
            .fg(if enabled { Color::White } else { MUTED })
            .bg(if focused { FIELD_FOCUS_BG } else { FIELD_BG });
        let style = if error.is_some() {
            style
                .underline_color(Color::Red)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            style
        };
        buf.set_style(Rect::new(r.x, y, r.width, 1), style);
        let mask = st(a, "variant") == Some("password");
        let (text, col) = ed.view(r.width.saturating_sub(2), mask);
        put(buf, r.x + 1, y, &text, r.width.saturating_sub(2), style);
        if let Some(p) = placeholder {
            put(
                buf,
                r.x + 1,
                y,
                p,
                r.width.saturating_sub(2),
                style.patch(ph_style),
            );
        }
        if focused && enabled {
            out.cursor = Some(Position::new(r.x + 1 + col, y));
        }
        y += 1;
    }
    if let Some(e) = error {
        put_lines(
            buf,
            r.x,
            y,
            &wrap(e, r.width),
            r.width,
            Style::new().fg(Color::Red),
        );
    }
    control(out, id, r, enabled);
}

fn draw_select(env: &Env, out: &mut Layer, buf: &mut Buffer, n: &ResolvedTree, r: Rect) {
    let a = &n.attrs;
    let id = n.id.as_str();
    let focused = env.focused(id);
    let enabled = env.enabled(n);
    let opts = options(a);
    let cur = selected_option(n, env.client, &opts);
    let mut y = r.y;
    if let Some(l) = st(a, "label") {
        put(buf, r.x, y, l, r.width, label_style(focused, enabled));
        y += 1;
    }
    let opt_hit = |out: &mut Layer, i: usize, rect: Rect| {
        if enabled {
            out.hits.push(Hit {
                id: id.to_string(),
                rect,
                part: Part::Option(i),
            });
        }
    };
    match st(a, "variant").unwrap_or("dropdown") {
        "radio" => {
            control(out, id, r, enabled);
            for (i, (_, label)) in opts.iter().enumerate() {
                let on = cur == Some(i);
                let mut ms = Style::new().fg(if !enabled {
                    MUTED
                } else if on {
                    ACCENT
                } else {
                    Color::Reset
                });
                if focused && (on || (cur.is_none() && i == 0)) {
                    ms = ms.add_modifier(Modifier::REVERSED);
                }
                put(buf, r.x, y, if on { "(•)" } else { "( )" }, 3, ms);
                put(
                    buf,
                    r.x + 4,
                    y,
                    label,
                    r.width.saturating_sub(4),
                    text_style(enabled),
                );
                opt_hit(out, i, Rect::new(r.x, y, r.width, 1));
                y += 1;
            }
        }
        "segmented" => {
            control(out, id, r, enabled);
            let mut x = r.x;
            for (i, (_, label)) in opts.iter().enumerate() {
                let text = format!(" {label} ");
                let w = (text.width() as u16).min(r.right().saturating_sub(x));
                let mut style = if cur == Some(i) {
                    Style::new()
                        .fg(Color::Black)
                        .bg(if enabled { ACCENT } else { MUTED })
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                        .fg(if enabled { Color::White } else { MUTED })
                        .bg(FIELD_BG)
                };
                if focused {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                put(buf, x, y, &text, w, style);
                opt_hit(out, i, Rect::new(x, y, w, 1));
                x = x.saturating_add(w + 1);
            }
        }
        _ => {
            let field = Rect::new(r.x, y, r.width, 1);
            let style = Style::new()
                .fg(if enabled { Color::White } else { MUTED })
                .bg(if focused { FIELD_FOCUS_BG } else { FIELD_BG });
            buf.set_style(field, style);
            match cur {
                Some(i) => put(
                    buf,
                    r.x + 1,
                    y,
                    &opts[i].1,
                    r.width.saturating_sub(4),
                    style,
                ),
                None => put(
                    buf,
                    r.x + 1,
                    y,
                    "Choose…",
                    r.width.saturating_sub(4),
                    style.add_modifier(Modifier::DIM),
                ),
            }
            put(buf, r.right().saturating_sub(2), y, "▾", 1, style);
            if env.page.popup.as_ref().is_some_and(|p| p.id == id) {
                out.anchor = Some(field);
            }
            control(out, id, r, enabled);
        }
    }
}

/// Registers a control as focusable and clickable.
fn control(out: &mut Layer, id: &str, r: Rect, enabled: bool) {
    if enabled {
        out.focusables.push((id.to_string(), r));
        out.hits.push(Hit {
            id: id.to_string(),
            rect: r,
            part: Part::Control,
        });
    }
}

// --- helpers -----------------------------------------------------------------

fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, max: u16, style: Style) {
    let area = buf.area;
    if y < area.top() || y >= area.bottom() || x >= area.right() {
        return;
    }
    buf.set_stringn(x, y, s, max as usize, style);
}

fn put_lines(buf: &mut Buffer, x: u16, y: u16, lines: &[String], max: u16, style: Style) {
    for (i, l) in lines.iter().enumerate() {
        put(buf, x, y + i as u16, l, max, style);
    }
}

/// Copies rows of `src` starting at `src_y` into `dst` at `to`.
fn blit(src: &Buffer, src_y: u16, dst: &mut Buffer, to: Rect) {
    for row in 0..to.height {
        let sy = src_y + row;
        if sy >= src.area.height {
            break;
        }
        for col in 0..to.width.min(src.area.width) {
            if let (Some(c), Some(d)) =
                (src.cell((col, sy)), dst.cell_mut((to.x + col, to.y + row)))
            {
                *d = c.clone();
            }
        }
    }
}

fn wrap(text: &str, w: u16) -> Vec<String> {
    let w = w.max(1) as usize;
    text.split('\n')
        .flat_map(|p| {
            let lines: Vec<String> = textwrap::wrap(p, w)
                .into_iter()
                .map(|c| c.into_owned())
                .collect();
            if lines.is_empty() {
                vec![String::new()]
            } else {
                lines
            }
        })
        .collect()
}

fn walk(tree: &[ResolvedTree], f: &mut impl FnMut(&ResolvedTree)) {
    for n in tree {
        f(n);
        walk(&n.children, f);
    }
}

fn find<'a>(tree: &'a [ResolvedTree], id: &str) -> Option<&'a ResolvedTree> {
    tree.iter().find_map(|n| {
        if n.id == id {
            Some(n)
        } else {
            find(&n.children, id)
        }
    })
}

fn shown(kids: &[ResolvedTree]) -> impl Iterator<Item = &ResolvedTree> {
    kids.iter().filter(|k| bl(&k.attrs, "visible"))
}

fn tab_label(t: &ResolvedTree) -> &str {
    st(&t.attrs, "label").unwrap_or(&t.id)
}

fn image_text(a: &Map<String, Value>) -> String {
    match st(a, "alt").filter(|s| !s.is_empty()) {
        Some(alt) => format!("▣ {alt}"),
        None => "▣ image".into(),
    }
}

fn variant_color(v: &str) -> Color {
    match v {
        "info" => Color::Blue,
        "success" => Color::Green,
        "warning" => Color::Yellow,
        "failure" => Color::Red,
        _ => Color::Gray,
    }
}

fn label_style(focused: bool, enabled: bool) -> Style {
    match (focused, enabled) {
        (_, false) => Style::new().fg(MUTED),
        (true, true) => Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        (false, true) => Style::new().add_modifier(Modifier::BOLD),
    }
}

fn text_style(enabled: bool) -> Style {
    if enabled {
        Style::new()
    } else {
        Style::new().fg(MUTED)
    }
}

/// A select's options as (value, label).
fn options(a: &Map<String, Value>) -> Vec<(String, String)> {
    a.get("options")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|o| {
                    let o = o.as_object()?;
                    Some((st(o, "value")?.to_string(), st(o, "label")?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn selected_option(n: &ResolvedTree, client: &Client, opts: &[(String, String)]) -> Option<usize> {
    let v = st(&n.attrs, "bind")
        .and_then(|p| client.value(p))
        .and_then(Value::as_str)?;
    opts.iter().position(|o| o.0 == v)
}

fn slider_range(a: &Map<String, Value>) -> (f64, f64, f64) {
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
    (min, max, step)
}

fn slider_value(n: &ResolvedTree, client: &Client) -> f64 {
    let (min, max, _) = slider_range(&n.attrs);
    st(&n.attrs, "bind")
        .and_then(|p| client.value(p))
        .and_then(Value::as_f64)
        .unwrap_or(min)
        .clamp(min, max)
}

fn st<'a>(a: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

fn bl(a: &Map<String, Value>, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn session(lines: &[&str]) -> (Client, Vec<ResolvedTree>) {
        let mut c = Client::new();
        c.hello();
        for l in lines {
            c.receive_line(l);
        }
        let t = c.tree();
        (c, t)
    }

    fn render(page: &mut Page, client: &Client, tree: &[ResolvedTree], w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        page.draw(area, &mut buf, tree, client);
        buf
    }

    fn text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn draws_and_presses_a_button() {
        let (c, t) = session(&[
            r#"element {"id":"t","type":"text","text":"Hello","variant":"h1"}"#,
            r#"element {"id":"b","type":"button","label":"Go","onPress":{"action":"go"}}"#,
        ]);
        let mut p = Page::new();
        let buf = render(&mut p, &c, &t, 30, 8);
        let s = text(&buf);
        assert!(s.contains("Hello"), "{s}");
        assert!(s.contains("[ Go ]"), "{s}");
        p.key(KeyEvent::from(KeyCode::Tab), &t, &c);
        let acts = p.key(KeyEvent::from(KeyCode::Enter), &t, &c);
        assert_eq!((acts[0].id.as_str(), acts[0].event), ("b", Some("onPress")));
    }

    #[test]
    fn disconnected_controls_are_not_focusable() {
        let (mut c, _) = session(&[r#"element {"id":"b","type":"button","label":"Go"}"#]);
        c.close();
        let t = c.tree();
        let mut p = Page::new();
        render(&mut p, &c, &t, 30, 8);
        assert!(p.doc.focusables.is_empty());
    }

    #[test]
    fn row_shares_width_and_justifies() {
        let (c, t) = session(&[
            r#"element {"id":"r","type":"row","justify":"between"}"#,
            r#"element {"id":"a","type":"text","text":"left","parent":"r"}"#,
            r#"element {"id":"b","type":"badge","text":"right","parent":"r"}"#,
        ]);
        let mut p = Page::new();
        let s = text(&render(&mut p, &c, &t, 23, 4));
        assert!(s.lines().any(|l| l == " left          right"), "{s}");
    }

    #[test]
    fn typing_writes_the_bound_signal() {
        let (c, t) = session(&[
            r#"signal {"name":""}"#,
            r#"element {"id":"i","type":"input","bind":"name","label":"Name"}"#,
        ]);
        let mut p = Page::new();
        p.sync(&t, &c);
        render(&mut p, &c, &t, 30, 8);
        p.focus = Some("i".into());
        let k = KeyEvent::from(KeyCode::Char('x'));
        let acts = p.key(k, &t, &c);
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].value, Some(json!("x")));
        assert!(p.next_deadline().is_some());
        // Leaving the input reports the edit at once.
        let acts = p.key(KeyEvent::from(KeyCode::Esc), &t, &c);
        assert_eq!(acts[0].event, Some("onChange"));
    }

    #[test]
    fn dialog_draws_over_page() {
        let (c, t) = session(&[
            r#"element {"id":"bg","type":"text","text":"behind"}"#,
            r#"element {"id":"d","type":"dialog","title":"Confirm","onDismiss":{"action":"x"}}"#,
            r#"element {"id":"q","type":"text","text":"Sure?","parent":"d"}"#,
        ]);
        let mut p = Page::new();
        let s = text(&render(&mut p, &c, &t, 60, 12));
        assert!(
            s.contains("Confirm") && s.contains("Sure?") && s.contains("✕"),
            "{s}"
        );
        assert!(p.dialog.is_some());
    }

    #[test]
    fn number_helpers() {
        assert_eq!(parse_number(" 12 "), json!(12));
        assert_eq!(parse_number("1."), json!(1.0));
        assert_eq!(parse_number("-"), Value::Null);
        assert_eq!(format_number(0.25, 0.05), "0.25");
        assert_eq!(number(3.0), json!(3));
    }
}
