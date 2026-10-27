//! Windows: Ctrl+Shift shortcuts (plain Ctrl belongs to the shell, except paste and tab
//! switching), native title bar areas, installed editors, Notepad or a named program for
//! files. No Keychain or rsync.

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui::{
    App, Div, Entity, InteractiveElement, KeyBinding, Modifiers, MouseButton, MouseDownEvent,
    Stateful, StatefulInteractiveElement, TextSystem, TitlebarOptions, Window, WindowControlArea,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, HTCAPTION, IsZoomed, PostMessageW, SC_MOVE, SW_MAXIMIZE, SW_RESTORE,
    ShowWindowAsync, WM_SYSCOMMAND,
};

use crate::workspace::{NextTab, PrevTab, SelectTab, Workspace};

/// rsync transfers need the macOS bridge; Windows uses SFTP only.
pub const RSYNC: bool = false;

/// Modifiers a shortcut must hold, for settings text.
pub const MODIFIER_HINT: &str = "Ctrl or Alt";

/// How quitting is done, for settings text.
pub const QUIT_KEYS: &str = "Alt+F4";

/// Placeholder of the agent socket setting: what an empty one uses.
pub const AGENT_DEFAULT: &str = "Windows OpenSSH agent";
pub const AGENT_HINT: &str = "Pipe of the agent used by hosts that log in with the SSH agent. Empty uses the Windows OpenSSH agent; 1Password uses that pipe too when its agent is on.";
pub const EDITOR_HINT: &str = "Program that opens files with Enter. Pick one below or type the full path to any .exe. Empty uses Notepad.";

/// Plain Ctrl+D, Ctrl+W and Ctrl+K mean something to the shell, so app shortcuts add Shift,
/// like Windows Terminal. Ctrl+Tab and Ctrl+1 to Ctrl+9 switch tabs, so Ctrl+2 (NUL) and
/// Ctrl+6 no longer reach the shell. Alt+F4 quits through the window's close request.
pub fn fixed_bindings() -> Vec<KeyBinding> {
    let mut keys = vec![
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
    ];
    keys.extend((1..=9).map(|n| KeyBinding::new(&format!("ctrl-{n}"), SelectTab(n - 1), None)));
    keys
}

/// Whether `keys` is one of `fixed_bindings`.
pub fn is_fixed(keys: &str) -> bool {
    matches!(keys, "ctrl-tab" | "ctrl-shift-tab")
        || keys
            .strip_prefix("ctrl-")
            .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
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
        "new_host" => "ctrl-shift-n",
        "settings" => "ctrl-,",
        "close_tab" => "ctrl-shift-w",
        _ => return None,
    })
}

/// Keystrokes written the Windows way: "ctrl-shift-d" is "Ctrl+Shift+D".
pub fn format_keys(keys: &str) -> String {
    let Ok(k) = gpui::Keystroke::parse(keys) else {
        return keys.to_string();
    };
    let m = k.modifiers;
    let mut out = String::new();
    for (held, name) in [
        (m.control, "Ctrl+"),
        (m.alt, "Alt+"),
        (m.shift, "Shift+"),
        (m.platform, "Win+"),
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
/// reaches the shell (except paste, `terminal_paste`, and tab switching). `Some(alt)` when the chord is held, Alt picking the variant
/// (previous match, type password).
pub fn terminal_chord(m: Modifiers) -> Option<bool> {
    (m.control && m.shift).then_some(m.alt)
}

/// Ctrl+V and Shift+Insert paste in the terminal, like Windows Terminal, even though plain
/// Ctrl otherwise belongs to the shell. AltGr arrives as Ctrl+Alt, so Alt rules it out.
pub fn terminal_paste(m: Modifiers, key: &str) -> bool {
    match key {
        "v" => m.control && !m.shift && !m.alt,
        "insert" => m.shift && !m.control && !m.alt,
        _ => false,
    }
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

/// Whether a font family is installed or bundled. DirectWrite gives every unknown family
/// its own copy of the fallback font, so comparing resolved fonts can't tell; compare the
/// names instead (case-insensitive, like DirectWrite).
pub fn font_exists(ts: &TextSystem) -> impl Fn(&str) -> bool {
    let names: HashSet<String> = ts
        .all_font_names()
        .into_iter()
        .map(|n| n.to_lowercase())
        .collect();
    move |name| names.contains(&name.to_lowercase())
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

/// The title bar area lets Windows offer its window menu and snapping. Presses are handled
/// here only: gpui passes title bar presses on to the app too, and Windows acting on the
/// same double-click would maximize and restore at once.
pub fn title_bar<E: InteractiveElement>(el: E) -> E {
    el.window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            if ev.click_count >= 2 {
                toggle_maximize(window);
            } else {
                drag(window);
            }
        })
}

/// Native areas give snap layouts and hover; the click runs the action when the native
/// hit test misses (a click Windows handles itself never reaches it).
pub fn window_button(
    el: Stateful<Div>,
    area: WindowControlArea,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    el.window_control_area(area)
        .on_click(move |_, w, cx| match area {
            WindowControlArea::Max => toggle_maximize(w),
            _ => action(w, cx),
        })
}

/// gpui's `zoom_window` only maximizes on Windows, so a second press never restores.
fn toggle_maximize(window: &Window) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    // SAFETY: the handle comes from the live window, and both calls only take it by value.
    unsafe {
        let _ = ShowWindowAsync(
            hwnd,
            if IsZoomed(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_MAXIMIZE
            },
        );
    }
}

fn hwnd(window: &Window) -> Option<HWND> {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return None;
    };
    let RawWindowHandle::Win32(h) = handle.as_raw() else {
        return None;
    };
    Some(HWND(h.hwnd.get() as *mut std::ffi::c_void))
}

