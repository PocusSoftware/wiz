//! Tests for `terminal`.

use gpui::Modifiers;

use super::keys::key_bytes;

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
