//! Tests for `terminal`.

use gpui::{Modifiers, px};

use super::keys::key_bytes;
use super::mouse::autoscroll_lines;

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Default::default()
    }
}

#[test]
fn option_composed_chars_have_no_esc_prefix() {
    // German layout: Option+7 is "|", Option+L is "@".
    assert_eq!(key_bytes("7", Some("|"), alt(), false).unwrap(), b"|");
    assert_eq!(key_bytes("l", Some("@"), alt(), false).unwrap(), b"@");
    // Plain Option+key where the OS gives the same char becomes Meta (ESC prefix).
    assert_eq!(key_bytes("b", Some("b"), alt(), false).unwrap(), b"\x1bb");
}

#[test]
fn basic_keys() {
    let none = Modifiers::default();
    assert_eq!(key_bytes("a", Some("a"), none, false).unwrap(), b"a");
    assert_eq!(key_bytes("enter", None, none, false).unwrap(), b"\r");
    assert_eq!(key_bytes("up", None, none, false).unwrap(), b"\x1b[A");
    assert_eq!(key_bytes("up", None, none, true).unwrap(), b"\x1bOA");
    let ctrl = Modifiers {
        control: true,
        ..Default::default()
    };
    assert_eq!(key_bytes("c", Some("c"), ctrl, false).unwrap(), b"\x03");
}

#[test]
fn space_bar() {
    // Windows reports the space bar with no text; macOS sends " ".
    for key_char in [None, Some(" ")] {
        assert_eq!(
            key_bytes("space", key_char, Modifiers::default(), false).unwrap(),
            b" "
        );
    }
    let ctrl = Modifiers {
        control: true,
        ..Default::default()
    };
    assert_eq!(key_bytes("space", None, ctrl, false).unwrap(), b"\0");
    assert_eq!(key_bytes("space", None, alt(), false).unwrap(), b"\x1b ");
}

#[test]
fn autoscroll_follows_the_mouse_past_the_edges() {
    let (top, height, cell) = (px(100.), px(400.), px(20.));
    assert_eq!(autoscroll_lines(px(300.), top, height, cell), 0);
    assert_eq!(autoscroll_lines(px(90.), top, height, cell), 1);
    assert_eq!(autoscroll_lines(px(510.), top, height, cell), -1);
    assert_eq!(autoscroll_lines(px(-500.), top, height, cell), 5);
    assert_eq!(autoscroll_lines(px(900.), top, height, cell), -5);
}
