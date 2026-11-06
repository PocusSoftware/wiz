//! Tests for `sftp`.

use gpui::{Modifiers, TestAppContext, point, px, size};

use super::transfers::self_copy_error;
use super::*;

fn n(v: &[&str]) -> Vec<String> {
    v.iter().map(ToString::to_string).collect()
}

#[test]
fn refuses_same_folder_and_self_nesting() {
    assert!(self_copy_error(true, "/a/b", "/a/b", &n(&["f"])).is_some());
    assert!(self_copy_error(true, "/a/b/", "/a/b", &n(&["f"])).is_some());
    assert!(self_copy_error(true, "/a", "/a/dir", &n(&["dir"])).is_some());
    assert!(self_copy_error(true, "/a", "/a/dir/deeper", &n(&["dir"])).is_some());
}

#[test]
fn allows_normal_copies() {
    assert!(self_copy_error(true, "/a", "/b", &n(&["x"])).is_none());
    // Prefix of the name but a different folder.
    assert!(self_copy_error(true, "/a", "/a/dirty", &n(&["dir"])).is_none());
    // Different file systems can use identical paths.
    assert!(self_copy_error(false, "/a", "/a", &n(&["x"])).is_none());
}

#[cfg(windows)]
#[test]
fn refuses_self_nesting_with_windows_paths() {
    assert!(self_copy_error(true, r"C:\a", r"C:\a", &n(&["f"])).is_some());
    assert!(self_copy_error(true, r"C:\a", r"C:\a\dir", &n(&["dir"])).is_some());
    assert!(self_copy_error(true, r"C:\a", r"C:\b", &n(&["dir"])).is_none());
}

#[gpui::test]
fn keys_and_clicks_reach_the_right_pane(cx: &mut TestAppContext) {
    let (view, cx) =
        cx.add_window_view(|_, cx| SftpView::new(Source::Local, Source::Local, vec![], cx));
    cx.simulate_resize(size(px(1000.), px(600.)));
    let pane = |cx: &mut gpui::VisualTestContext, side: Side| {
        view.read_with(cx, |v, _| v.pane(side).focus.clone())
    };
    let (left, right) = (pane(cx, Side::Left), pane(cx, Side::Right));
    cx.update(|w, cx| w.focus(&left, cx));
    cx.run_until_parked();
    // The arrows switch panes.
    cx.simulate_keystrokes("right");
    assert!(cx.update(|w, _| right.is_focused(w)));
    cx.simulate_keystrokes("left");
    assert!(cx.update(|w, _| left.is_focused(w)));
    // From a button in the pane, the keys are not for the file list.
    cx.update(Window::focus_next);
    let on_button = |cx: &mut gpui::VisualTestContext| {
        cx.update(|w, cx| !left.is_focused(w) && left.contains_focused(w, cx))
    };
    assert!(on_button(cx));
    cx.simulate_keystrokes("right delete");
    assert!(on_button(cx));
    assert!(view.read_with(cx, |v, _| v.prompt.is_none()));
    // A click on a button in the other pane makes that pane the one keys go to.
    cx.update(|w, cx| w.focus(&right, cx));
    cx.simulate_click(point(px(20.), px(11.)), Modifiers::default());
    assert!(cx.update(|w, _| left.is_focused(w)));
}
