//! Tests for `platform::windows`.

use gpui::Modifiers;

use super::terminal_paste;

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
    assert!(terminal_paste(mods(true, false, false), "v"));
    assert!(terminal_paste(mods(false, true, false), "insert"));
    // Ctrl+Shift+V goes through the terminal chord; AltGr (Ctrl+Alt) types a character.
    assert!(!terminal_paste(mods(true, true, false), "v"));
    assert!(!terminal_paste(mods(true, false, true), "v"));
    // Plain Insert and other Ctrl keys still reach the shell.
    assert!(!terminal_paste(Modifiers::default(), "insert"));
    assert!(!terminal_paste(mods(true, false, false), "c"));
}
