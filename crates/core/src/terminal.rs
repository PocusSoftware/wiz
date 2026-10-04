//! Terminal emulation state (alacritty_terminal) fed by remote bytes.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Collects bytes the emulator wants written back to the remote (cursor position
/// reports, device attributes) and the latest window title.
#[derive(Clone, Default)]
pub struct Listener {
    replies: Arc<Mutex<Vec<u8>>>,
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(s) => self.replies.lock().unwrap().extend_from_slice(s.as_bytes()),
            Event::Title(t) => *self.title.lock().unwrap() = Some(t),
            _ => {}
        }
    }
}

pub struct Terminal {
    pub term: Term<Listener>,
    parser: Processor,
    listener: Listener,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        let listener = Listener::default();
        let size = Size {
            cols: cols as usize,
            rows: rows as usize,
        };
        Self {
            term: Term::new(Config::default(), &size, listener.clone()),
            parser: Processor::new(),
            listener,
        }
    }

    /// Feed remote output. Returns bytes that must be sent back to the remote.
    pub fn advance(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.parser.advance(&mut self.term, bytes);
        std::mem::take(&mut *self.listener.replies.lock().unwrap())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size {
            cols: cols as usize,
            rows: rows as usize,
        });
    }

    pub fn title(&self) -> Option<String> {
        self.listener.title.lock().unwrap().clone()
    }

    /// Visible screen as plain text, trailing blanks trimmed. Handy for tests.
    pub fn screen_text(&self) -> String {
        use alacritty_terminal::index::{Column, Line};
        let grid = self.term.grid();
        let mut out = String::new();
        for l in 0..grid.screen_lines() {
            let row = &grid[Line(l as i32)];
            let line: String = (0..grid.columns()).map(|c| row[Column(c)].c).collect();
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_text_and_answers_queries() {
        let mut t = Terminal::new(40, 5);
        let reply = t.advance(b"hello\r\n\x1b[1;31mred\x1b[0m\x1b[6n");
        let text = t.screen_text();
        assert!(text.starts_with("hello\nred\n"), "{text}");
        // CPR reply: ESC [ row ; col R
        assert_eq!(String::from_utf8_lossy(&reply), "\x1b[2;4R");
    }
}
