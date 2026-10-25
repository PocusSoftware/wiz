//! Shortcut lookup: defaults, the user's changes and clashes.

use ssh_core::config::Settings;

use super::*;
use crate::testing::GLOBALS;

fn with_settings(s: Settings) -> std::sync::MutexGuard<'static, ()> {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::prefs::init(s);
    guard
}

#[test]
fn every_command_has_a_binding_and_a_default() {
    let _g = with_settings(Settings::default());
    for (id, _) in COMMANDS {
        let keys = platform::default_shortcut(id);
        if let Some(keys) = keys {
            assert!(is_valid(keys), "{id}: {keys}");
            assert!(binding(id, keys).is_some(), "{id} has no action");
        }
    }
}

#[test]
fn saved_shortcut_wins_over_the_default() {
    let _g = with_settings(Settings {
        shortcuts: Some([("split".to_string(), "ctrl-alt-j".to_string())].into()),
        ..Default::default()
    });
    assert_eq!(keys("split").as_deref(), Some("ctrl-alt-j"));
    assert_eq!(
        keys("close_tab").as_deref(),
        platform::default_shortcut("close_tab")
    );
}

#[test]
fn clashes_are_found_and_the_command_itself_is_not_one() {
    let _g = with_settings(Settings::default());
    let split = platform::default_shortcut("split").unwrap();
    assert_eq!(taken_by(split, "split"), None);
    assert!(taken_by(split, "close_tab").is_some());
    assert!(taken_by("ctrl-tab", "split").is_some());
}

#[test]
fn only_single_normalized_keystrokes_are_valid() {
    assert!(is_valid("cmd-shift-d"));
    assert!(!is_valid("cmd-k cmd-d"));
    assert!(!is_valid(""));
    assert!(!is_valid("shift-cmd-d"));
}
