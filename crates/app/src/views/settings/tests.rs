//! Tests for the settings page.

use gpui::TestAppContext;

use super::*;

#[gpui::test]
fn opens_on_appearance_and_keeps_the_picked_page(cx: &mut TestAppContext) {
    let view = cx.new(SettingsView::new);
    view.read_with(cx, |v, _| assert_eq!(v.category, Category::Appearance));
    view.update(cx, |v, cx| v.show(Category::Privacy, cx));
    view.read_with(cx, |v, _| assert_eq!(v.category, Category::Privacy));
}

#[test]
fn every_page_has_its_own_name() {
    let mut names: Vec<_> = Category::ALL.iter().map(|c| c.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Category::ALL.len());
}

#[test]
fn picker_colors_round_trip() {
    use super::accent::{hsv_to_rgb, rgb_to_hsv};
    for rgb in [0xf05a29, 0x3e63dd, 0xffffff, 0x000000, 0x808080, 0x12a594] {
        assert_eq!(hsv_to_rgb(rgb_to_hsv(rgb)), rgb, "{rgb:06x}");
    }
    assert_eq!(hsv_to_rgb((0., 1., 1.)), 0xff0000);
    assert_eq!(hsv_to_rgb((120., 1., 1.)), 0x00ff00);
    assert_eq!(hsv_to_rgb((240., 1., 1.)), 0x0000ff);
}
