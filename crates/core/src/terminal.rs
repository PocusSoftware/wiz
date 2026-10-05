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

/// A web link found in the terminal text.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub url: String,
    /// Grid (line, column) of every cell the link covers, across wrapped rows.
    pub cells: Vec<(i32, usize)>,
}

/// Put the emulator back in its normal modes after the program it ran vanished (leaves the
/// alternate screen, resets scroll region, cursor keys, keypad, origin mode, bracketed paste,
/// charset). Unlike a full reset (`ESC c`) it keeps the scrollback.
pub const RESET_MODES: &[u8] =
    b"\x1b[?1049l\x1b[?25h\x1b[0m\x1b[r\x1b[?1l\x1b>\x1b[?6l\x1b[?7h\x1b[?2004l\x1b(B";

/// Clear the screen and the scrollback and home the cursor.
pub const CLEAR_ALL: &[u8] = b"\x1b[2J\x1b[3J\x1b[H";

/// Remove the "System is booting up ... pam_nologin(8)." notice (and its line break) that a
/// server prints while it starts. `None` when `data` has no complete notice.
pub fn strip_boot_notice(data: &[u8]) -> Option<Vec<u8>> {
    const START: &[u8] = b"System is booting up";
    const END: &[u8] = b"pam_nologin(8).";
    let find = |from: usize, pat: &[u8]| {
        data[from..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|i| i + from)
    };
    let start = find(0, START)?;
    let mut end = find(start, END)? + END.len();
    if data[end..].starts_with(b"\r\n") {
        end += 2;
    } else if data[end..].starts_with(b"\n") {
        end += 1;
    }
    let mut out = data[..start].to_vec();
    out.extend_from_slice(&data[end..]);
    Some(out)
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

    /// The http(s) link under grid cell (`line`, `col`), if any. Follows lines the terminal
    /// wrapped, and drops trailing punctuation like the `.` or `)` after a link in prose.
    pub fn link_at(&self, line: i32, col: usize) -> Option<Link> {
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::cell::Flags;
        let grid = self.term.grid();
        let last = Column(grid.columns().checked_sub(1)?);
        if line < grid.topmost_line().0 || line > grid.bottommost_line().0 {
            return None;
        }
        let wraps = |l: i32| grid[Line(l)][last].flags.contains(Flags::WRAPLINE);
        let (mut a, mut b) = (line, line);
        while a > grid.topmost_line().0 && wraps(a - 1) {
            a -= 1;
        }
        while b < grid.bottommost_line().0 && wraps(b) {
            b += 1;
        }
        let mut chars: Vec<(char, i32, usize)> = Vec::new();
        for l in a..=b {
            for c in 0..grid.columns() {
                let cell = &grid[Line(l)][Column(c)];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    chars.push((cell.c, l, c));
                }
            }
        }
        let click = chars.iter().position(|&(_, l, c)| l == line && c >= col)?;
        let stop = |ch: char| ch.is_whitespace() || ch == '\0' || "<>\"'`".contains(ch);
        if stop(chars[click].0) {
            return None;
        }
        let mut lo = click;
        while lo > 0 && !stop(chars[lo - 1].0) {
            lo -= 1;
        }
        let mut hi = click + 1;
        while hi < chars.len() && !stop(chars[hi].0) {
            hi += 1;
        }
        let tok: String = chars[lo..hi].iter().map(|x| x.0).collect();
        // Start at the last scheme at or before the click.
        let click_in = click - lo;
        let start = ["https://", "http://"]
            .iter()
            .flat_map(|sch| tok.match_indices(sch).map(|(i, _)| i))
            .filter(|&i| tok[..i].chars().count() <= click_in)
            .max()?;
        let mut url: Vec<char> = tok[start..].chars().collect();
        let first = tok[..start].chars().count();
        while let Some(&ch) = url.last() {
            let count = |x: char| url.iter().filter(|&&y| y == x).count();
            let unbalanced = |open: char| count(ch) > count(open);
            if ".,;:!?".contains(ch)
                || (ch == ')' && unbalanced('('))
                || (ch == ']' && unbalanced('['))
                || (ch == '}' && unbalanced('{'))
            {
                url.pop();
            } else {
                break;
            }
        }
        let scheme = if tok[start..].starts_with("https") {
            8
        } else {
            7
        };
        if url.len() <= scheme || click_in >= first + url.len() {
            return None;
        }
        Some(Link {
            url: url.iter().collect(),
            cells: chars[lo + first..lo + first + url.len()]
                .iter()
                .map(|&(_, l, c)| (l, c))
                .collect(),
        })
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
    fn reset_modes_clears_app_modes_and_clear_all_drops_history() {
        use alacritty_terminal::term::TermMode;
        let mut t = Terminal::new(20, 3);
        t.advance(b"a\r\nb\r\nc\r\nd\r\n\x1b[?1h\x1b[?2004h\x1b[?1049h");
        assert!(t.term.mode().contains(TermMode::APP_CURSOR));
        assert!(t.term.mode().contains(TermMode::BRACKETED_PASTE));
        t.advance(RESET_MODES);
        let m = t.term.mode();
        assert!(!m.contains(TermMode::APP_CURSOR));
        assert!(!m.contains(TermMode::BRACKETED_PASTE));
        assert!(!m.contains(TermMode::ALT_SCREEN));
        // Primary screen scrollback is untouched.
        assert!(t.term.grid().history_size() > 0);
        t.advance(CLEAR_ALL);
        assert_eq!(t.term.grid().history_size(), 0);
        assert!(t.screen_text().trim().is_empty());
    }

    #[test]
    fn strips_boot_notice_only_when_complete() {
        let msg = "System is booting up. Unprivileged users are not permitted to log in yet. \
                   Please come back later. For technical details, see pam_nologin(8).";
        let data = format!("{msg}\r\nWelcome to Ubuntu\r\n");
        let out = strip_boot_notice(data.as_bytes()).unwrap();
        assert_eq!(out, b"Welcome to Ubuntu\r\n");
        assert!(strip_boot_notice(b"Welcome to Ubuntu\r\n").is_none());
        assert!(strip_boot_notice(b"System is booting up. Unpriv").is_none());
    }

    #[test]
    fn finds_links_with_trailing_punctuation_and_wraps() {
        let mut t = Terminal::new(20, 5);
        t.advance(b"see https://a.io/x_(y). ok\r\nhttps://example.com/very/long/path, end");
        let l = t.link_at(0, 8).unwrap();
        assert_eq!(l.url, "https://a.io/x_(y)");
        assert_eq!(l.cells.len(), 18);
        assert!(t.link_at(0, 1).is_none());
        assert!(t.link_at(1, 4).is_none());
        // Wrapped over two rows; the comma is dropped.
        let l = t.link_at(3, 3).unwrap();
        assert_eq!(l.url, "https://example.com/very/long/path");
        assert_eq!(l.cells[0], (2, 0));
        assert_eq!(*l.cells.last().unwrap(), (3, 13));
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
