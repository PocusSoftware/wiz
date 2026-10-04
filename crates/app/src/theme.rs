use gpui::{Hsla, rgb};

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub const BG: u32 = 0x1e1e2e;
pub const PANEL: u32 = 0x181825;
pub const SURFACE: u32 = 0x313244;
pub const HOVER: u32 = 0x2a2b3c;
pub const BORDER: u32 = 0x45475a;
pub const TEXT: u32 = 0xcdd6f4;
pub const DIM: u32 = 0x9399b2;
pub const ACCENT: u32 = 0x89b4fa;
pub const GREEN: u32 = 0xa6e3a1;
pub const RED: u32 = 0xf38ba8;
pub const YELLOW: u32 = 0xf9e2af;

pub const FONT_UI: &str = ".SystemUIFont";
pub const FONT_MONO: &str = "Menlo";
