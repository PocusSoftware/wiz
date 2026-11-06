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
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
fn gray_theme_icons_drop_color() {
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_monochrome(false);
    assert_eq!(icon_color(RED), RED);
    set_monochrome(true);
    assert_eq!(icon_color(RED), MUTED);
    assert_eq!(icon_color(accent_text()), TEXT);
    assert_eq!(icon_color(MUTED), MUTED);
    set_monochrome(false);
}

#[test]
fn monochrome_status_dots_stay_apart() {
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_monochrome(false);
    assert_eq!(dot_color(GREEN), GREEN);
    set_monochrome(true);
    assert_eq!(dot_color(GREEN), TEXT);
    assert_eq!(dot_color(YELLOW), MUTED);
    assert_eq!(dot_color(ACTIVE), ACTIVE);
    set_monochrome(false);
}

#[test]
fn oled_black() {
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_oled(true);
    assert_eq!(bg(), 0x000000);
    set_oled(false);
    assert_eq!(bg(), DARK_BG);
}
