//! App-wide shortcuts: the commands that can be rebound, the platform defaults and the
//! user's changes from `settings.toml`.

#[cfg(test)]
mod tests;

use gpui::{Action, App, KeyBinding, Keystroke};

use crate::platform;
use crate::workspace::{
    CloseTab, FocusTabs, NewHost, NextPane, NextTab, OpenSettings, PrevPane, PrevTab, QuickConnect,
    ShowHelp, Snippets, SplitMenu, SplitPane, TabMenu,
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
    ("snippets", "Snippets"),
    ("new_host", "New host"),
    ("settings", "Settings"),
    ("close_tab", "Close tab or pane"),
    ("help", "Keyboard shortcuts"),
];

/// Commands by section, in the order shown. Every id in `COMMANDS` appears once.
pub const GROUPS: &[(&str, &[&str])] = &[
    ("Splits", &["split", "split_menu", "next_pane", "prev_pane"]),
    (
        "Tabs",
        &[
            "tab_menu",
            "next_tab",
            "prev_tab",
            "focus_tabs",
            "close_tab",
        ],
    ),
    (
        "App",
        &["quick_connect", "snippets", "new_host", "settings", "help"],
    ),
];

/// A shortcut as one cap per key: "⇧⌘D" gives ⇧ ⌘ D, "Ctrl+Shift+D" gives Ctrl Shift D.
pub fn keycaps(label: &str) -> Vec<String> {
    if !label.contains('+') {
        let key = label.trim_start_matches(['⌃', '⌥', '⇧', '⌘']);
        let mut caps: Vec<String> = label[..label.len() - key.len()]
            .chars()
            .map(String::from)
            .collect();
        if !key.is_empty() {
            caps.push(key.to_string());
        }
        return caps;
    }
    let mut caps: Vec<String> = label
        .split('+')
        .filter(|k| !k.is_empty())
        .map(String::from)
        .collect();
    // "Ctrl++" is Ctrl and the plus key.
    if label.ends_with('+') {
        caps.push("+".into());
    }
    caps
}

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
    Keystroke::parse_with_os(keys, platform::OS)
        .is_ok_and(|k| !k.key.is_empty() && k.unparse(platform::OS) == keys)
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

fn binding(id: &str, keys: &str, cx: &App) -> Option<KeyBinding> {
    let action: Box<dyn Action> = match id {
        "split" => Box::new(SplitPane),
        "split_menu" => Box::new(SplitMenu),
        "tab_menu" => Box::new(TabMenu),
        "next_pane" => Box::new(NextPane),
        "prev_pane" => Box::new(PrevPane),
        "next_tab" => Box::new(NextTab),
        "prev_tab" => Box::new(PrevTab),
        "focus_tabs" => Box::new(FocusTabs),
        "quick_connect" => Box::new(QuickConnect),
        "snippets" => Box::new(Snippets),
        "new_host" => Box::new(NewHost),
        "settings" => Box::new(OpenSettings),
        "close_tab" => Box::new(CloseTab),
        "help" => Box::new(ShowHelp),
        _ => return None,
    };
    KeyBinding::load(keys, action, None, false, None, cx).ok()
}

/// The keystrokes each command is bound to. A saved keystroke that does not parse or is
/// already taken by a built-in or an earlier command (hand-edited settings) falls back to
/// the default, and a default that is taken too leaves the command unbound.
fn resolved() -> Vec<(&'static str, String)> {
    let mut used: Vec<String> = Vec::new();
    COMMANDS
        .iter()
        .filter_map(|&(id, _)| {
            let free = |k: &String| is_valid(k) && !platform::is_fixed(k) && !used.contains(k);
            let chosen = keys(id).filter(free).or_else(|| {
                platform::default_shortcut(id)
                    .map(str::to_string)
                    .filter(free)
            })?;
            used.push(chosen.clone());
            Some((id, chosen))
        })
        .collect()
}

/// Bind the built-in shortcuts and the commands, replacing whatever was bound before.
pub fn apply(cx: &mut App) {
    cx.clear_key_bindings();
    cx.bind_keys(platform::fixed_bindings());
    // Built before binding, since parsing them reads `cx`.
    let commands: Vec<KeyBinding> = resolved()
        .iter()
        .filter_map(|(id, keys)| binding(id, keys, cx))
        .collect();
    cx.bind_keys(commands);
}
