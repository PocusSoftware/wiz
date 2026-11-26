//! Small building blocks shared by every view: buttons, icons, tooltips, checkboxes, plus
//! the text input and animations in the submodules.

pub mod anim;
pub mod input;

use gpui::{
    Action, AnyView, App, Context, Div, ElementId, IntoElement, KeyDownEvent, MouseButton, Render,
    SharedString, Stateful, Window, actions, div, prelude::*, px, svg,
};

use crate::theme::*;

// The arrow keys on a focused control, handled by the window (which keeps focus inside an
// open dialog).
actions!(widgets, [FocusNext, FocusPrev]);

struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .mt_1()
            .px_2()
            .py_1()
            .rounded(px(2.))
            .bg(c(palette().elevated))
            .border_1()
            .border_color(c(palette().border))
            .text_xs()
            .text_color(c(palette().text))
            .child(self.0.clone())
    }
}

pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

/// What follows the mouse while a tab or split pane is dragged.
pub fn drag_chip(label: SharedString) -> Div {
    div()
        .px_2()
        .py(px(3.))
        .rounded(px(2.))
        .bg(c(palette().elevated))
        .border_1()
        .border_color(c(palette().border))
        .text_color(c(palette().text))
        .font_family(ui())
        .text_size(px(12.))
        .child(label)
}

/// The text a key press types. gpui on Windows reports the space bar with none.
pub fn typed<'a>(key: &str, key_char: Option<&'a str>) -> Option<&'a str> {
    key_char.or((key == "space").then_some(" "))
}

/// Make a clickable element reachable with Tab and the arrow keys and pressable with Enter
/// or Space, with a ring while it has keyboard focus. Callers without a border override
/// `focus` with a background instead.
pub fn keyboard(el: Stateful<Div>) -> Stateful<Div> {
    el.tab_index(0)
        .focus(|s| s.border_color(c(palette().text)))
        // A mouse click leaves focus where it was (the terminal keeps typing), like macOS
        // buttons, and the ring shows only for the keyboard.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        // gpui clicks on key up; stop the key down from also reaching the dialog's
        // Enter (submit) while Cancel has focus.
        .on_key_down(|ev: &KeyDownEvent, window, cx| {
            let k = &ev.keystroke;
            if k.modifiers.modified() {
                return;
            }
            match k.key.as_str() {
                "enter" | "space" => {}
                "left" | "up" => window.dispatch_action(FocusPrev.boxed_clone(), cx),
                "right" | "down" => window.dispatch_action(FocusNext.boxed_clone(), cx),
                _ => return,
            }
            cx.stop_propagation();
        })
}

pub fn icon(name: &str, size: f32, color: u32) -> gpui::Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(c(icon_color(color)))
}

/// Square icon button with a hover tooltip.
pub fn icon_button(
    id: impl Into<ElementId>,
    name: &str,
    tooltip: impl Into<SharedString>,
    color: u32,
) -> Stateful<Div> {
    keyboard(div().id(id))
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        // Room for the focus ring.
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .hover(|s| s.bg(c(palette().hover)))
        .active(|s| s.bg(c(palette().active)))
        .tooltip(tip(tooltip))
        .child(icon(name, 13., color))
}

pub fn surface<E: Styled>(el: E) -> E {
    el.rounded(px(3.))
        .bg(c(palette().elevated))
        .border_1()
        .border_color(c(palette().border))
        .shadow_lg()
}

pub fn dialog<E: Styled>(el: E, width: f32) -> E {
    surface(el.w(px(width)).p_3().flex().flex_col().gap_2())
}

pub fn dialog_header(title: impl IntoElement, close: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(div().text_sm().child(title))
        .child(close)
}

pub fn close_button(id: impl Into<ElementId>, tooltip: &'static str) -> Stateful<Div> {
    icon_button(id, "x", tooltip, palette().muted).tab_index(1)
}

fn base(id: impl Into<ElementId>, text: &str, bg: u32, fg: u32, hover: u32) -> Stateful<Div> {
    keyboard(div().id(id))
        .px_3()
        .h(px(26.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .border_1()
        .border_color(c(palette().border))
        .flex()
        .items_center()
        .rounded(px(2.))
        .bg(c(bg))
        .text_xs()
        .text_color(c(fg))
        .cursor_pointer()
        .hover(move |s| s.bg(c(hover)))
        .child(text.to_string())
}

pub fn button(id: impl Into<ElementId>, text: &str) -> Stateful<Div> {
    base(
        id,
        text,
        palette().active,
        palette().text,
        palette().active_hover,
    )
}

pub fn primary_button(id: impl Into<ElementId>, text: &str) -> Stateful<Div> {
    base(id, text, accent(), on_accent(), accent_text())
}

pub fn label(text: &str) -> Div {
    div()
        .text_xs()
        .text_color(c(palette().muted))
        .child(text.to_string())
}

/// Square checkbox with a label. The caller attaches `on_click`.
pub fn checkbox(id: impl Into<ElementId>, checked: bool, text: &str) -> Stateful<Div> {
    keyboard(div().id(id))
        .h(px(22.))
        .px(px(4.))
        .flex()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(c(palette().hover)))
        .focus(|s| s.bg(c(palette().hover)))
        .child(
            div()
                .size(px(14.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_2()
                .border_color(c(if checked { accent() } else { palette().border }))
                .bg(c(if checked { accent() } else { palette().element }))
                .when(checked, |d| d.child(icon("check", 10., on_accent()))),
        )
        .child(text.to_string())
}

pub fn keycap(text: String, lit: bool) -> Div {
    div()
        .min_w(px(24.))
        .h(px(24.))
        .px_1p5()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_b_2()
        .border_color(c(if lit { accent() } else { palette().border }))
        .bg(c(palette().element))
        .text_sm()
        .text_color(c(if lit { accent_text() } else { palette().text }))
        .child(text)
}
