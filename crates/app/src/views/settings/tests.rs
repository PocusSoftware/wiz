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
