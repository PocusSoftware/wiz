//! Tests for `platform::linux`.

use gpui::Modifiers;

use super::{alt_gr, terminal_paste, types_char};

fn mods(control: bool, shift: bool, alt: bool) -> Modifiers {
    Modifiers {
        control,
        shift,
        alt,
        ..Default::default()
    }
}

#[test]
fn paste_keys() {
    assert!(terminal_paste(mods(false, true, false), "insert"));
    assert!(!terminal_paste(mods(true, false, false), "v"));
    assert!(!terminal_paste(Modifiers::default(), "insert"));
}

#[test]
fn ctrl_alt_is_a_shortcut() {
    assert!(types_char(mods(false, true, true)));
    assert!(!types_char(mods(true, false, true)));
    assert!(!alt_gr(mods(true, false, true), "q", Some("@")));
}
