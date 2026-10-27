//! macOS: Cmd shortcuts, the Dock icon, AppKit window dragging, `open` for files.

use std::path::Path;

use gpui::{
    App, Div, Entity, InteractiveElement, KeyBinding, Modifiers, MouseButton, MouseDownEvent,
    Stateful, StatefulInteractiveElement, TextSystem, TitlebarOptions, Window, WindowControlArea,
    point, px,
};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSView};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::workspace::{NextTab, PrevTab, Quit, SelectTab, Workspace};

/// Large transfers between this Mac and a server can run through rsync.
pub const RSYNC: bool = true;

/// Modifiers a shortcut must hold, for settings text.
pub const MODIFIER_HINT: &str = "Cmd, Ctrl or Option";

/// How quitting is done, for settings text.
pub const QUIT_KEYS: &str = "⌘Q";

pub const EDITOR_HINT: &str = "App that opens files with Enter. Pick one below or type any app name. Empty uses the default text editor.";

/// Shortcuts that cannot be rebound: quit, the tab bar's Ctrl+Tab and Cmd+1 to Cmd+9.
pub fn fixed_bindings() -> Vec<KeyBinding> {
    let mut keys = vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
    ];
    keys.extend((1..=9).map(|n| KeyBinding::new(&format!("cmd-{n}"), SelectTab(n - 1), None)));
    keys
}

/// Whether `keys` is one of `fixed_bindings`.
pub fn is_fixed(keys: &str) -> bool {
    matches!(keys, "cmd-q" | "ctrl-tab" | "ctrl-shift-tab")
        || keys
            .strip_prefix("cmd-")
            .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
}

/// The keystrokes a command starts with (ids are in `shortcuts::COMMANDS`).
pub fn default_shortcut(id: &str) -> Option<&'static str> {
    Some(match id {
        "split" => "cmd-d",
        "split_menu" => "cmd-shift-d",
        "tab_menu" => "cmd-shift-m",
        "next_pane" => "cmd-]",
        "prev_pane" => "cmd-[",
        "next_tab" => "cmd-shift-]",
        "prev_tab" => "cmd-shift-[",
        "focus_tabs" => "cmd-l",
        "quick_connect" => "cmd-k",
        "new_host" => "cmd-n",
        "settings" => "cmd-,",
        "close_tab" => "cmd-w",
        _ => return None,
    })
}

/// Keystrokes written the Mac way: "cmd-shift-d" is "⇧⌘D".
pub fn format_keys(keys: &str) -> String {
    let Ok(k) = gpui::Keystroke::parse(keys) else {
        return keys.to_string();
    };
    let m = k.modifiers;
    let mut out = String::new();
    for (held, mark) in [
        (m.control, '⌃'),
        (m.alt, '⌥'),
        (m.shift, '⇧'),
        (m.platform, '⌘'),
    ] {
        if held {
            out.push(mark);
        }
    }
    out.push_str(&k.key.to_uppercase());
    out
}

/// Label of a shortcut inside a view (files, logs), like "⌘R" or "⇧⌘N".
pub fn view_shortcut(shift: bool, key: &str) -> String {
    format!("{}⌘{key}", if shift { "⇧" } else { "" })
}

/// Terminal shortcuts (copy, paste, find, ...) use Cmd, which never reaches the shell.
/// `Some(shift)` when the chord is held, `shift` picking the variant (previous match,
/// type password).
pub fn terminal_chord(m: Modifiers) -> Option<bool> {
    m.platform.then_some(m.shift)
}

/// Pasting is Cmd+V (`terminal_chord`); Ctrl+V and Shift+Insert go to the shell.
pub fn terminal_paste(_: Modifiers, _: &str) -> bool {
    false
}

/// Whether a key press with these modifiers types its character into a text field (no Cmd
/// or Ctrl shortcut).
pub fn types_char(m: Modifiers) -> bool {
    !m.platform && !m.control
}

/// macOS has no AltGr: Option composes characters and is handled as Alt.
pub fn alt_gr(_: Modifiers, _: &str, _: Option<&str>) -> bool {
    false
}

/// Cmd-click opens a link in the terminal.
pub fn link_click(m: Modifiers) -> bool {
    m.platform
}

/// Whether a font family is installed or bundled. An unknown family resolves to the fallback
/// font, so a candidate exists exactly when it resolves to something else.
pub fn font_exists(ts: &TextSystem) -> impl Fn(&str) -> bool {
    let fallback = ts.resolve_font(&gpui::font("\u{1}no-such-font"));
    move |name| ts.resolve_font(&gpui::font(name.to_string())) != fallback
}

/// Show the hat in the Dock even when running unbundled (cargo run).
pub fn on_launch() {
    use objc2::AnyThread;
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../../assets/brand/icon-512.png"));
    if let Some(img) = NSImage::initWithData(NSImage::alloc(), &data) {
        // SAFETY: called on the main thread with a valid image.
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&img)) };
    }
}

pub fn titlebar() -> TitlebarOptions {
    TitlebarOptions {
        title: Some("Wiz".into()),
        appears_transparent: true,
        // Native buttons are parked off-screen; the title bar draws its own.
        traffic_light_position: Some(point(px(-100.), px(-100.))),
    }
}

/// Closing the window already goes through the title bar's own button and Quit.
pub fn on_window_open(_: &mut Window, _: &Entity<Workspace>, _: &mut App) {}

/// Make an element a title bar: drag moves the window, double-click zooms it. GPUI's
/// macOS backend has no hook for this, so hand the mouse-down event to AppKit.
pub fn title_bar<E: InteractiveElement>(el: E) -> E {
    el.on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, _| {
        if ev.click_count >= 2 {
            window.titlebar_double_click();
        } else {
            drag(window);
        }
    })
}

/// The title bar buttons work through their click handlers.
pub fn window_button(
    el: Stateful<Div>,
    _: WindowControlArea,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    el.on_click(move |_, w, cx| action(w, cx))
}

fn drag(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: the handle comes from the live window, and this runs on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    let Some(ns_window) = view.window() else {
        return;
    };
    if let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() {
        ns_window.performWindowDragWithEvent(&event);
    }
}

/// Editors offered as quick picks when installed.
const KNOWN_EDITORS: &[&str] = &[
    "Visual Studio Code",
    "Cursor",
    "Zed",
    "Sublime Text",
    "BBEdit",
    "Nova",
    "TextMate",
    "CotEditor",
    "Windsurf",
    "VSCodium",
    "Xcode",
    "TextEdit",
];

/// Known editors found in the Applications folders.
pub fn installed_editors() -> Vec<&'static str> {
    let user = std::env::home_dir().map(|h| h.join("Applications"));
    KNOWN_EDITORS
        .iter()
        .copied()
        .filter(|n| {
            let app = format!("{n}.app");
            Path::new("/Applications").join(&app).exists()
                || user.as_ref().is_some_and(|u| u.join(&app).exists())
        })
        .collect()
}

/// Open a local file in `app`, or without one in the default text editor (`-t`), so a
/// server file named like an app or script is never launched.
pub fn open_in_editor(path: &Path, app: &str) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("open");
    if app.is_empty() {
        cmd.arg("-t");
    } else {
        cmd.arg("-a").arg(app);
    }
    cmd.arg(path).spawn().map(drop)
}

/// The logged-in user, masked by "hide sensitive information".
pub fn local_user() -> Option<String> {
    std::env::var("USER").ok()
}
