use gpui::{Div, ElementId, Stateful, div, prelude::*, px};

use crate::theme::*;

fn base(id: impl Into<ElementId>, text: &str, bg: u32, fg: u32, hover: u32) -> Stateful<Div> {
    div()
        .id(id)
        .px_2()
        .h(px(24.))
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
    base(id, text, SURFACE, TEXT, BORDER)
}

pub fn primary_button(id: impl Into<ElementId>, text: &str) -> Stateful<Div> {
    base(id, text, ACCENT, PANEL, 0xb4befe)
}

pub fn label(text: &str) -> Div {
    div().text_xs().text_color(c(DIM)).child(text.to_string())
}
