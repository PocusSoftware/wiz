//! Tests for `terminal`.

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
