//! Dark and light colors in standard and high contrast, with an accent the user can pick.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use gpui::{Hsla, SharedString, rgb, rgba, transparent_black};
use wiz_core::config::{Appearance, Contrast};

/// 0xRRGGBB, or 0xRRGGBBAA when the value does not fit in 24 bits.
pub fn c(hex: u32) -> Hsla {
    if hex > 0x00ff_ffff {
        rgba(hex).into()
    } else {
        rgb(hex).into()
    }
}

/// The colors of one look (dark or light, standard or high contrast).
#[derive(Debug)]
pub struct Palette {
    /// The window and terminal background, unless OLED black is on.
    bg: u32,
    pub panel: u32,
    pub elevated: u32,
    pub element: u32,
    pub active: u32,
    /// A plain button under the pointer.
    pub active_hover: u32,
    pub hover: u32,
    pub border: u32,
    pub border_variant: u32,
    pub text: u32,
    pub muted: u32,
    pub placeholder: u32,
    pub green: u32,
    pub red: u32,
    pub yellow: u32,
    /// The accent of the gray theme (`colorful` off).
    gray_accent: u32,
    /// Background of terminal search hits other than the current one.
    pub match_bg: u32,
    /// The 16 terminal colors.
    pub ansi: [u32; 16],
}

/// Near black, for text on a light accent.
const DARK_BG: u32 = 0x0a0a0b;

const DARK_ANSI: [u32; 16] = [
    0x0a0a0b, 0xf66335, 0xdfbda0, 0xf05a29, 0xa0a0a6, 0xf87c49, 0xb6b6bb, 0xd7d7db, 0x626269,
    0xf87c49, 0xe8cdb6, 0xf87c49, 0xc4c4c9, 0xf9926a, 0xd7d7db, 0xfafafb,
];

/// White and bright white are grays, so text in them still reads on white.
const LIGHT_ANSI: [u32; 16] = [
    0x1d1d1f, 0xc2421a, 0x8c6a4b, 0xb8441c, 0x5f5f66, 0xc25428, 0x5a5a61, 0x6e6e75, 0x55555c,
    0xcf4f24, 0x7d5e40, 0xc4502a, 0x4c4c52, 0xb84e26, 0x46464c, 0x8e8e93,
];

/// From the Zed "Hearth Dark" theme.
const DARK: Palette = Palette {
    bg: DARK_BG,
    panel: 0x171718,
    elevated: 0x1d1d1e,
    element: 0x161617,
    active: 0x262628,
    active_hover: 0x323235,
    hover: 0xffffff0a,
    border: 0xffffff1a,
    border_variant: 0xffffff12,
    text: 0xd7d7db,
    muted: 0xa4a4a9,
    placeholder: 0x808085,
    green: 0x8fbf8a,
    red: 0xe5534b,
    yellow: 0xe3b341,
    gray_accent: 0xe8e8eb,
    match_bg: 0x6b5416,
    ansi: DARK_ANSI,
};

const DARK_HIGH: Palette = Palette {
    bg: 0x000000,
    panel: 0x141415,
    elevated: 0x1c1c1e,
    element: 0x0f0f10,
    active: 0x2f2f32,
    active_hover: 0x3d3d41,
    hover: 0xffffff1f,
    border: 0xffffff66,
    border_variant: 0xffffff40,
    text: 0xffffff,
    muted: 0xdcdce0,
    placeholder: 0xb4b4ba,
    green: 0xa8e6a3,
    red: 0xff7067,
    yellow: 0xffd060,
    gray_accent: 0xffffff,
    match_bg: 0x7a5f10,
    ansi: DARK_ANSI,
};

/// Overlays are a near black with alpha: `c` reads a value of 0xffffff or less as opaque.
const LIGHT: Palette = Palette {
    bg: 0xf3f2ef,
    panel: 0xebeae6,
    elevated: 0xf8f7f4,
    element: 0xf9f8f6,
    active: 0xe0dfda,
    active_hover: 0xd5d4cf,
    hover: 0x1c1c1e0d,
    border: 0x1c1c1e24,
    border_variant: 0x1c1c1e17,
    text: 0x2a2a2d,
    muted: 0x5f5f63,
    placeholder: 0x8b8a8e,
    green: 0x2e7d32,
    red: 0xd1242f,
    yellow: 0x9a6700,
    gray_accent: 0x2f2f31,
    match_bg: 0xfde68a,
    ansi: LIGHT_ANSI,
};

