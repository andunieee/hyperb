//! A parser for `text/event-stream` (Server-Sent Events, as the HTML
//! standard defines it), fed with bytes as they arrive. Only the `event`
//! and `data` fields are used; `id`, `retry` and comments are skipped.

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// The `event` field, or `message` if the event had none.
    pub event: String,
    /// The `data` fields joined by line feeds.
    pub data: String,
}

#[derive(Debug, Default)]
pub struct Parser {
    line: Vec<u8>,
    event: String,
    data: String,
    /// The last byte was a carriage return, so a line feed now is part of it.
    after_cr: bool,
    started: bool,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes and returns the events they complete.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        for &b in bytes {
            if std::mem::take(&mut self.after_cr) && b == b'\n' {
                continue;
            }
            match b {
                b'\r' => {
                    self.after_cr = true;
                    self.end_line(&mut out);
                }
                b'\n' => self.end_line(&mut out),
                _ => self.line.push(b),
            }
        }
        out
    }

    fn end_line(&mut self, out: &mut Vec<Event>) {
        let bytes = std::mem::take(&mut self.line);
        let mut line = String::from_utf8_lossy(&bytes).into_owned();
        if !std::mem::replace(&mut self.started, true) && line.starts_with('\u{feff}') {
            line.remove(0);
        }
        if line.is_empty() {
            let event = std::mem::take(&mut self.event);
            if !self.data.is_empty() {
                let mut data = std::mem::take(&mut self.data);
                data.pop();
                out.push(Event {
                    event: if event.is_empty() {
                        "message".into()
                    } else {
                        event
                    },
                    data,
                });
            }
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_str(), ""),
        };
        match field {
            "event" => self.event = value.to_string(),
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(event: &str, data: &str) -> Event {
        Event {
            event: event.into(),
            data: data.into(),
        }
    }

    #[test]
    fn events() {
        let mut p = Parser::new();
        let got = p.push(b"event: element\ndata: {\"id\":\"a\"}\n\n: ping\n\ndata: x\ndata:y\n\n");
        assert_eq!(
            got,
            vec![ev("element", "{\"id\":\"a\"}"), ev("message", "x\ny")]
        );
    }

    #[test]
    fn split_anywhere() {
        let input = "\u{feff}event: signal\r\ndata: {\"s\":\"ü\"}\r\n\r\nevent:remove\rdata:{}\r\r";
        let whole = Parser::new().push(input.as_bytes());
        assert_eq!(
            whole,
            vec![ev("signal", "{\"s\":\"ü\"}"), ev("remove", "{}")]
        );
        for chunk in 1..input.len() {
            let mut p = Parser::new();
            let got: Vec<_> = input
                .as_bytes()
                .chunks(chunk)
                .flat_map(|c| p.push(c))
                .collect();
            assert_eq!(got, whole, "chunk size {chunk}");
        }
    }

    #[test]
    fn incomplete_and_empty_events() {
        let mut p = Parser::new();
        assert_eq!(p.push(b"event: x\n\nid: 1\nretry: 5\n\ndata: half"), vec![]);
        assert_eq!(p.push(b"\n\n"), vec![ev("message", "half")]);
    }
}
