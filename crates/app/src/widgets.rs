//! Small building blocks shared by every view: buttons, icons, tooltips, checkboxes, plus
//! the text input, animations and window dragging in the submodules.

pub mod anim;
pub mod input;
pub mod window_drag;

use gpui::{
    AnyView, App, Context, Div, ElementId, IntoElement, Render, SharedString, Stateful, Window,
    div, prelude::*, px, svg,
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

pub fn tip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tip(text.into())).into()
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
    tooltip: &'static str,
    color: u32,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .cursor_pointer()
        .hover(|s| s.bg(c(HOVER)))
        .active(|s| s.bg(c(ACTIVE)))
        .tooltip(tip(tooltip))
        .child(icon(name, 13., color))
}

fn base(id: impl Into<ElementId>, text: &str, bg: u32, fg: u32, hover: u32) -> Stateful<Div> {
    div()
        .id(id)
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
    base(id, text, ACCENT, 0xffffff, 0xf87c49)
}

pub fn label(text: &str) -> Div {
    div().text_xs().text_color(c(MUTED)).child(text.to_string())
}

/// Square checkbox with a label. The caller attaches `on_click`.
pub fn checkbox(id: impl Into<ElementId>, checked: bool, text: &str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(22.))
        .px(px(4.))
        .flex()
        .items_center()
        .gap(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(c(HOVER)))
        .child(
            div()
                .size(px(14.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_2()
                .border_color(c(if checked { ACCENT } else { BORDER }))
                .bg(c(if checked { ACCENT } else { ELEMENT }))
                .when(checked, |d| d.child(icon("check", 10., 0xffffff))),
        )
        .child(text.to_string())
}
