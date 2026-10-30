//! The browser: a connection to the one app given on the command line, and
//! the rendered session, under a bar with its address.
//!
//! It connects at once; when the connection opens the browser sends `hello`
//! and from then on feeds every frame to the [`Client`]. When the transport
//! closes the last state stays on screen with controls disabled, and a banner
//! offers to reconnect.
//!
//! The window is titled with the name the app gives itself in its metadata.
//!
//! URLs that buttons ask to open go to the system's opener when there is a
//! graphical session, and the status line shows them for a while either way.
//! A hyperb address replaces the session, as there is only one.

use crate::page::{Action, Page, ACCENT, SPINNER};
use crate::Msg;
use hyperb_core::address;
use hyperb_core::hypereffect::client::{Client, ResolvedTree};
use hyperb_core::identity::Identity;
use hyperb_core::net::{Connection, Event};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use url::Url;

pub struct Browser {
    pub title: String,
    session: Session,
    page: Page,
    /// The resolved tree, rebuilt whenever the client reports changes.
    tree: Vec<ResolvedTree>,
    tx: Sender<Msg>,
    epoch: Instant,
    rects: Rects,
    identity: Identity,
    /// What the status line says instead of its hints, until when.
    notice: Option<(String, Instant)>,
}

/// How long a notice stays in the status line.
const NOTICE: Duration = Duration::from_secs(6);

struct Session {
    client: Client,
    conn: Option<Connection>,
    url: Url,
    /// Bumped on every connect, so events from replaced connections are dropped.
    generation: u64,
    state: State,
}

#[derive(Clone, PartialEq)]
enum State {
    Connecting,
    Open,
    Closed(Option<String>),
}

/// Where things were on the last frame.
#[derive(Default)]
struct Rects {
    reload: Rect,
    banner: Option<Rect>,
    content: Rect,
}

impl Browser {
    /// Connects to `url`.
    pub fn new(tx: Sender<Msg>, identity: Identity, url: Url) -> Browser {
        let mut b = Browser {
            title: address::label(&url),
            session: Session {
                client: Client::new(),
                conn: None,
                url,
                generation: 0,
                state: State::Connecting,
            },
            page: Page::new(),
            tree: Vec::new(),
            tx,
            epoch: Instant::now(),
            rects: Rects::default(),
            identity,
            notice: None,
        };
        b.reload();
        b
    }

    pub fn animating(&self) -> bool {
        self.session.state == State::Connecting || self.page.animating
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        let notice = self.notice.as_ref().map(|(_, until)| *until);
        self.page.next_deadline().into_iter().chain(notice).min()
    }

    /// What the status line shows instead of its hints, if anything.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|(n, _)| n.as_str())
    }

    pub fn tick(&mut self, now: Instant) {
        if self.notice.as_ref().is_some_and(|(_, until)| *until <= now) {
            self.notice = None;
        }
        let acts = self.page.due(now);
        self.apply(acts);
    }

    /// Connects again.
    pub fn reload(&mut self) {
        self.connect(self.session.url.clone());
    }

    fn connect(&mut self, url: Url) {
        let conn = Connection::open(&url, &self.identity);
        let events = conn.events();
        let s = &mut self.session;
        // Until the new transport opens, nothing may be sent: hello must come first.
        s.client.close();
        s.conn = Some(conn);
        s.url = url;
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
                let params = s
                    .conn
                    .as_ref()
                    .map(|c| c.params().clone())
                    .unwrap_or_default();
                let hello = s.client.hello_with(&params);
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
            log(s.url.as_str(), &diags);
        }
        let changes = s.client.take_changes();
        // Empty metadata is how every session starts: the title stays while
        // the app reconnects.
        let m = s.client.metadata();
        if changes.metadata && *m != Default::default() {
            self.title = m.name.clone().unwrap_or_else(|| address::label(&s.url));
        }
        if !changes.is_empty() {
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
        let line = event.and_then(|e| s.client.trigger(id, e));
        let mut app = None;
        for url in s.client.take_opens() {
            app = self.open(&url).or(app);
        }
        // The action still goes to the app that asked to open the address.
        if let Some(line) = line {
            if let Some(c) = &self.session.conn {
                c.send(line);
            }
        }
        if let Some(url) = app {
            self.title = address::label(&url);
            self.connect(url);
        }
    }

    /// Opens a URL a button asked for (`address::open`) and says so in the
    /// status line: a web page or an email address with the system's opener
    /// (without a graphical session the status line only shows it), while a
    /// hyperb address is returned for the session to connect to.
    fn open(&mut self, url: &str) -> Option<Url> {
        let (notice, app) = match address::open(url) {
            Err(e) => {
                log(
                    self.session.url.as_str(),
                    &[format!("not opening {url}: {e}")],
                );
                (format!("Not opened ({e}): {url}"), None)
            }
            Ok(address::Open::External(u)) if launch(u.as_str()) => (format!("Opened {u}"), None),
            Ok(address::Open::External(u)) => (format!("Open {u}"), None),
            Ok(address::Open::App(u)) => (format!("Opened {}", address::hyperb(&u)), Some(u)),
        };
        self.notice = Some((notice, Instant::now() + NOTICE));
        app
    }

    // --- input ---------------------------------------------------------------

    pub fn key(&mut self, k: KeyEvent) {
        let acts = self.page.key(k, &self.tree, &self.session.client);
        self.apply(acts);
    }

    pub fn paste(&mut self, text: &str) {
        let acts = self.page.paste(text, &self.tree, &self.session.client);
        self.apply(acts);
    }

    pub fn mouse(&mut self, m: MouseEvent) {
        let p = Position::new(m.column, m.row);
        let click = matches!(m.kind, MouseEventKind::Down(_));
        if m.kind == MouseEventKind::Down(MouseButton::Left)
            && (self.rects.reload.contains(p) || self.rects.banner.is_some_and(|b| b.contains(p)))
        {
            self.reload();
            return;
        }
        // Wheel and drags go to the page wherever they are; clicks only inside it.
        if click && !self.rects.content.contains(p) {
            return;
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
        let secure = address::is_secure(&self.session.url);
        let (glyph, color) = match &self.session.state {
            State::Connecting => (SPINNER[(tick / 80) as usize % SPINNER.len()], Color::Yellow),
            State::Open if secure => ("●", Color::Green),
            State::Open => ("●", Color::Yellow),
            State::Closed(_) => ("●", Color::Red),
        };
        buf.set_stringn(bar.x + 3, bar.y, glyph, 1, bar_style.fg(color));
        buf.set_stringn(
            bar.x + 5,
            bar.y,
            address::hyperb(&self.session.url),
            bar.width.saturating_sub(6) as usize,
            bar_style.fg(Color::Gray),
        );

        // Banner.
        let banner = match &self.session.state {
            State::Closed(Some(r)) => {
                Some(format!("Disconnected: {r}  ·  F5 or click to reconnect"))
            }
            State::Closed(None) => Some(
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
            self.rects.banner = Some(b);
            rest = Rect::new(
                rest.x,
                rest.y + 1,
                rest.width,
                rest.height.saturating_sub(1),
            );
        }

        self.rects.content = rest;
        self.page.draw(rest, buf, &self.tree, &self.session.client)
    }
}

/// Hands `url` to the system's opener, if there is a graphical session to
/// open it in. Returns whether it was handed over.
fn launch(url: &str) -> bool {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        let graphical = ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|v| std::env::var_os(v).is_some_and(|v| !v.is_empty()));
        if !graphical {
            return false;
        }
        Command::new("xdg-open")
    };
    let child = cmd
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
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
