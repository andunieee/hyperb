//! A small text editor for the address bar and for inputs: a string and a
//! cursor, with the usual readline keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

#[derive(Default, Clone)]
pub struct Editor {
    text: String,
    /// In chars.
    cursor: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Changed,
    /// The key was used but the text stayed the same.
    Moved,
    Ignored,
}

impl Editor {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text, keeping the cursor where it was if it still fits.
    pub fn set_text(&mut self, s: &str) {
        if self.text != s {
            self.text = s.to_string();
            self.cursor = self.cursor.min(self.len());
        }
    }

    pub fn cursor_to_end(&mut self) {
        self.cursor = self.len();
    }

    pub fn insert(&mut self, s: &str) {
        let b = self.byte(self.cursor);
        self.text.insert_str(b, s);
        self.cursor += s.chars().count();
    }

    pub fn key(&mut self, k: &KeyEvent, multiline: bool) -> Edit {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let chars: Vec<char> = self.text.chars().collect();
        let (start, end) = (
            line_start(&chars, self.cursor),
            line_end(&chars, self.cursor),
        );
        match k.code {
            KeyCode::Char('a') if ctrl => self.move_to(start),
            KeyCode::Char('e') if ctrl => self.move_to(end),
            KeyCode::Char('u') if ctrl => self.remove(start, self.cursor),
            KeyCode::Char('k') if ctrl => self.remove(self.cursor, end),
            KeyCode::Char(c) if !ctrl && !alt => {
                self.insert(c.encode_utf8(&mut [0; 4]));
                Edit::Changed
            }
            KeyCode::Enter if multiline => {
                self.insert("\n");
                Edit::Changed
            }
            KeyCode::Backspace if ctrl || alt => {
                self.remove(word_left(&chars, self.cursor), self.cursor)
            }
            KeyCode::Backspace if self.cursor > 0 => self.remove(self.cursor - 1, self.cursor),
            KeyCode::Delete if self.cursor < chars.len() => {
                self.remove(self.cursor, self.cursor + 1)
            }
            KeyCode::Backspace | KeyCode::Delete => Edit::Moved,
            KeyCode::Left if ctrl || alt => self.move_to(word_left(&chars, self.cursor)),
            KeyCode::Right if ctrl || alt => self.move_to(word_right(&chars, self.cursor)),
            KeyCode::Left => self.move_to(self.cursor.saturating_sub(1)),
            KeyCode::Right => self.move_to((self.cursor + 1).min(chars.len())),
            KeyCode::Home => self.move_to(start),
            KeyCode::End => self.move_to(end),
            // Up and Down leave the first and last lines, so focus can move on.
            KeyCode::Up if multiline && start > 0 => {
                let prev = line_start(&chars, start - 1);
                self.move_to(prev + (self.cursor - start).min(start - 1 - prev))
            }
            KeyCode::Down if multiline && end < chars.len() => {
                let next_end = line_end(&chars, end + 1);
                self.move_to(end + 1 + (self.cursor - start).min(next_end - end - 1))
            }
            _ => Edit::Ignored,
        }
    }

    /// The part of a single line that fits in `width` cells with the cursor
    /// in view, and the cursor's column in it. `mask` hides the characters.
    pub fn view(&self, width: u16, mask: bool) -> (String, u16) {
        let chars: Vec<char> = if mask {
            vec!['•'; self.len()]
        } else {
            self.text
                .chars()
                .map(|c| if c == '\n' { ' ' } else { c })
                .collect()
        };
        let w = width.max(1) as usize;
        let cw = |c: &char| c.width().unwrap_or(0);
        let before: usize = chars[..self.cursor].iter().map(cw).sum();
        let mut start = 0;
        if before >= w {
            let mut acc = 0;
            start = self.cursor;
            while start > 0 && acc + cw(&chars[start - 1]) < w {
                start -= 1;
                acc += cw(&chars[start]);
            }
        }
        let mut out = String::new();
        let mut used = 0;
        for c in &chars[start..] {
            if used + cw(c) > w {
                break;
            }
            used += cw(c);
            out.push(*c);
        }
        let col: usize = chars[start..self.cursor].iter().map(cw).sum();
        (out, col as u16)
    }

