//! The browser: an address bar, a connection to a hypereffect server, and
//! the rendered session.
//!
//! Entering an address opens a connection; when it opens the browser sends
//! `hello` and from then on feeds every frame to the [`Client`]. When the
//! transport closes the last state stays on screen with controls disabled,
//! and a banner offers to reconnect.

use crate::edit::Editor;
use crate::page::{Action, Page, ACCENT, SPINNER};
use crate::Msg;
use hyperb_core::address;
use hyperb_core::hypereffect::client::{Client, ResolvedTree};
use hyperb_core::identity::Identity;
use hyperb_core::net::{Connection, Event};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use std::io::Write;
use std::sync::mpsc::Sender;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;
use url::Url;

pub struct Browser {
    pub title: String,
    pub url_focused: bool,
    url: Editor,
    session: Session,
    page: Page,
    /// The resolved tree, rebuilt whenever the client reports changes.
    tree: Vec<ResolvedTree>,
    /// Why the last address could not be used.
    error: Option<String>,
    tx: Sender<Msg>,
    epoch: Instant,
    rects: Rects,
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

/// Where things were on the last frame.
#[derive(Default)]
struct Rects {
    reload: Rect,
    url: Rect,
    banner: Option<Rect>,
    content: Rect,
}

impl Browser {
    pub fn new(tx: Sender<Msg>, identity: Identity) -> Browser {
        Browser {
            title: "hyperb".into(),
            url_focused: true,
            url: Editor::default(),
            session: Session {
                client: Client::new(),
                conn: None,
                url: None,
                generation: 0,
                state: State::Idle,
            },
            page: Page::new(),
            tree: Vec::new(),
            error: None,
            tx,
            epoch: Instant::now(),
            rects: Rects::default(),
            identity,
        }
    }

    pub fn animating(&self) -> bool {
        self.session.state == State::Connecting || self.page.animating
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.page.next_deadline()
    }

    pub fn tick(&mut self, now: Instant) {
        let acts = self.page.due(now);
        self.apply(acts);
    }

    pub fn focus_url(&mut self) {
        self.url_focused = true;
        self.url.cursor_to_end();
    }

    /// Connects to what the user typed.
    pub fn navigate(&mut self, input: &str) {
        match address::normalize(input) {
            Ok(url) => {
                self.url.set_text(url.as_str());
                self.url.cursor_to_end();
                self.title = address::label(&url);
                self.url_focused = false;
                self.error = None;
                self.connect(url);
            }
            Err(e) => self.error = Some(format!("Invalid address: {e}")),
        }
    }

    pub fn reload(&mut self) {
        match self.session.url.clone() {
            Some(url) => self.connect(url),
            None => {
                let input = self.url.text().to_string();
                self.navigate(&input);
            }
        }
    }

    fn connect(&mut self, url: Url) {
        let conn = Connection::open(&url, &self.identity);
        let events = conn.events();
        let s = &mut self.session;
        // Until the new transport opens, nothing may be sent: hello must come first.
        s.client.close();
        s.conn = Some(conn);
        s.url = Some(url);
        s.generation += 1;
        s.state = State::Connecting;
        let (tx, generation) = (self.tx.clone(), s.generation);
        std::thread::spawn(move || {
            while let Ok(event) = events.recv_blocking() {
                if tx.send(Msg::Net { generation, event }).is_err() {
                    break;
                }
            }
        });
        self.refresh();
    }

    pub fn net(&mut self, generation: u64, ev: Event) {
        let s = &mut self.session;
        if s.generation != generation {
            return;
        }
        match ev {
            Event::Open => {
                let hello = s.client.hello();
                if let Some(c) = &s.conn {
                    c.send(hello);
                }
                s.state = State::Open;
            }
            Event::Data(lines) => s.client.receive(lines.as_bytes()),
            Event::Closed(reason) => {
                s.client.close();
                s.conn = None;
                s.state = State::Closed(reason);
            }
        }
        self.refresh();
    }

