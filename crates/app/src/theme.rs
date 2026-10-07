//! Colors from the Zed "Hearth Dark" theme.

use std::sync::OnceLock;

use gpui::{Hsla, SharedString, rgb, rgba};

/// 0xRRGGBB, or 0xRRGGBBAA when the value does not fit in 24 bits.
pub fn c(hex: u32) -> Hsla {
    if hex > 0x00ff_ffff {
        rgba(hex).into()
    } else {
        rgb(hex).into()
    }
}

pub const BG: u32 = 0x0a0a0b;
pub const PANEL: u32 = 0x171718;
pub const ELEVATED: u32 = 0x1d1d1e;
pub const ELEMENT: u32 = 0x161617;
pub const ACTIVE: u32 = 0x262628;
pub const HOVER: u32 = 0xffffff0a;
pub const BORDER: u32 = 0xffffff1a;
pub const BORDER_VARIANT: u32 = 0xffffff12;
pub const BORDER_FOCUSED: u32 = 0xf05a2966;
pub const TEXT: u32 = 0xd7d7db;
pub const MUTED: u32 = 0xa4a4a9;
pub const PLACEHOLDER: u32 = 0x808085;
pub const ACCENT: u32 = 0xf05a29;
pub const ACCENT_TEXT: u32 = 0xf87c49;
pub const SELECTION: u32 = 0x4a2a1e;
pub const SELECTED_ROW: u32 = 0xf05a2966;
/// Highlight behind selected text in inputs and logs.
pub const SELECTION_TEXT: u32 = 0xf05a2f66;
pub const GREEN: u32 = 0x8fbf8a;
pub const RED: u32 = 0xe5534b;
pub const YELLOW: u32 = 0xe3b341;

pub const TITLEBAR_H: f32 = 30.0;

static MONO: OnceLock<SharedString> = OnceLock::new();
static UI: OnceLock<SharedString> = OnceLock::new();

/// Families of the bundled fonts (`assets::FONTS`), used unless settings.toml names another.
const UI_FONT: &str = "IBM Plex Sans";
const MONO_FONT: &str = "JetBrains Mono";

fn pick(want: Option<&str>, exists: &dyn Fn(&str) -> bool, fallback: &str) -> SharedString {
    want.filter(|f| exists(f))
        .unwrap_or(fallback)
        .to_string()
        .into()
}

/// UI font: settings.toml, else the bundled IBM Plex Sans.
/// Terminal font: settings.toml, else the Zed buffer font when installed (it has the Nerd
/// Font icons), else the bundled JetBrains Mono.
/// `exists` must be cheap: listing every installed font costs ~150 ms at startup on macOS.
pub fn init_fonts(exists: &dyn Fn(&str) -> bool, s: &ssh_core::config::Settings) {
    let _ = UI.set(pick(s.ui_font.as_deref(), exists, UI_FONT));
    let nerd = "JetBrainsMono Nerd Font";
    let _ = MONO.set(pick(
        s.terminal_font.as_deref(),
        exists,
        if exists(nerd) { nerd } else { MONO_FONT },
    ));
}

pub fn ui() -> SharedString {
    UI.get().cloned().unwrap_or_else(|| UI_FONT.into())
}

pub fn mono() -> SharedString {
    MONO.get().cloned().unwrap_or_else(|| MONO_FONT.into())
}

pub fn mono_size() -> f32 {
    crate::prefs::get().terminal_font_size().clamp(6.0, 48.0)
}
