//! Linux: Ctrl+Shift shortcuts like Windows, a client-drawn title bar and resize borders
//! (Wayland and X11), installed editors found on `PATH`. No Keychain or rsync.

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, App, Decorations, Div, InteractiveElement, IntoElement, Modifiers, MouseButton,
    MouseDownEvent, ResizeEdge, Stateful, StatefulInteractiveElement, Styled, TitlebarOptions,
    Window, WindowAppearance, WindowControlArea, WindowDecorations, WindowOptions, div, px,
};

pub use super::ctrl::{
    MODIFIER_HINT, ZOOM_KEYS, default_shortcut, fixed_bindings, font_exists, format_keys, is_fixed,
    link_click, on_window_open, terminal_chord, view_shortcut,
};

pub const RSYNC: bool = false;

pub const OS: gpui::OperatingSystem = gpui::OperatingSystem::Linux;

pub const LOGO_KEY: &str = "Super+";

pub const QUIT_KEYS: &str = "Alt+F4";

pub const AGENT_DEFAULT: &str = "SSH_AUTH_SOCK";
pub const AGENT_HINT: &str = "Socket of the agent used by hosts that log in with the SSH agent. Empty uses the system agent (ssh-add).";
pub const ONEPASSWORD_AGENT: Option<&str> = Some("~/.1password/agent.sock");
pub const EDITOR_HINT: &str = "Program that opens files with Enter. Pick one below or type a program name or path. Empty uses the first installed one below.";

/// Ctrl+V is the shell's literal-next, so only Shift+Insert pastes outside the chord.
pub fn terminal_paste(m: Modifiers, key: &str) -> bool {
    key == "insert" && m.shift && !m.control && !m.alt
}

/// AltGr is its own level in xkb and arrives without Ctrl or Alt, so it needs no exception.
pub fn types_char(m: Modifiers) -> bool {
    !m.platform && !m.control
}

pub fn alt_gr(_: Modifiers, _: &str, _: Option<&str>) -> bool {
    false
}

pub fn on_launch() {}

/// The app draws its own title bar, so it asks for client-side decorations. `app_id`
/// matches the `.desktop` file, which gives the window its icon.
pub fn window_options() -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Wiz".into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        app_id: Some(ssh_core::APP_ID.into()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

/// Width of the invisible borders that resize a client-decorated window.
const EDGE: f32 = 5.;

/// Borders along the window's edges that start a compositor resize. None when the window
/// is server-decorated, or tiled or maximized (nothing to drag then).
pub fn resize_handles(window: &Window) -> Vec<AnyElement> {
    let Decorations::Client { tiling } = window.window_decorations() else {
        return Vec::new();
    };
    if tiling.is_tiled() {
        return Vec::new();
    }
    use ResizeEdge::*;
    [
        Top,
        Bottom,
        Left,
        Right,
        TopLeft,
        TopRight,
        BottomLeft,
        BottomRight,
    ]
    .into_iter()
    .map(resize_handle)
    .collect()
}

fn resize_handle(edge: ResizeEdge) -> AnyElement {
    use ResizeEdge::*;
    let d = div().absolute().occlude();
    let corner = px(EDGE * 2.);
    let d = match edge {
        Top => d.top_0().left_0().w_full().h(px(EDGE)),
        Bottom => d.bottom_0().left_0().w_full().h(px(EDGE)),
        Left => d.top_0().left_0().h_full().w(px(EDGE)),
        Right => d.top_0().right_0().h_full().w(px(EDGE)),
        TopLeft => d.top_0().left_0().size(corner),
        TopRight => d.top_0().right_0().size(corner),
        BottomLeft => d.bottom_0().left_0().size(corner),
        BottomRight => d.bottom_0().right_0().size(corner),
    };
    let d = match edge {
        Top | Bottom => d.cursor_ns_resize(),
        Left | Right => d.cursor_ew_resize(),
        TopLeft | BottomRight => d.cursor_nwse_resize(),
        TopRight | BottomLeft => d.cursor_nesw_resize(),
    };
    d.on_mouse_down(MouseButton::Left, move |_, window, cx| {
        cx.stop_propagation();
        window.start_window_resize(edge);
    })
    .into_any_element()
}

/// Drag moves the window, double-click maximizes or restores it.
pub fn title_bar<E: InteractiveElement>(el: E) -> E {
    el.on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, _| {
        if ev.click_count >= 2 {
            window.zoom_window();
        } else {
            window.start_window_move();
        }
    })
}

pub fn window_button(
    el: Stateful<Div>,
    _: WindowControlArea,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    el.on_click(move |_, w, cx| action(w, cx))
}

/// Editors offered as quick picks when installed, with the programs they install (Arch
/// names Zed's `zeditor`).
const KNOWN_EDITORS: &[(&str, &[&str])] = &[
    ("Visual Studio Code", &["code"]),
    ("Cursor", &["cursor"]),
    ("Zed", &["zeditor", "zed"]),
    ("Sublime Text", &["subl"]),
    ("Kate", &["kate"]),
    ("GNOME Text Editor", &["gnome-text-editor"]),
    ("gedit", &["gedit"]),
    ("Mousepad", &["mousepad"]),
    ("Windsurf", &["windsurf"]),
    ("VSCodium", &["codium"]),
];

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|p| p.is_file())
}

fn editor_program(name: &str) -> Option<PathBuf> {
    let (_, programs) = KNOWN_EDITORS.iter().find(|(n, _)| *n == name)?;
    programs.iter().find_map(|p| on_path(p))
}

pub fn installed_editors() -> Vec<&'static str> {
    KNOWN_EDITORS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| editor_program(n).is_some())
        .collect()
}

/// Open a local file in `app` (a known editor's name or a program), or in the first
/// installed known editor. Never `xdg-open`: a server file named like a `.desktop` file or
/// script could run.
pub fn open_in_editor(path: &Path, app: &str) -> std::io::Result<()> {
    let program = match app {
        "" => installed_editors()
            .first()
            .and_then(|n| editor_program(n))
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "No text editor found. Pick one under Settings.",
                )
            })?,
        _ => editor_program(app).unwrap_or_else(|| PathBuf::from(app)),
    };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(drop)
}

pub fn local_user() -> Option<String> {
    std::env::var("USER").ok()
}

/// Whether the desktop prefers dark colors. gpui has no high contrast setting on Linux.
pub fn system_look(window: &Window) -> (bool, bool) {
    let dark = matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    (dark, false)
}
