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
pub const GREEN: u32 = 0x8fbf8a;
pub const RED: u32 = 0xe5534b;
pub const YELLOW: u32 = 0xe3b341;

pub const TITLEBAR_H: f32 = 30.0;
/// Space left of the first tab for the macOS traffic lights.
pub const TRAFFIC_LIGHTS_W: f32 = 80.0;
pub const FONT_UI: &str = ".SystemUIFont";

static MONO: OnceLock<SharedString> = OnceLock::new();

/// Terminal font: the Zed buffer font when installed, else Menlo.
pub fn init_fonts(available: &[String]) {
    let pick = ["JetBrainsMono Nerd Font", "JetBrains Mono", "Menlo"]
        .into_iter()
        .find(|f| available.iter().any(|a| a == f))
        .unwrap_or("Menlo");
    let _ = MONO.set(pick.into());
}

pub fn mono() -> SharedString {
    MONO.get().cloned().unwrap_or_else(|| "Menlo".into())
}
