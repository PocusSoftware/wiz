//! Minimal single-line text input (no IME, no selection).

use gpui::{
    ClipboardItem, Context, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    SharedString, Window, div, prelude::*, px,
};

use crate::theme::*;

pub struct TextInput {
    text: String,
    /// Byte index, always on a char boundary.
    cursor: usize,
    masked: bool,
    placeholder: SharedString,
    focus: FocusHandle,
}

impl TextInput {
    pub fn new(placeholder: &str, masked: bool, cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            masked,
            placeholder: placeholder.to_string().into(),
            focus: cx.focus_handle(),
        }
    }

    pub fn value(&self) -> &str {
        &self.text
    }

    pub fn set_value(&mut self, v: &str) {
        self.text = v.to_string();
        self.cursor = self.text.len();
    }

    fn prev(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        match k.key.as_str() {
            "backspace" if m.platform => {
                self.text.drain(..self.cursor);
                self.cursor = 0;
            }
            "backspace" => {
                let p = self.prev();
                self.text.drain(p..self.cursor);
                self.cursor = p;
            }
            "delete" => {
                let n = self.next();
                self.text.drain(self.cursor..n);
            }
            "left" => self.cursor = self.prev(),
            "right" => self.cursor = self.next(),
            "home" => self.cursor = 0,
            "end" => self.cursor = self.text.len(),
            "v" if m.platform => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    let t = t.replace(['\n', '\r'], "");
                    self.text.insert_str(self.cursor, &t);
                    self.cursor += t.len();
                }
            }
            "a" if m.platform => self.cursor = self.text.len(),
            "c" if m.platform && !self.masked => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.text.clone()));
            }
            // Tab, enter, escape and shortcuts belong to the parent.
            "tab" | "enter" | "escape" => {
                cx.propagate();
                return;
            }
            _ => match &k.key_char {
                Some(ch) if !m.control && !m.platform && !ch.chars().any(char::is_control) => {
                    self.text.insert_str(self.cursor, ch);
                    self.cursor += ch.len();
                }
                _ => {
                    cx.propagate();
                    return;
                }
            },
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let shown = |s: &str| -> String {
            if self.masked {
                "•".repeat(s.chars().count())
            } else {
                s.to_string()
            }
        };
        let (before, after) = self.text.split_at(self.cursor);
        let caret = div()
            .w(px(1.))
            .h(px(13.))
            .bg(c(if focused { ACCENT_TEXT } else { ACTIVE }));
        let mut row = div()
            .flex()
            .items_center()
            .child(shown(before))
            .child(caret)
            .child(shown(after));
        if self.text.is_empty() && !focused {
            row = div()
                .text_color(c(PLACEHOLDER))
                .child(self.placeholder.clone());
        }
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, _| window.focus(&this.focus)),
            )
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(2.))
            .bg(c(ELEMENT))
            .border_2()
            .border_color(c(if focused { BORDER_FOCUSED } else { BORDER }))
            .text_color(c(TEXT))
            .child(row)
    }
}