const LIGHT_HIGH: Palette = Palette {
    bg: 0xffffff,
    panel: 0xefeff1,
    elevated: 0xffffff,
    element: 0xffffff,
    active: 0xd6d6da,
    active_hover: 0xc6c6cb,
    hover: 0x1c1c1e1f,
    border: 0x1c1c1e80,
    border_variant: 0x1c1c1e59,
    text: 0x000000,
    muted: 0x333338,
    placeholder: 0x55555b,
    green: 0x1b5e20,
    red: 0xa8071a,
    yellow: 0x6b4700,
    gray_accent: 0x000000,
    match_bg: 0xfacc15,
    ansi: LIGHT_ANSI,
};

/// Indexed by `light * 2 + high`.
static PALETTES: [Palette; 4] = [DARK, DARK_HIGH, LIGHT, LIGHT_HIGH];

pub const TITLEBAR_H: f32 = 30.0;

/// Blue, the colorful accent unless settings.toml picks another (`accent_color`).
pub const DEFAULT_ACCENT: u32 = 0x3a63ec;

/// What the palette is picked from: the settings, and what the system uses.
struct Look {
    appearance: Appearance,
    contrast: Contrast,
    system_dark: bool,
    system_high: bool,
}

static LOOK: Mutex<Look> = Mutex::new(Look {
    appearance: Appearance::System,
    contrast: Contrast::System,
    system_dark: true,
    system_high: false,
});
static PALETTE: AtomicU8 = AtomicU8::new(0);
static OLED: AtomicBool = AtomicBool::new(false);
static BLUR: AtomicBool = AtomicBool::new(false);
static ACCENT: AtomicU32 = AtomicU32::new(DEFAULT_ACCENT);
static MONOCHROME: AtomicBool = AtomicBool::new(false);

/// Change what the palette is picked from. True when that changed the palette.
fn update_look(f: impl FnOnce(&mut Look)) -> bool {
    let mut look = LOOK.lock().unwrap_or_else(PoisonError::into_inner);
    f(&mut look);
    let dark = match look.appearance {
        Appearance::System => look.system_dark,
        Appearance::Light => false,
        Appearance::Dark => true,
    };
    let high = match look.contrast {
        Contrast::System => look.system_high,
        Contrast::Standard => false,
        Contrast::High => true,
    };
    let index = u8::from(!dark) * 2 + u8::from(high);
    PALETTE.swap(index, Ordering::Relaxed) != index
}

/// The light or dark and contrast settings.
pub fn set_look(appearance: Appearance, contrast: Contrast) {
    update_look(|l| (l.appearance, l.contrast) = (appearance, contrast));
}

/// What the system uses, followed by the `System` settings. True when the colors changed.
pub fn set_system(dark: bool, high: bool) -> bool {
    update_look(|l| (l.system_dark, l.system_high) = (dark, high))
}

/// The colors in use.
pub fn palette() -> &'static Palette {
    &PALETTES[PALETTE.load(Ordering::Relaxed) as usize]
}

fn is_dark() -> bool {
    PALETTE.load(Ordering::Relaxed) < 2
}

fn is_high() -> bool {
    PALETTE.load(Ordering::Relaxed) % 2 == 1
}

/// Use `rgb` (0xRRGGBB) as the accent while colorful.
pub fn set_accent(rgb: u32) {
    ACCENT.store(rgb, Ordering::Relaxed);
}

/// Pure black behind the terminal and the window of the dark theme, so OLED screens turn
/// those pixels off. Panels and inputs keep their grays to stay apart from it.
pub fn set_oled(on: bool) {
    OLED.store(on, Ordering::Relaxed);
}

/// See-through window of the dark theme, with the desktop blurred behind it.
pub fn set_blur(on: bool) {
    BLUR.store(on, Ordering::Relaxed);
}

/// Whether the window shows the blurred desktop behind it.
pub fn blurred() -> bool {
    BLUR.load(Ordering::Relaxed) && is_dark()
}

