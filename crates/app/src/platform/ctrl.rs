//! Shortcuts and window handling shared by Windows and Linux, where plain Ctrl belongs to
//! the shell, so app shortcuts add Shift like Windows Terminal, GNOME Terminal and Konsole.

use std::collections::HashSet;

use gpui::{App, Entity, KeyBinding, Modifiers, TextSystem, Window};

use crate::workspace::{NextTab, PrevTab, ResetZoom, SelectTab, Workspace, ZoomIn, ZoomOut};

/// Modifiers a shortcut must hold, for settings text.
pub const MODIFIER_HINT: &str = "Ctrl or Alt";

pub const ZOOM_KEYS: &str = "Ctrl+= and Ctrl+- change it, Ctrl+0 resets it.";

/// Ctrl+1 to Ctrl+9, which pick a tab. Literals because gpui only builds bindings from `&'static str`.
const DIGITS: [&str; 9] = [
    "ctrl-1", "ctrl-2", "ctrl-3", "ctrl-4", "ctrl-5", "ctrl-6", "ctrl-7", "ctrl-8", "ctrl-9",
];

/// Plain Ctrl+D, Ctrl+W and Ctrl+K mean something to the shell, so app shortcuts add Shift.
/// Ctrl+Tab and Ctrl+1 to Ctrl+9 switch tabs, so Ctrl+2 (NUL) and Ctrl+6 no longer reach
/// the shell. Alt+F4 quits through the window's close request.
pub fn fixed_bindings() -> Vec<KeyBinding> {
    let mut keys = vec![
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("ctrl-=", ZoomIn, None),
        KeyBinding::new("ctrl-+", ZoomIn, None),
        KeyBinding::new("ctrl--", ZoomOut, None),
        KeyBinding::new("ctrl-0", ResetZoom, None),
    ];
    keys.extend(
        DIGITS
            .into_iter()
            .enumerate()
            .map(|(n, k)| KeyBinding::new(k, SelectTab(n), None)),
    );
    keys
}

/// Whether `keys` is one of `fixed_bindings`.
pub fn is_fixed(keys: &str) -> bool {
    matches!(
        keys,
        "ctrl-tab" | "ctrl-shift-tab" | "ctrl-=" | "ctrl-+" | "ctrl--" | "ctrl-0"
    ) || DIGITS.contains(&keys)
}

/// The keystrokes a command starts with (ids are in `shortcuts::COMMANDS`). Tabs switch
/// with the fixed Ctrl+Tab, so they have none of their own.
pub fn default_shortcut(id: &str) -> Option<&'static str> {
    Some(match id {
        "split" => "ctrl-shift-d",
        "split_menu" => "ctrl-shift-s",
        "tab_menu" => "ctrl-shift-m",
        // Like Windows Terminal.
        "next_pane" => "alt-right",
        "prev_pane" => "alt-left",
        "focus_tabs" => "ctrl-shift-l",
        "quick_connect" => "ctrl-shift-k",
        "snippets" => "ctrl-shift-j",
        "new_host" => "ctrl-shift-n",
        "settings" => "ctrl-,",
        "close_tab" => "ctrl-shift-w",
        _ => return None,
    })
}

/// Keystrokes written like "Ctrl+Shift+D".
pub fn format_keys(keys: &str) -> String {
    let Ok(k) = gpui::Keystroke::parse_with_os(keys, super::OS) else {
        return keys.to_string();
    };
    let m = k.modifiers;
    let mut out = String::new();
    for (held, name) in [
        (m.control, "Ctrl+"),
        (m.alt, "Alt+"),
        (m.shift, "Shift+"),
        (m.platform, super::LOGO_KEY),
    ] {
        if held {
            out.push_str(name);
        }
    }
    let mut chars = k.key.chars();
    out.extend(chars.next().map(|c| c.to_ascii_uppercase()));
    out.push_str(chars.as_str());
    out
}

/// Label of a shortcut inside a view (files, logs), like "Ctrl+R" or "Ctrl+Shift+N".
pub fn view_shortcut(shift: bool, key: &str) -> String {
    format!("Ctrl+{}{key}", if shift { "Shift+" } else { "" })
}

/// Terminal shortcuts (copy, paste, find, ...) are Ctrl+Shift+key, so plain Ctrl+key still
/// reaches the shell (except `terminal_paste` and tab switching). `Some(alt)` when the chord
/// is held, Alt picking the variant (previous match, type password).
pub fn terminal_chord(m: Modifiers) -> Option<bool> {
    (m.control && m.shift).then_some(m.alt)
}

/// Ctrl-click opens a link in the terminal.
pub fn link_click(m: Modifiers) -> bool {
    m.control
}

/// Whether a font family is installed or bundled. DirectWrite and cosmic-text give an
/// unknown family a fallback font, so comparing resolved fonts can't tell; compare the
/// names instead (case-insensitive).
pub fn font_exists(ts: &TextSystem) -> impl Fn(&str) -> bool {
    let names: HashSet<String> = ts
        .all_font_names()
        .into_iter()
        .map(|n| n.to_lowercase())
        .collect();
    move |name| names.contains(&name.to_lowercase())
}

/// The system (Alt+F4, the taskbar, the compositor) closes the window directly; route that
/// through the same check as Quit, so open sessions can block it.
pub fn on_window_open(window: &mut Window, workspace: &Entity<Workspace>, cx: &mut App) {
    let workspace = workspace.downgrade();
    window.on_window_should_close(cx, move |window, cx| {
        workspace
            .update(cx, |w, cx| w.request_quit(window, cx))
            .ok();
        false
    });
}
