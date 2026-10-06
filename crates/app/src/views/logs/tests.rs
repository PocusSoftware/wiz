//! Tests for `logs`.

use super::*;

#[test]
fn strips_colors_cr_and_tabs() {
    assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m plain"), "red plain");
    assert_eq!(strip_ansi("a\tb\r"), "a b");
    assert_eq!(strip_ansi("\u{1b}[2K\u{1b}[1;32mok"), "ok");
}