/// Start Windows' move loop at the cursor. It restores a maximized window only once the
/// mouse actually moves. Posted, so it runs after gpui finishes this event.
fn drag(window: &Window) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    let mut at = POINT::default();
    // SAFETY: the handle comes from the live window, `at` outlives the call, and the calls
    // only take their arguments by value.
    unsafe {
        if GetCursorPos(&mut at).is_err() {
            return;
        }
        let _ = ReleaseCapture();
        let _ = PostMessageW(
            Some(hwnd),
            WM_SYSCOMMAND,
            WPARAM((SC_MOVE | HTCAPTION) as usize),
            LPARAM(((at.y as u16 as isize) << 16) | at.x as u16 as isize),
        );
    }
}

/// Editors offered as quick picks when installed, with where their installers put them
/// (an environment variable for the base folder, then the path below it).
const KNOWN_EDITORS: &[(&str, &[(&str, &str)])] = &[
    (
        "Visual Studio Code",
        &[
            ("LOCALAPPDATA", r"Programs\Microsoft VS Code\Code.exe"),
            ("ProgramFiles", r"Microsoft VS Code\Code.exe"),
        ],
    ),
    ("Cursor", &[("LOCALAPPDATA", r"Programs\cursor\Cursor.exe")]),
    ("Zed", &[("LOCALAPPDATA", r"Programs\Zed\Zed.exe")]),
    (
        "Sublime Text",
        &[("ProgramFiles", r"Sublime Text\sublime_text.exe")],
    ),
    ("Notepad++", &[("ProgramFiles", r"Notepad++\notepad++.exe")]),
    (
        "Windsurf",
        &[("LOCALAPPDATA", r"Programs\Windsurf\Windsurf.exe")],
    ),
    (
        "VSCodium",
        &[
            ("LOCALAPPDATA", r"Programs\VSCodium\VSCodium.exe"),
            ("ProgramFiles", r"VSCodium\VSCodium.exe"),
        ],
    ),
];

/// The .exe of a known editor, when installed. Never a .cmd or .bat launcher (like VS
/// Code's `code.cmd`): those run through cmd.exe, which would interpret a server file name.
fn editor_exe(name: &str) -> Option<PathBuf> {
    let (_, places) = KNOWN_EDITORS.iter().find(|(n, _)| *n == name)?;
    places
        .iter()
        .filter_map(|(var, rel)| Some(PathBuf::from(std::env::var_os(var)?).join(rel)))
        .find(|p| p.exists())
}

/// Known editors that are installed.
pub fn installed_editors() -> Vec<&'static str> {
    KNOWN_EDITORS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| editor_exe(n).is_some())
        .collect()
}

/// Open a local file in `app` (a known editor's name or a program), or in Notepad without
/// one. Never the file association: a server file named like a program or script would run.
pub fn open_in_editor(path: &Path, app: &str) -> std::io::Result<()> {
    let program = match app {
        "" => PathBuf::from("notepad.exe"),
        _ => editor_exe(app).unwrap_or_else(|| PathBuf::from(app)),
    };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(drop)
}

/// The logged-in user, masked by "hide sensitive information".
pub fn local_user() -> Option<String> {
    std::env::var("USERNAME").ok()
}
