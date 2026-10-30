//! Tests for `logs`.

use super::*;

#[test]
fn strips_colors_cr_and_tabs() {
    assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m plain"), "red plain");
    assert_eq!(strip_ansi("a\tb\r"), "a b");
    assert_eq!(strip_ansi("\u{1b}[2K\u{1b}[1;32mok"), "ok");
}

#[test]
fn compacts_docker_timestamps() {
    assert_eq!(
        compact_timestamp("2026-10-30T10:55:40.123456789Z hello world"),
        "[10:55:40] hello world"
    );
    assert_eq!(timestamp_len("[10:55:40] hello"), TS_LEN);
    assert_eq!(timestamp_len("hello"), 0);
    assert_eq!(compact_timestamp("no stamp here"), "no stamp here");
    assert_eq!(compact_timestamp("x"), "x");
}
