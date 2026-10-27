//! Tests for the container list.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, point, size};
use ssh_core::docker;

use super::*;

fn container(name: &str) -> Container {
    Container {
        id: format!("{name}-id"),
        name: name.into(),
        image: "nginx".into(),
        state: "running".into(),
        status: "Up 1 minute".into(),
        ports: String::new(),
    }
}

/// Mouse down and up at `pos` as the `count`th click in a row.
fn click(cx: &mut gpui::VisualTestContext, x: f32, y: f32, count: usize) {
    let position = point(px(x), px(y));
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: count,
    });
}

#[gpui::test]
fn double_click_opens_logs(cx: &mut TestAppContext) {
    // The view needs a Docker client; without one there is nothing to show.
    let Ok(local) = docker::Local::new() else {
        return;
    };
    cx.executor().allow_parking();
    let (view, cx) =
        cx.add_window_view(|_, cx| DockerView::new(Backend::Local(Arc::new(local)), false, cx));
    cx.simulate_resize(size(px(800.), px(400.)));
    // Let the first listing finish (it runs on the network runtime) so it can't replace
    // the rows below mid-test.
    for _ in 0..500 {
        cx.run_until_parked();
        if view.read_with(cx, |v, _| v.loaded) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    view.update(cx, |v, cx| {
        v.items = vec![container("web")];
        v.error = None;
        cx.notify();
    });
    let opened = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let opened = opened.clone();
        cx.subscribe(&view, move |_, ev: &DockerEvent, _| {
            if let DockerEvent::Logs { name, .. } = ev {
                opened.borrow_mut().push(name.clone());
            }
        })
        .detach();
    });
    cx.run_until_parked();

    // The row sits under the 26 px header.
    click(cx, 60., 39., 1);
    assert!(opened.borrow().is_empty());
    click(cx, 60., 39., 2);
    assert_eq!(*opened.borrow(), ["web"]);
}
