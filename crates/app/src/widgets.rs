//! Small building blocks shared by every view: buttons, icons, tooltips, checkboxes, plus
//! the text input and animations in the submodules.

pub mod anim;
pub mod input;

use gpui::{
    AnyView, App, Context, Div, ElementId, IntoElement, KeyDownEvent, MouseButton, Render,
    SharedString, Stateful, Window, div, prelude::*, px, svg,
};

use crate::theme::*;

struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .mt_1()
            .px_2()
            .py_1()
            .rounded(px(2.))
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .text_xs()
            .text_color(c(TEXT))
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
        .bg(c(ELEVATED))
        .border_1()
        .border_color(c(BORDER))
        .text_color(c(TEXT))
        .font_family(ui())
        .text_size(px(12.))
        .child(label)
}

/// The text a key press types. gpui on Windows reports the space bar with none.
pub fn typed<'a>(key: &str, key_char: Option<&'a str>) -> Option<&'a str> {
    key_char.or((key == "space").then_some(" "))
}

/// Make a clickable element reachable with Tab and pressable with Enter or Space, with a
/// ring while it has keyboard focus. Callers without a border override `focus` with a
/// background instead.
pub fn keyboard(el: Stateful<Div>) -> Stateful<Div> {
    el.tab_index(0)
        .focus(|s| s.border_color(c(TEXT)))
        // A mouse click leaves focus where it was (the terminal keeps typing), like macOS
        // buttons, and the ring shows only for the keyboard.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        // gpui clicks on key up; stop the key down from also reaching the dialog's
        // Enter (submit) while Cancel has focus.
        .on_key_down(|ev: &KeyDownEvent, _, cx| {
            let k = &ev.keystroke;
            if matches!(k.key.as_str(), "enter" | "space") && !k.modifiers.modified() {
                cx.stop_propagation();
            }
        })
}

pub fn icon(name: &str, size: f32, color: u32) -> gpui::Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .flex_none()
        .text_color(c(color))
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
        .hover(|s| s.bg(c(HOVER)))
        .active(|s| s.bg(c(ACTIVE)))
        .tooltip(tip(tooltip))
        .child(icon(name, 13., color))
}

fn base(id: impl Into<ElementId>, text: &str, bg: u32, fg: u32, hover: u32) -> Stateful<Div> {
    keyboard(div().id(id))
        .px_3()
        .h(px(26.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .border_1()
        .border_color(c(BORDER))
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
    base(id, text, ACTIVE, TEXT, 0x323235)
}

pub fn primary_button(id: impl Into<ElementId>, text: &str) -> Stateful<Div> {
    base(id, text, accent(), 0xffffff, accent_text())
}

pub fn label(text: &str) -> Div {
    div().text_xs().text_color(c(MUTED)).child(text.to_string())
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
        .hover(|s| s.bg(c(HOVER)))
        .focus(|s| s.bg(c(HOVER)))
        .child(
            div()
                .size(px(14.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_2()
                .border_color(c(if checked { accent() } else { BORDER }))
                .bg(c(if checked { accent() } else { ELEMENT }))
                .when(checked, |d| d.child(icon("check", 10., 0xffffff))),
        )
        .child(text.to_string())
}
