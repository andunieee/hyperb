//! hyperb in the terminal: a browser for hypereffect servers.
//!
//! ```text
//! hyperb-tui [address]
//! ```
//!
//! Terminal input and network events arrive on one channel,
//! so the loop sleeps until something happens, an onChange is due, or an
//! animation needs its next frame.

mod browser;
mod edit;
mod page;

use browser::Browser;
use hyperb_core::identity::Identity;
use hyperb_core::net;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::SetTitle;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::{DefaultTerminal, Frame};
use std::io::{self, stdout};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

pub enum Msg {
    Term(Event),
    Net { generation: u64, event: net::Event },
}

const FRAME: Duration = Duration::from_millis(80);
const IDLE: Duration = Duration::from_secs(60);

fn main() -> io::Result<()> {
    let url = std::env::args().nth(1);
    let identity = Identity::load_default().unwrap_or_else(|e| {
        eprintln!("hyperb: cannot load identity ({e}); using a temporary one");
        Identity::generate()
    });
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
        hook(info);
    }));
    let result = run(&mut terminal, url.as_deref(), identity);
    let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, url: Option<&str>, identity: Identity) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.send(Msg::Term(ev)).is_err() {
                    break;
                }
            }
        });
    }
    let mut app = App::new(tx, url, identity);
    let mut title = String::new();
    while !app.quit {
        terminal.draw(|f| app.draw(f))?;
        let t = app.window_title();
        if t != title {
            execute!(stdout(), SetTitle(&t))?;
            title = t;
        }
        match rx.recv_timeout(app.timeout()) {
            Ok(m) => {
                app.handle(m);
                // Take everything that has arrived before redrawing.
                while let Ok(m) = rx.try_recv() {
                    app.handle(m);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        app.tick(Instant::now());
    }
    Ok(())
}

struct App {
    browser: Browser,
    quit: bool,
}

impl App {
    fn new(tx: Sender<Msg>, url: Option<&str>, identity: Identity) -> App {
        let mut browser = Browser::new(tx, identity);
        if let Some(u) = url {
            browser.navigate(u);
        }
        App {
            browser,
            quit: false,
        }
    }

    fn timeout(&self) -> Duration {
        if self.browser.animating() {
            return FRAME;
        }
        self.browser
            .next_deadline()
            .map_or(IDLE, |d| d.saturating_duration_since(Instant::now()))
    }

    fn tick(&mut self, now: Instant) {
        self.browser.tick(now);
    }

    fn window_title(&self) -> String {
        format!("{} — hyperb", self.browser.title)
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Net { generation, event } => self.browser.net(generation, event),
            Msg::Term(Event::Key(k)) if k.kind != KeyEventKind::Release => self.key(k),
            Msg::Term(Event::Mouse(m)) => self.browser.mouse(m),
            Msg::Term(Event::Paste(s)) => self.browser.paste(&s),
            Msg::Term(_) => {}
        }
    }

    fn key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('q' | 'c') if ctrl => self.quit = true,
            KeyCode::Char('l') if ctrl => self.browser.focus_url(),
            KeyCode::F(6) => self.browser.focus_url(),
            KeyCode::Char('r') if ctrl => self.browser.reload(),
            KeyCode::F(5) => self.browser.reload(),
            _ => self.browser.key(k),
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        if area.height < 2 || area.width < 10 {
            return;
        }
        let content = Rect::new(area.x, area.y, area.width, area.height - 1);
        if let Some(c) = self.browser.draw(content, f.buffer_mut()) {
            f.set_cursor_position(c);
        }
        let status = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        let hints = if self.browser.url_focused {
            "Enter connect · Esc back · Ctrl+Q quit"
        } else {
            "Tab/↑↓ move · Enter/Space activate · Esc back · Ctrl+L address · F5 reconnect · Ctrl+Q quit"
        };
        f.buffer_mut().set_stringn(
            status.x + 1,
            status.y,
            hints,
            status.width.saturating_sub(2) as usize,
            Style::new().fg(Color::DarkGray),
        );
    }
}
