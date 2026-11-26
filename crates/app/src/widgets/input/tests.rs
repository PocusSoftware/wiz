//! Tests for `input`.

use gpui::px;

use super::*;

#[test]
fn scrolls_just_enough_to_keep_the_caret_in_view() {
    let view = px(100.);
    assert_eq!(scroll_to_show(px(50.), px(80.), view, px(0.)), px(0.));
    assert_eq!(scroll_to_show(px(300.), px(300.), view, px(0.)), px(202.));
    assert_eq!(scroll_to_show(px(250.), px(300.), view, px(202.)), px(202.));
    assert_eq!(scroll_to_show(px(120.), px(300.), view, px(202.)), px(120.));
    assert_eq!(scroll_to_show(px(40.), px(60.), view, px(30.)), px(0.));
}