/// Draw icons and status dots without color, and the accent in gray (the gray theme).
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
    let p = palette();
    match color {
        c if c == accent() || c == accent_text() => p.text,
        c if c == p.red || c == p.green || c == p.yellow => p.muted,
        c => c,
    }
}

/// The color of a status dot: as asked, or shades of gray in the gray theme.
/// Live (green) turns white and connecting (yellow) gray, so they stay apart.
pub fn dot_color(color: u32) -> u32 {
    if !MONOCHROME.load(Ordering::Relaxed) {
        return color;
    }
    let p = palette();
    match color {
        c if c == p.green => p.text,
        c if c == p.yellow => p.muted,
        c => c,
    }
}

pub fn danger() -> u32 {
    if MONOCHROME.load(Ordering::Relaxed) {
        accent()
    } else {
        palette().red
    }
}

pub fn on_danger() -> u32 {
    if MONOCHROME.load(Ordering::Relaxed) {
        on_accent()
    } else {
        0xffffff
    }
}

/// The window and terminal background.
pub fn bg() -> u32 {
    if OLED.load(Ordering::Relaxed) && is_dark() {
        0x000000
    } else {
        palette().bg
    }
}

/// The fill of the window itself: the background, tinted over the desktop when blurred.
pub fn backdrop() -> Hsla {
    if blurred() {
        with_alpha(bg(), 0.6)
    } else {
        c(bg())
    }
}

/// The fill of views on the window background: none when blurred, so they show the window's
/// tint instead of stacking their own on it.
pub fn bg_fill() -> Hsla {
    if blurred() {
        transparent_black()
    } else {
        c(bg())
    }
}

/// Buttons, the active tab, the cursor, focus.
pub fn accent() -> u32 {
    if MONOCHROME.load(Ordering::Relaxed) {
        palette().gray_accent
    } else {
        ACCENT.load(Ordering::Relaxed)
    }
}

/// Accent text and icons: lighter on dark colors and darker on light ones, so they read on
/// the background.
pub fn accent_text() -> u32 {
    let rgb = accent();
    let dark = is_dark();
    let amount = if is_high() { 0.4 } else { 0.2 };
    mix(rgb, if dark { 0xffffff } else { 0x000000 }, amount)
}

/// Text and icons on an accent background: white, or near black when the accent is too
/// light for white to read (like the white swatch).
pub fn on_accent() -> u32 {
    let rgb = accent();
    let [r, g, b] = [16, 8, 0].map(|shift| ((rgb >> shift) & 0xff) as f32);
    if 0.2126 * r + 0.7152 * g + 0.0722 * b > 160. {
        DARK_BG
    } else {
        0xffffff
    }
}

/// Selected text in the terminal: the accent mixed into the background.
pub fn selection() -> u32 {
    mix(accent(), palette().bg, if is_high() { 0.6 } else { 0.72 })
}

/// The accent see-through: focus borders, selected rows and text, drop targets.
pub fn accent_alpha(alpha: f32) -> Hsla {
    with_alpha(accent(), alpha)
}

