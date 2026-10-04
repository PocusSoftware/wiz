//! Dragging the window by the custom title bar. GPUI's macOS backend has no hook for
//! this, so hand the current mouse-down event to AppKit's native window drag.

use gpui::{InteractiveElement, MouseButton, MouseDownEvent, Window};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSView};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn drag(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: the handle comes from the live window, and this runs on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    let Some(ns_window) = view.window() else {
        return;
    };
    if let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() {
        ns_window.performWindowDragWithEvent(&event);
    }
}

/// Make an element a title bar: drag moves the window, double-click zooms it.
pub fn title_bar<E: InteractiveElement>(el: E) -> E {
    el.on_mouse_down(MouseButton::Left, |ev: &MouseDownEvent, window, _| {
        if ev.click_count >= 2 {
            window.titlebar_double_click();
        } else {
            drag(window);
        }
    })
}
