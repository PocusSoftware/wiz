//! Colors from the Zed "Hearth Dark" theme, with an accent the user can pick.

#[cfg(test)]
mod tests;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use gpui::{Hsla, SharedString, rgb, rgba};

/// 0xRRGGBB, or 0xRRGGBBAA when the value does not fit in 24 bits.
pub fn c(hex: u32) -> Hsla {
    if hex > 0x00ff_ffff {
        rgba(hex).into()
    } else {
        rgb(hex).into()
    }
}

/// The window background, unless OLED black is on.
const DARK_BG: u32 = 0x0a0a0b;
pub const PANEL: u32 = 0x171718;
pub const ELEVATED: u32 = 0x1d1d1e;
pub const ELEMENT: u32 = 0x161617;
pub const ACTIVE: u32 = 0x262628;
pub const HOVER: u32 = 0xffffff0a;
pub const BORDER: u32 = 0xffffff1a;
pub const BORDER_VARIANT: u32 = 0xffffff12;
pub const TEXT: u32 = 0xd7d7db;
pub const MUTED: u32 = 0xa4a4a9;
pub const PLACEHOLDER: u32 = 0x808085;
pub const GREEN: u32 = 0x8fbf8a;
pub const RED: u32 = 0xe5534b;
pub const YELLOW: u32 = 0xe3b341;

pub const TITLEBAR_H: f32 = 30.0;

/// Orange, the colorful accent unless settings.toml picks another (`accent_color`).
pub const DEFAULT_ACCENT: u32 = 0xf05a29;
/// The default's hand-picked shades: for text and icons, and for the terminal selection.
const DEFAULT_ACCENT_TEXT: u32 = 0xf87c49;
const DEFAULT_SELECTION: u32 = 0x4a2a1e;
/// The accent of the gray theme (`colorful` off): near white, so buttons get dark text.
pub const GRAY_ACCENT: u32 = 0xe8e8eb;

static BG: AtomicU32 = AtomicU32::new(DARK_BG);
static ACCENT: AtomicU32 = AtomicU32::new(DEFAULT_ACCENT);
static ACCENT_TEXT: AtomicU32 = AtomicU32::new(DEFAULT_ACCENT_TEXT);
static SELECTION: AtomicU32 = AtomicU32::new(DEFAULT_SELECTION);
static MONOCHROME: AtomicBool = AtomicBool::new(false);

/// Use `rgb` (0xRRGGBB) as the accent. Other colors than the default get their shades
/// mixed: lighter for text, dark for the terminal selection.
pub fn set_accent(rgb: u32) {
    let (text, selection) = if rgb == DEFAULT_ACCENT {
        (DEFAULT_ACCENT_TEXT, DEFAULT_SELECTION)
    } else {
        (mix(rgb, 0xffffff, 0.2), mix(rgb, DARK_BG, 0.72))
    };
    ACCENT.store(rgb, Ordering::Relaxed);
    ACCENT_TEXT.store(text, Ordering::Relaxed);
    SELECTION.store(selection, Ordering::Relaxed);
}

/// Pure black behind the terminal and the window, so OLED screens turn those pixels off.
/// Panels and inputs keep their grays to stay apart from it.
pub fn set_oled(on: bool) {
    BG.store(if on { 0x000000 } else { DARK_BG }, Ordering::Relaxed);
}

/// Draw icons and status dots without color (the gray theme).
pub fn set_monochrome(on: bool) {
    MONOCHROME.store(on, Ordering::Relaxed);
}

/// The color an icon is drawn in: as asked, or gray and white in the gray theme.
/// Colored icons turn white (the accent) or gray (red, green, yellow), so an active tab
/// still stands out from the others.
pub fn icon_color(color: u32) -> u32 {
    if !MONOCHROME.load(Ordering::Relaxed) {
        return color;
    }
    match color {
        c if c == accent() || c == accent_text() => TEXT,
        RED | GREEN | YELLOW => MUTED,
        c => c,
    }
}

/// The color of a status dot: as asked, or shades of gray in the gray theme.
/// Live (green) turns white and connecting (yellow) gray, so they stay apart.
pub fn dot_color(color: u32) -> u32 {
    if !MONOCHROME.load(Ordering::Relaxed) {
        return color;
    }
    match color {
        GREEN => TEXT,
        YELLOW => MUTED,
        c => c,
    }
}

/// The window and terminal background.
pub fn bg() -> u32 {
    BG.load(Ordering::Relaxed)
}

/// Buttons, the active tab, the cursor, focus.
pub fn accent() -> u32 {
    ACCENT.load(Ordering::Relaxed)
}

/// Accent text and icons, a little lighter so they read on the dark background.
pub fn accent_text() -> u32 {
    ACCENT_TEXT.load(Ordering::Relaxed)
}

/// Text and icons on an accent background: white, or the dark background color when the
/// accent is too light for white to read (like the white swatch).
pub fn on_accent() -> u32 {
    let rgb = accent();
    let [r, g, b] = [16, 8, 0].map(|shift| ((rgb >> shift) & 0xff) as f32);
    if 0.2126 * r + 0.7152 * g + 0.0722 * b > 160. {
        DARK_BG
    } else {
        0xffffff
    }
}

/// Selected text in the terminal.
pub fn selection() -> u32 {
    SELECTION.load(Ordering::Relaxed)
}

/// The accent see-through: focus borders, selected rows and text, drop targets.
pub fn accent_alpha(alpha: f32) -> Hsla {
    let mut color = c(accent());
    color.a = alpha;
    color
}

/// "#RRGGBB" (the # is optional) as 0xRRGGBB.
pub fn parse_hex(s: &str) -> Option<u32> {
    let hex = s.trim().trim_start_matches('#');
    (hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

/// `a` moved toward `b` by `t` (0 to 1), per channel.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    [16, 8, 0].iter().fold(0, |out, shift| {
        let (x, y) = ((a >> shift) & 0xff, (b >> shift) & 0xff);
        let v = (x as f32 + (y as f32 - x as f32) * t).round() as u32;
        out | (v << shift)
    })
}

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