/// `rgb` (0xRRGGBB) see-through.
pub fn with_alpha(rgb: u32, alpha: f32) -> Hsla {
    let mut color = c(rgb);
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
pub fn init_fonts(exists: &dyn Fn(&str) -> bool, s: &wiz_core::config::Settings) {
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

pub fn zoomed(size: f32, step: f32) -> f32 {
    (size.round() + step).clamp(8.0, 32.0)
}

pub fn zoom_percent(size: f32) -> u32 {
    (size / wiz_core::config::Settings::default().terminal_font_size() * 100.).round() as u32
}

#[cfg(test)]
mod tests {
    //! Tests for `theme`.

    use super::*;

    #[test]
    fn parse_hex_colors() {
        assert_eq!(parse_hex("#f05a29"), Some(0xf05a29));
        assert_eq!(parse_hex(" 3A63EC "), Some(0x3a63ec));
        for bad in ["", "#fff", "#f05a2", "#f05a29aa", "#+12345", "orange"] {
            assert_eq!(parse_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn accent_shades() {
        let _g = crate::testing::GLOBALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set_monochrome(false);
        // Mixed: lighter toward white, darker toward the background.
        set_accent(DEFAULT_ACCENT);
        assert_eq!(accent(), 0x3a63ec);
        assert_eq!(accent_text(), 0x6182f0);
        assert_eq!(selection(), 0x17234a);
        assert_eq!(on_accent(), 0xffffff);
        // White text would vanish on a white accent.
        set_accent(0xffffff);
        assert_eq!(on_accent(), DARK_BG);
        set_accent(DEFAULT_ACCENT);
    }

    #[test]
    fn settings_and_system_pick_the_palette() {
        let _g = crate::testing::GLOBALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set_look(Appearance::System, Contrast::System);
        set_system(true, false);
        assert!(std::ptr::eq(palette(), &PALETTES[0]));
        // The system turning light and high contrast changes the colors.
        assert!(set_system(false, true));
        assert!(!is_dark() && is_high());
        assert!(!set_system(false, true));
        // Picked settings win over the system.
        set_look(Appearance::Dark, Contrast::Standard);
        assert!(is_dark() && !is_high());
        set_look(Appearance::Light, Contrast::High);
        assert_eq!(palette().text, 0x000000);
        set_look(Appearance::System, Contrast::System);
        set_system(true, false);
    }

    #[test]
    fn light_accent_shades_darken() {
        let _g = crate::testing::GLOBALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set_look(Appearance::Light, Contrast::Standard);
        set_monochrome(false);
        set_accent(DEFAULT_ACCENT);
        assert_eq!(accent_text(), 0x2e4fbd);
        assert_eq!(selection(), 0xbfcaee);
        // The gray theme's accent turns dark, so it shows on white.
        set_monochrome(true);
        assert_eq!(accent(), 0x2f2f31);
        assert_eq!(on_accent(), 0xffffff);
        set_monochrome(false);
        // OLED black is for the dark theme only.
        set_oled(true);
        assert_eq!(bg(), LIGHT.bg);
        set_oled(false);
        set_look(Appearance::System, Contrast::System);
    }

    #[test]
    fn gray_theme_icons_drop_color() {
        let _g = crate::testing::GLOBALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set_monochrome(false);
        let p = palette();
        assert_eq!(icon_color(p.red), p.red);
        set_monochrome(true);
        assert_eq!(icon_color(p.red), p.muted);
        assert_eq!(icon_color(accent_text()), p.text);
        assert_eq!(icon_color(p.muted), p.muted);
        assert_eq!(danger(), accent());
        assert_eq!(on_danger(), on_accent());
        set_monochrome(false);
        assert_eq!(danger(), p.red);
    }

    #[test]
    fn monochrome_status_dots_stay_apart() {
        let _g = crate::testing::GLOBALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set_monochrome(false);
        let p = palette();
        assert_eq!(dot_color(p.green), p.green);
        set_monochrome(true);
        assert_eq!(dot_color(p.green), p.text);
        assert_eq!(dot_color(p.yellow), p.muted);
        assert_eq!(dot_color(p.active), p.active);
        set_monochrome(false);
    }

    #[test]
    fn oled_black() {
        let _g = crate::testing::GLOBALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set_oled(true);
        assert_eq!(bg(), 0x000000);
        set_oled(false);
        assert_eq!(bg(), DARK.bg);
    }

    #[test]
    fn blur_tints_only_the_window_of_the_dark_theme() {
        let _g = crate::testing::GLOBALS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set_look(Appearance::Dark, Contrast::Standard);
        set_blur(true);
        assert!(blurred());
        assert_eq!(backdrop().a, 0.6);
        assert_eq!(bg_fill().a, 0.);
        assert_eq!(bg(), DARK.bg);
        set_look(Appearance::Light, Contrast::Standard);
        assert!(!blurred());
        assert_eq!(backdrop(), c(LIGHT.bg));
        assert_eq!(bg_fill(), c(LIGHT.bg));
        set_blur(false);
        set_look(Appearance::System, Contrast::System);
    }

    #[test]
    fn zoom_steps_and_percent() {
        assert_eq!(zoomed(13., 1.), 14.);
        assert_eq!(zoomed(32., 1.), 32.);
        assert_eq!(zoomed(8., -1.), 8.);
        assert_eq!(zoom_percent(13.), 100);
        assert_eq!(zoom_percent(26.), 200);
        assert_eq!(zoom_percent(14.), 108);
    }
}
