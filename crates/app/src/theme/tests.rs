//! Tests for `theme`.

use super::*;

#[test]
fn parse_hex_colors() {
    assert_eq!(parse_hex("#f05a29"), Some(0xf05a29));
    assert_eq!(parse_hex(" 3E63DD "), Some(0x3e63dd));
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
    // The default keeps its hand-picked shades.
    set_accent(DEFAULT_ACCENT);
    assert_eq!(accent_text(), 0xf87c49);
    assert_eq!(selection(), 0x4a2a1e);
    // Others are mixed: lighter toward white, darker toward the background.
    set_accent(0x3e63dd);
    assert_eq!(accent(), 0x3e63dd);
    assert_eq!(accent_text(), 0x6582e4);
    assert_eq!(selection(), 0x192346);
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
    assert_eq!(accent_text(), 0xc04821);
    assert_eq!(selection(), 0xfbd1c3);
    // The gray theme's accent turns dark, so it shows on white.
    set_monochrome(true);
    assert_eq!(accent(), 0x2c2c2e);
    assert_eq!(on_accent(), 0xffffff);
    set_monochrome(false);
    // OLED black is for the dark theme only.
    set_oled(true);
    assert_eq!(bg(), 0xffffff);
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
    set_monochrome(false);
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
