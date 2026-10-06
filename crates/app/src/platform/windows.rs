//! Windows: Ctrl+Shift shortcuts (plain Ctrl belongs to the shell), native title bar
//! areas, Notepad or a named program for files. No Keychain, rsync or editor picks.

use std::path::Path;

use gpui::{
    App, Div, Entity, InteractiveElement, KeyBinding, Modifiers, Stateful, TitlebarOptions, Window,
    WindowControlArea,
};

use crate::workspace::{
    CloseTab, NewHost, NextTab, OpenSettings, PrevTab, QuickConnect, SplitPane, Workspace,
};

/// Fallback fonts when none of the preferred ones is installed.
pub const UI_FONT: &str = "Segoe UI";
pub const MONO_FONT: &str = "Consolas";

/// rsync transfers need the macOS bridge; Windows uses SFTP only.
pub const RSYNC: bool = false;

/// How quitting is done, for settings text.
pub const QUIT_KEYS: &str = "Alt+F4";

pub const EDITOR_HINT: &str = "Program that opens files with Enter: the full path to its .exe (C:\\Program Files\\Notepad++\\notepad++.exe). Empty uses Notepad.";

/// Plain Ctrl+D, Ctrl+W and Ctrl+K mean something to the shell, so app shortcuts add Shift,
/// like Windows Terminal. Alt+F4 quits through the window's close request.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("ctrl-shift-w", CloseTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("ctrl-shift-n", NewHost, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-shift-d", SplitPane, None),
        KeyBinding::new("ctrl-shift-k", QuickConnect, None),
    ]
}

/// Label of an app-wide shortcut (`key_bindings`), like "Ctrl+Shift+D".
pub fn app_shortcut(key: &str) -> String {
    if key == "," {
        "Ctrl+,".into()
    } else {
        format!("Ctrl+Shift+{key}")
    }
}

/// Label of a shortcut inside a view (files, logs), like "Ctrl+R" or "Ctrl+Shift+N".
pub fn view_shortcut(shift: bool, key: &str) -> String {
    format!("Ctrl+{}{key}", if shift { "Shift+" } else { "" })
}

/// Terminal shortcuts (copy, paste, find, ...) are Ctrl+Shift+key, so plain Ctrl+key still
/// reaches the shell. `Some(alt)` when the chord is held, Alt picking the variant
/// (previous match, type password).
pub fn terminal_chord(m: Modifiers) -> Option<bool> {
    (m.control && m.shift).then_some(m.alt)
}

/// Whether a key press with these modifiers types its character into a text field. AltGr
/// arrives as Ctrl+Alt and types characters (@ { } \ on German layouts).
pub fn types_char(m: Modifiers) -> bool {
    !m.platform && (!m.control || m.alt)
}

/// AltGr (Ctrl+Alt) that produced a different character than the key: send the character,
/// not a Ctrl sequence.
pub fn alt_gr(m: Modifiers, key: &str, key_char: Option<&str>) -> bool {
    m.control && m.alt && key_char.is_some_and(|c| c != key)
}

/// Ctrl-click opens a link in the terminal.
pub fn link_click(m: Modifiers) -> bool {
    m.control
}

pub fn on_launch() {}

pub fn titlebar() -> TitlebarOptions {
    TitlebarOptions {
        title: Some("Wiz".into()),
        appears_transparent: true,
        traffic_light_position: None,
    }
}

/// Alt+F4 and the taskbar close the window directly; route them through the same check as
/// Quit, so open sessions can block it.
pub fn on_window_open(window: &mut Window, workspace: &Entity<Workspace>, cx: &mut App) {
    let workspace = workspace.downgrade();
    window.on_window_should_close(cx, move |window, cx| {
        workspace
            .update(cx, |w, cx| w.request_quit(window, cx))
            .ok();
        false
    });
}

/// Windows moves, snaps and maximizes the window itself when told where the title bar is.
pub fn title_bar<E: InteractiveElement>(el: E) -> E {
    el.window_control_area(WindowControlArea::Drag)
}

/// The title bar buttons are native areas, so snap layouts and hover work like elsewhere.
/// Windows runs the action itself (the close goes through `on_window_open`'s check).
pub fn window_button(
    el: Stateful<Div>,
    area: WindowControlArea,
    _: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    el.window_control_area(area)
}

/// Open a local file in `app`, or in Notepad without one. Never the file association: a
/// server file named like a program or script would run.
pub fn open_in_editor(path: &Path, app: &str) -> std::io::Result<()> {
    let program = if app.is_empty() { "notepad.exe" } else { app };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(drop)
}

/// No quick picks on Windows; the setting takes a program.
pub const EDITOR_PICKS: bool = false;

/// No list of installed editors on Windows; the setting takes a program name.
pub fn installed_editors() -> Vec<&'static str> {
    Vec::new()
}

/// The logged-in user, masked by "hide sensitive information".
pub fn local_user() -> Option<String> {
    std::env::var("USERNAME").ok()
}