    /// Picks up what changed in the client.
    fn refresh(&mut self) {
        let s = &mut self.session;
        let diags = s.client.take_diagnostics();
        if !diags.is_empty() {
            let url = s.url.as_ref().map(Url::as_str).unwrap_or("-");
            log(url, &diags);
        }
        if !s.client.take_changes().is_empty() {
            self.tree = s.client.tree();
            self.page.sync(&self.tree, &s.client);
        }
    }

    fn apply(&mut self, acts: Vec<Action>) {
        if acts.is_empty() {
            return;
        }
        for a in acts {
            self.input(&a.id, a.value, a.event);
        }
        self.refresh();
    }

    /// User input on element `id`: writes `value` to its bound signal, if
    /// any, then runs the handler in attribute `event`, if any.
    fn input(
        &mut self,
        id: &str,
        value: Option<hyperb_core::hypereffect::Value>,
        event: Option<&str>,
    ) {
        let s = &mut self.session;
        if let Some(v) = value {
            let bind = s.client.attr(id, "bind");
            if let Some(path) = bind.as_ref().and_then(|b| b.as_str()) {
                s.client.write(path, v);
            }
        }
        if let Some(line) = event.and_then(|e| s.client.trigger(id, e)) {
            if let Some(c) = &s.conn {
                c.send(line);
            }
        }
    }

    fn welcome(&self) -> bool {
        self.session.url.is_none()
    }

    // --- input ---------------------------------------------------------------

    pub fn key(&mut self, k: KeyEvent) {
        if self.url_focused {
            match k.code {
                KeyCode::Enter => {
                    let input = self.url.text().to_string();
                    self.navigate(&input);
                }
                KeyCode::Esc | KeyCode::Tab | KeyCode::Down => {
                    if let Some(u) = &self.session.url {
                        self.url.set_text(u.as_str());
                    }
                    self.url_focused = false;
                }
                _ => {
                    if self.url.key(&k, false) == crate::edit::Edit::Changed {
                        self.error = None;
                    }
                }
            }
            return;
        }
        if self.welcome() {
            match k.code {
                KeyCode::Enter | KeyCode::Esc => self.focus_url(),
                KeyCode::Char(_) => {
                    self.focus_url();
                    self.url.key(&k, false);
                }
                _ => {}
            }
            return;
        }
        let acts = self.page.key(k, &self.tree, &self.session.client);
        self.apply(acts);
    }

    pub fn paste(&mut self, text: &str) {
        if self.url_focused {
            self.url.insert(&text.replace(['\r', '\n'], ""));
        } else if !self.welcome() {
            let acts = self.page.paste(text, &self.tree, &self.session.client);
            self.apply(acts);
        }
    }

    pub fn mouse(&mut self, m: MouseEvent) {
        let p = Position::new(m.column, m.row);
        if m.kind == MouseEventKind::Down(MouseButton::Left) {
            if self.rects.url.contains(p) {
                self.focus_url();
                return;
            }
            if self.rects.reload.contains(p) || self.rects.banner.is_some_and(|b| b.contains(p)) {
                self.reload();
                return;
            }
            if self.welcome() {
                return;
            }
        }
        if self.welcome() {
            return;
        }
        // Wheel and drags go to the page wherever they are; clicks only inside it.
        let click = matches!(m.kind, MouseEventKind::Down(_));
        if click && !self.rects.content.contains(p) {
            return;
        }
        if click {
            self.url_focused = false;
        }
        let acts = self.page.mouse(m, &self.tree, &self.session.client);
        self.apply(acts);
    }

    // --- drawing -------------------------------------------------------------

    /// Draws the browser and returns where the cursor goes.
    pub fn draw(&mut self, area: Rect, buf: &mut Buffer) -> Option<Position> {
        let tick = self.epoch.elapsed().as_millis() as u64;
        let bar = Rect::new(area.x, area.y, area.width, 1);
        let mut rest = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );

