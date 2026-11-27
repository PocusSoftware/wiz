//! Tests for `sftp`.

use gpui::{Modifiers, TestAppContext, point, px, size};

use super::transfers::{group_by_folder, self_copy_error};
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

#[gpui::test]
fn the_line_between_the_panes_can_be_dragged(cx: &mut TestAppContext) {
    let (view, cx) =
        cx.add_window_view(|_, cx| SftpView::new(Source::Local, Source::Local, vec![], cx));
    cx.simulate_resize(size(px(1000.), px(600.)));
    cx.run_until_parked();
    let none = Modifiers::default();
    let at = |x: f32| point(px(x), px(300.));
    cx.simulate_mouse_down(at(500.), MouseButton::Left, none);
    for x in [480., 400., 300.] {
        cx.simulate_mouse_move(at(x), MouseButton::Left, none);
    }
    cx.simulate_mouse_up(at(300.), MouseButton::Left, none);
    let split = view.read_with(cx, |v, _| v.split);
    assert!((split - 0.3).abs() < 0.02, "{split}");
    // It stops before a pane gets too narrow to use.
    cx.simulate_mouse_down(at(300.), MouseButton::Left, none);
    for x in [280., 100., 0.] {
        cx.simulate_mouse_move(at(x), MouseButton::Left, none);
    }
    cx.simulate_mouse_up(at(0.), MouseButton::Left, none);
    assert_eq!(view.read_with(cx, |v, _| v.split), MIN_SPLIT);
    // A double click puts it back in the middle.
    cx.simulate_click(at(200.), none);
    cx.simulate_event(MouseDownEvent {
        position: at(200.),
        button: MouseButton::Left,
        modifiers: none,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: at(200.),
        button: MouseButton::Left,
        modifiers: none,
        click_count: 2,
    });
    assert_eq!(view.read_with(cx, |v, _| v.split), 0.5);
}

#[gpui::test]
fn mouse_buttons_go_back_and_forward_in_history(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let inner = dir.path().join("inner");
    std::fs::create_dir(&inner).unwrap();
    let (view, cx) =
        cx.add_window_view(|_, cx| SftpView::new(Source::Local, Source::Local, vec![], cx));
    cx.simulate_resize(size(px(1000.), px(600.)));
    let open = |cx: &mut gpui::VisualTestContext, path: &std::path::Path| {
        let path = path.to_string_lossy().into_owned();
        view.update(cx, |v, cx| v.go(Side::Left, path, Nav::New, cx));
        settle(cx, &view);
    };
    settle(cx, &view);
    open(cx, dir.path());
    open(cx, &inner);
    let start = view.read_with(cx, |v, _| v.left.history.last().cloned());
    cx.simulate_mouse_down(
        point(px(200.), px(300.)),
        MouseButton::Navigate(NavigationDirection::Back),
        Modifiers::default(),
    );
    settle(cx, &view);
    view.read_with(cx, |v, _| assert_eq!(Some(v.left.path.clone()), start));
    cx.simulate_mouse_down(
        point(px(200.), px(300.)),
        MouseButton::Navigate(NavigationDirection::Forward),
        Modifiers::default(),
    );
    settle(cx, &view);
    let inner = std::fs::canonicalize(&inner).unwrap();
    view.read_with(cx, |v, _| {
        assert_eq!(v.left.path, inner.to_string_lossy());
        assert!(v.left.forward.is_empty());
    });
    cx.simulate_mouse_down(
        point(px(200.), px(300.)),
        MouseButton::Navigate(NavigationDirection::Back),
        Modifiers::default(),
    );
    settle(cx, &view);
    view.read_with(cx, |v, _| assert_eq!(v.left.forward.len(), 1));
    open(cx, &inner);
    view.read_with(cx, |v, _| assert!(v.left.forward.is_empty()));
}

#[test]
fn dropped_paths_are_grouped_by_folder() {
    let p = |s: &str| PathBuf::from(s);
    let groups = group_by_folder(&[p("/a/x"), p("/b/y"), p("/a/z"), p("/")]);
    assert_eq!(
        groups,
        [
            ("/a".to_string(), n(&["x", "z"])),
            ("/b".to_string(), n(&["y"]))
        ]
    );
}

#[gpui::test]
fn files_dropped_from_another_app_are_copied_into_the_pane(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("one.txt"), "1").unwrap();
    std::fs::create_dir(src.path().join("dir")).unwrap();
    std::fs::write(src.path().join("dir/two.txt"), "2").unwrap();
    let (view, cx) =
        cx.add_window_view(|_, cx| SftpView::new(Source::Local, Source::Local, vec![], cx));
    cx.simulate_resize(size(px(1000.), px(600.)));
    settle(cx, &view);
    let dst_path = dst.path().to_string_lossy().into_owned();
    view.update(cx, |v, cx| v.go(Side::Right, dst_path, Nav::New, cx));
    settle(cx, &view);

    let at = point(px(750.), px(300.));
    let paths = ExternalPaths(
        [src.path().join("one.txt"), src.path().join("dir")]
            .into_iter()
            .collect(),
    );
    cx.simulate_event(gpui::FileDropEvent::Entered {
        position: at,
        paths,
    });
    cx.simulate_event(gpui::FileDropEvent::Submit { position: at });

    let copied = dst.path().join("dir/two.txt");
    for _ in 0..500 {
        cx.run_until_parked();
        if copied.exists() && dst.path().join("one.txt").exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("dropped files were not copied");
}

fn settle(cx: &mut gpui::VisualTestContext, view: &Entity<SftpView>) {
    for _ in 0..500 {
        cx.run_until_parked();
        if view.read_with(cx, |v, _| !v.left.loading && !v.right.loading) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("listing never finished");
}
