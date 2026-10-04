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
            .rounded_md()
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .text_xs()
            .text_color(c(TEXT))
            .child(self.0.clone())
    }
}

fn tip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
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
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(c(HOVER)))
        .active(|s| s.bg(c(ACTIVE)))
        .tooltip(tip(tooltip))
        .child(icon(name, 14., color))
}

fn base(id: impl Into<ElementId>, text: &str, bg: u32, fg: u32, hover: u32) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .h(px(26.))
        .flex()
        .items_center()
        .rounded_md()
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
