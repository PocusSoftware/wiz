//! App-wide shortcuts: the commands that can be rebound, the platform defaults and the
//! user's changes from `settings.toml`.

#[cfg(test)]
mod tests;

use gpui::{App, KeyBinding, Keystroke};

use crate::platform;
use crate::workspace::{
    CloseTab, FocusTabs, NewHost, NextPane, NextTab, OpenSettings, PrevPane, PrevTab, QuickConnect,
    SplitMenu, SplitPane, TabMenu,
};

/// Commands the user can rebind: id (the key in `settings.toml`) and the name shown.
pub const COMMANDS: &[(&str, &str)] = &[
    ("split", "Split with the same server"),
    ("split_menu", "Split with another server"),
    ("tab_menu", "Tab menu (pin, rename, merge)"),
    ("next_pane", "Next pane"),
    ("prev_pane", "Previous pane"),
    ("next_tab", "Next tab"),
    ("prev_tab", "Previous tab"),
    ("focus_tabs", "Focus the tab bar"),
    ("quick_connect", "Quick connect"),
    ("new_host", "New host"),
    ("settings", "Settings"),
    ("close_tab", "Close tab or pane"),
];

/// The keystrokes for command `id`: the user's choice, else the platform default, `None`
/// when it has neither.
pub fn keys(id: &str) -> Option<String> {
    crate::prefs::get()
        .shortcuts()
        .get(id)
        .cloned()
        .or_else(|| platform::default_shortcut(id).map(str::to_string))
}

/// How command `id` is written for the user, like "⇧⌘D", or "none".
pub fn label(id: &str) -> String {
    keys(id).map_or_else(|| "none".into(), |k| platform::format_keys(&k))
}

/// Whether `keys` is a single keystroke that gpui can bind.
pub fn is_valid(keys: &str) -> bool {
    Keystroke::parse(keys).is_ok_and(|k| !k.key.is_empty() && k.unparse() == keys)
}

/// What already uses `keys` (a command, apart from `except`, or a built-in shortcut).
pub fn taken_by(keys: &str, except: &str) -> Option<&'static str> {
    if platform::is_fixed(keys) {
        return Some("a built-in shortcut");
    }
    COMMANDS
        .iter()
        .find(|(id, _)| *id != except && self::keys(id).as_deref() == Some(keys))
        .map(|(_, name)| *name)
}

fn binding(id: &str, keys: &str) -> Option<KeyBinding> {
    Some(match id {
        "split" => KeyBinding::new(keys, SplitPane, None),
        "split_menu" => KeyBinding::new(keys, SplitMenu, None),
        "tab_menu" => KeyBinding::new(keys, TabMenu, None),
        "next_pane" => KeyBinding::new(keys, NextPane, None),
        "prev_pane" => KeyBinding::new(keys, PrevPane, None),
        "next_tab" => KeyBinding::new(keys, NextTab, None),
        "prev_tab" => KeyBinding::new(keys, PrevTab, None),
        "focus_tabs" => KeyBinding::new(keys, FocusTabs, None),
        "quick_connect" => KeyBinding::new(keys, QuickConnect, None),
        "new_host" => KeyBinding::new(keys, NewHost, None),
        "settings" => KeyBinding::new(keys, OpenSettings, None),
        "close_tab" => KeyBinding::new(keys, CloseTab, None),
        _ => return None,
    })
}

/// Bind the built-in shortcuts and the commands, replacing whatever was bound before.
/// A saved keystroke that does not parse falls back to the default.
pub fn apply(cx: &mut App) {
    cx.clear_key_bindings();
    cx.bind_keys(platform::fixed_bindings());
    let bindings = COMMANDS.iter().filter_map(|(id, _)| {
        let saved = keys(id).filter(|k| is_valid(k));
        let chosen = saved.or_else(|| platform::default_shortcut(id).map(str::to_string));
        binding(id, &chosen?)
    });
    cx.bind_keys(bindings);
}
