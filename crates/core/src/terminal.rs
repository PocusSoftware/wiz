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

/// A hit from [`Terminal::find`]: `line` is a grid line (negative is scrollback), `col` the
/// first column and `len` the width in columns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    pub line: i32,
    pub col: usize,
    pub len: usize,
}

/// Most hits kept, so a one-letter query on a huge scrollback stays cheap.
const MAX_MATCHES: usize = 10_000;

pub struct Terminal {
    pub term: Term<Listener>,
    parser: Processor,
    listener: Listener,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::with_scrollback(cols, rows, 10_000)
    }

    pub fn with_scrollback(cols: u16, rows: u16, scrollback: usize) -> Self {
        let listener = Listener::default();
        let size = Size {
            cols: cols as usize,
            rows: rows as usize,
        };
        Self {
            term: Term::new(
                Config {
                    scrolling_history: scrollback,
                    ..Config::default()
                },
                &size,
                listener.clone(),
            ),
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

    /// All occurrences of `query` in scrollback and screen, oldest first. Case-insensitive
    /// unless the query has an uppercase letter. Matches never span a line break.
    pub fn find(&self, query: &str) -> Vec<Match> {
        use alacritty_terminal::index::{Column, Line};
        let fold = !query.chars().any(char::is_uppercase);
        let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
        let q: Vec<char> = query
            .chars()
            .map(|c| if fold { lower(c) } else { c })
            .collect();
        let mut out = Vec::new();
        if q.is_empty() {
            return out;
        }
        let grid = self.term.grid();
        for l in grid.topmost_line().0..=grid.bottommost_line().0 {
            let row = &grid[Line(l)];
            let cells: Vec<char> = (0..grid.columns())
                .map(|c| {
                    let ch = row[Column(c)].c;
                    if fold { lower(ch) } else { ch }
                })
                .collect();
            if cells.len() < q.len() {
                continue;
            }
            for col in 0..=cells.len() - q.len() {
                if cells[col..col + q.len()] == q[..] {
                    out.push(Match {
                        line: l,
                        col,
                        len: q.len(),
                    });
                    if out.len() >= MAX_MATCHES {
                        return out;
                    }
                }
            }
        }
        out
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

    #[test]
    fn finds_in_scrollback_and_screen() {
        let mut t = Terminal::new(20, 3);
        t.advance(b"one Error\r\ntwo\r\nthree\r\nfour error\r\nfive");
        // Lowercase query ignores case; "one Error" has scrolled into history.
        let m = t.find("error");
        assert_eq!(m.len(), 2);
        // 5 lines into 3 rows: "one Error" is in history, "four error" on screen.
        assert_eq!((m[0].line, m[1].line), (-2, 1), "{m:?}");
        assert_eq!((m[0].col, m[0].len), (4, 5));
        // An uppercase letter makes it case-sensitive.
        assert_eq!(t.find("Error").len(), 1);
        assert!(t.find("").is_empty());
    }
}
