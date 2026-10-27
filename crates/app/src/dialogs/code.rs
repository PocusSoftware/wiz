//! The questions a server asks during login beyond the saved credentials, usually a one-time
//! code. The text comes from the server, so it is cleaned up and masked before it is shown.

use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    SharedString, Window, div, prelude::*, px,
};
use ssh_core::session::Question;

use crate::theme::*;
use crate::widgets::input::TextInput;
use crate::widgets::{button, icon_button, label, primary_button};

pub enum CodeEvent {
    Answered(Vec<String>),
    Cancelled,
}

impl EventEmitter<CodeEvent> for CodeDialog {}

/// Longest server text shown, so a hostile server can't fill the window.
const MAX_TEXT: usize = 400;
const MAX_QUESTION: usize = 80;

/// Server text made safe to show: no control characters, bounded, masked.
fn clean(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .take(max)
        .collect();
    crate::privacy::redact(flat.trim()).into_owned()
}

pub struct CodeDialog {
    host: SharedString,
    text: SharedString,
    fields: Vec<(SharedString, Entity<TextInput>)>,
    focus: FocusHandle,
}

impl CodeDialog {
    pub fn new(host: &str, text: &str, questions: &[Question], cx: &mut Context<Self>) -> Self {
        let fields = questions
            .iter()
            .map(|q| {
                let label = match clean(&q.text, MAX_QUESTION) {
                    t if t.is_empty() => "Answer".to_string(),
                    t => t,
                };
                (label.into(), cx.new(|cx| TextInput::new("", q.hidden, cx)))
            })
            .collect();
        Self {
            host: crate::privacy::redact(host).into_owned().into(),
            text: clean(text, MAX_TEXT).into(),
            fields,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &gpui::App) {
        if let Some((_, input)) = self.fields.first() {
            window.focus(&input.focus_handle(cx));
        }
    }

    /// The server's text as shown.
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let answers = self
            .fields
            .iter()
            .map(|(_, input)| input.read(cx).value().to_string())
            .collect();
        // Nothing typed stays in the dialog once it is sent.
        for (_, input) in &self.fields {
            input.update(cx, |i, _| i.set_value(""));
        }
        cx.emit(CodeEvent::Answered(answers));
    }
}

impl Focusable for CodeDialog {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CodeDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("code")
            .track_focus(&self.focus)
            // Tab visits the fields first, then the buttons (tab index 1).
            .tab_group()
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, _, cx| {
                match ev.keystroke.key.as_str() {
                    "enter" => t.submit(cx),
                    "escape" => cx.emit(CodeEvent::Cancelled),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .w(px(360.))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(3.))
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .child(format!("{} asks for more", self.host)),
                    )
                    .child(
                        icon_button("code-close", "x", "Cancel login (Esc)", MUTED)
                            .tab_index(1)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(CodeEvent::Cancelled))),
                    ),
            )
            .when(!self.text.is_empty(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(c(MUTED))
                        .child(self.text.clone()),
                )
            })
            .children(self.fields.iter().map(|(name, input)| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(name))
                    .child(input.clone())
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("code-cancel", "Cancel")
                            .tab_index(1)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(CodeEvent::Cancelled))),
                    )
                    .child(
                        primary_button("code-ok", "Continue")
                            .tab_index(1)
                            .on_click(cx.listener(|t, _, _, cx| t.submit(cx))),
                    ),
            )
    }
}