    /// The text broken into lines of at most `width` cells, and the cursor's
    /// (row, column).
    pub fn wrapped(&self, width: u16) -> (Vec<String>, (u16, u16)) {
        let w = width.max(1) as usize;
        let mut lines = Vec::new();
        let mut cur = String::new();
        let mut used = 0;
        let mut pos = None;
        for (i, c) in self.text.chars().enumerate() {
            if c == '\n' {
                if i == self.cursor {
                    pos = Some((lines.len(), used));
                }
                lines.push(std::mem::take(&mut cur));
                used = 0;
                continue;
            }
            let cw = c.width().unwrap_or(0);
            if used + cw > w {
                lines.push(std::mem::take(&mut cur));
                used = 0;
            }
            if i == self.cursor {
                pos = Some((lines.len(), used));
            }
            cur.push(c);
            used += cw;
        }
        let (mut row, mut col) = pos.unwrap_or((lines.len(), used));
        lines.push(cur);
        if col >= w {
            row += 1;
            col = 0;
            if row >= lines.len() {
                lines.push(String::new());
            }
        }
        (lines, (row as u16, col as u16))
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, ci: usize) -> usize {
        self.text
            .char_indices()
            .nth(ci)
            .map_or(self.text.len(), |(b, _)| b)
    }

    fn move_to(&mut self, ci: usize) -> Edit {
        self.cursor = ci;
        Edit::Moved
    }

    fn remove(&mut self, from: usize, to: usize) -> Edit {
        if from == to {
            return Edit::Moved;
        }
        let (a, b) = (self.byte(from), self.byte(to));
        self.text.replace_range(a..b, "");
        self.cursor = from;
        Edit::Changed
    }
}

fn line_start(chars: &[char], at: usize) -> usize {
    chars[..at]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |i| i + 1)
}

fn line_end(chars: &[char], at: usize) -> usize {
    chars[at..]
        .iter()
        .position(|c| *c == '\n')
        .map_or(chars.len(), |i| at + i)
}

fn word_left(chars: &[char], mut at: usize) -> usize {
    while at > 0 && !chars[at - 1].is_alphanumeric() {
        at -= 1;
    }
    while at > 0 && chars[at - 1].is_alphanumeric() {
        at -= 1;
    }
    at
}

fn word_right(chars: &[char], mut at: usize) -> usize {
    while at < chars.len() && !chars[at].is_alphanumeric() {
        at += 1;
    }
    while at < chars.len() && chars[at].is_alphanumeric() {
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press)
    }

    fn typed(s: &str) -> Editor {
        let mut e = Editor::default();
        e.insert(s);
        e
    }

    #[test]
    fn edits_at_cursor() {
        let mut e = typed("héllo");
        e.key(&key(KeyCode::Left), false);
        e.key(&key(KeyCode::Backspace), false);
        assert_eq!(e.text(), "hélo");
        e.key(&key(KeyCode::Home), false);
        e.key(&key(KeyCode::Char('>')), false);
        assert_eq!(e.text(), ">hélo");
    }

    #[test]
    fn view_scrolls_to_cursor() {
        let e = typed("abcdefghij");
        assert_eq!(e.view(4, false), ("hij".into(), 3));
        assert_eq!(typed("ab").view(4, true), ("••".into(), 2));
    }

    #[test]
    fn wraps_lines_and_cursor() {
        let e = typed("abcd\nef");
        assert_eq!(
            e.wrapped(3),
            (vec!["abc".into(), "d".into(), "ef".into()], (2, 2))
        );
        let e = typed("abc");
        assert_eq!(e.wrapped(3), (vec!["abc".into(), "".into()], (1, 0)));
    }

    #[test]
    fn moves_between_lines() {
        let mut e = typed("abcd\nef");
        assert!(e.key(&key(KeyCode::Up), true) == Edit::Moved);
        e.key(&key(KeyCode::Char('X')), true);
        assert_eq!(e.text(), "abXcd\nef");
        assert!(e.key(&key(KeyCode::Up), true) == Edit::Ignored);
    }
}