        // Address bar: reload button, connection state, address.
        let bar_style = Style::new().bg(Color::Indexed(235));
        buf.set_style(bar, bar_style);
        self.rects.reload = Rect::new(bar.x, bar.y, 3, 1);
        buf.set_stringn(bar.x, bar.y, " ⟳ ", 3, bar_style.fg(ACCENT));
        let (glyph, color) = match (
            &self.session.state,
            self.session.url.as_ref().map(Url::scheme),
        ) {
            (State::Connecting, _) => {
                (SPINNER[(tick / 80) as usize % SPINNER.len()], Color::Yellow)
            }
            (State::Open, Some("https" | "iroh")) => ("●", Color::Green),
            (State::Open, _) => ("●", Color::Yellow),
            (State::Closed(_), _) => ("●", Color::Red),
            (State::Idle, _) => (" ", Color::Reset),
        };
        buf.set_stringn(bar.x + 3, bar.y, glyph, 1, bar_style.fg(color));
        let field = Rect::new(bar.x + 5, bar.y, bar.width.saturating_sub(6), 1);
        self.rects.url = field;
        let fstyle = Style::new().fg(Color::White).bg(if self.url_focused {
            Color::Indexed(238)
        } else {
            Color::Indexed(236)
        });
        buf.set_style(field, fstyle);
        let mut cursor = None;
        if self.url.text().is_empty() {
            let hint = "Server address, e.g. https://example.com/app or localhost:9001/restaurant";
            buf.set_stringn(
                field.x + 1,
                field.y,
                hint,
                field.width.saturating_sub(2) as usize,
                fstyle.fg(Color::Gray).add_modifier(Modifier::DIM),
            );
        }
        let (text, col) = self.url.view(field.width.saturating_sub(2), false);
        buf.set_stringn(
            field.x + 1,
            field.y,
            text,
            field.width.saturating_sub(2) as usize,
            fstyle,
        );
        if self.url_focused {
            cursor = Some(Position::new(field.x + 1 + col, field.y));
        }

        // Banner.
        let banner = match (&self.error, &self.session.state) {
            (Some(e), _) => Some(e.clone()),
            (None, State::Closed(Some(r))) => {
                Some(format!("Disconnected: {r}  ·  F5 or click to reconnect"))
            }
            (None, State::Closed(None)) => Some(
                "Disconnected. The server closed the connection.  ·  F5 or click to reconnect"
                    .into(),
            ),
            _ => None,
        };
        self.rects.banner = None;
        if let Some(text) = banner {
            let b = Rect::new(rest.x, rest.y, rest.width, 1);
            let style = Style::new().fg(Color::Black).bg(Color::Yellow);
            buf.set_style(b, style);
            buf.set_stringn(
                b.x + 1,
                b.y,
                format!("✗ {text}"),
                b.width.saturating_sub(2) as usize,
                style,
            );
            if self.error.is_none() {
                self.rects.banner = Some(b);
            }
            rest = Rect::new(
                rest.x,
                rest.y + 1,
                rest.width,
                rest.height.saturating_sub(1),
            );
        }

        self.rects.content = rest;
        if self.welcome() {
            self.draw_welcome(rest, buf);
            return cursor;
        }
        let page_cursor = self.page.draw(rest, buf, &self.tree, &self.session.client);
        cursor.or(page_cursor)
    }

    fn draw_welcome(&mut self, area: Rect, buf: &mut Buffer) {
        let hint =
            "Enter the address of a hypereffect server, e.g. hyperb://example.com/app. \
                    Remote servers use https; localhost, IP addresses and .onion use http.";
        let hint = textwrap::wrap(hint, (area.width.saturating_sub(4) as usize).clamp(1, 64));
        let height = 2 + hint.len() as u16;
        let mut y = area.y + area.height.saturating_sub(height) / 2;
        let center = |buf: &mut Buffer, y: u16, s: &str, style: Style| {
            let w = s.width() as u16;
            if y < area.bottom() {
                buf.set_stringn(
                    area.x + area.width.saturating_sub(w) / 2,
                    y,
                    s,
                    area.width as usize,
                    style,
                );
            }
        };
        center(
            buf,
            y,
            "hyperb",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        );
        y += 2;
        for l in &hint {
            center(buf, y, l, Style::new().fg(Color::Gray));
            y += 1;
        }
    }
}

/// Diagnostics would garble the screen, so they go to the file named by
/// `HYPERB_LOG`, if set.
fn log(url: &str, diags: &[String]) {
    let Some(path) = std::env::var_os("HYPERB_LOG") else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        for d in diags {
            let _ = writeln!(f, "[{url}] {d}");
        }
    }
}
