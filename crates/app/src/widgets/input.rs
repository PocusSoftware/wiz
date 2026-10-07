//! Single-line text input with a real selection (no IME).

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, Font, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render,
    SharedString, TextRun, Window, canvas, div, point, prelude::*, px, size,
};

use crate::privacy;
use crate::theme::*;

/// How the input behaves when "hide sensitive information" is on and it is not focused.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Hide {
    /// Always readable.
    Never,
    /// Mask known hostnames, IPs and usernames inside the text.
    Redact,
    /// Show only bullets.
    Full,
    /// Show only bullets while port masking is on.
    Port,
}

/// Emitted whenever the user changes the text.
pub enum InputEvent {
    Changed,
}

impl EventEmitter<InputEvent> for TextInput {}

pub struct TextInput {
    text: String,
    /// Byte indices, always on char boundaries. The selection is `anchor..cursor` (either order).
    cursor: usize,
    anchor: usize,
    masked: bool,
    hide_mode: Hide,
    placeholder: SharedString,
    focus: FocusHandle,
    /// Where the text is painted, captured during paint, for click positioning.
    text_bounds: Rc<Cell<Bounds<Pixels>>>,
    font: Font,
    font_size: Pixels,
    dragging: bool,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

impl TextInput {
    pub fn new(placeholder: &str, masked: bool, cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: 0,
            masked,
            hide_mode: Hide::Never,
            placeholder: placeholder.to_string().into(),
            focus: cx.focus_handle(),
            text_bounds: Rc::new(Cell::new(Bounds::default())),
            font: gpui::font(ui()),
            font_size: px(12.),
            dragging: false,
        }
    }

    pub fn set_hide_mode(&mut self, mode: Hide) {
        self.hide_mode = mode;
    }

    /// True when the text is currently shown masked because of the privacy setting.
    fn hidden_view(&self, focused: bool) -> bool {
        !focused
            && !self.text.is_empty()
            && match self.hide_mode {
                Hide::Never => false,
                Hide::Port => privacy::hides_ports(),
                Hide::Redact | Hide::Full => privacy::hide(),
            }
    }

    pub fn value(&self) -> &str {
        &self.text
    }

    pub fn set_value(&mut self, v: &str) {
        self.text = v.to_string();
        self.cursor = self.text.len();
        self.anchor = self.cursor;
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    fn prev(&self, from: usize) -> usize {
        self.text[..from]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next(&self, from: usize) -> usize {
        self.text[from..]
            .chars()
            .next()
            .map_or(from, |c| from + c.len_utf8())
    }

    fn word_start(&self, from: usize) -> usize {
        let mut i = from;
        // Skip separators first, then the word itself.
        while i > 0 && !is_word(self.text[..i].chars().next_back().unwrap()) {
            i = self.prev(i);
        }
        while i > 0 && is_word(self.text[..i].chars().next_back().unwrap()) {
            i = self.prev(i);
        }
        i
    }

    fn word_end(&self, from: usize) -> usize {
        let mut i = from;
        while i < self.text.len() && !is_word(self.text[i..].chars().next().unwrap()) {
            i = self.next(i);
        }
        while i < self.text.len() && is_word(self.text[i..].chars().next().unwrap()) {
            i = self.next(i);
        }
        i
    }

    /// Word (or run of separators) around byte index `i`, for double-click.
    fn word_range_at(&self, i: usize) -> Range<usize> {
        if self.text.is_empty() {
            return 0..0;
        }
        let i = i.min(self.text.len());
        let probe = self.text[i..]
            .chars()
            .next()
            .or_else(|| self.text[..i].chars().next_back());
        let kind = probe.map(is_word).unwrap_or(false);
        let mut s = i;
        while s > 0 && self.text[..s].chars().next_back().map(is_word) == Some(kind) {
            s = self.prev(s);
        }
        let mut e = i;
        while e < self.text.len() && self.text[e..].chars().next().map(is_word) == Some(kind) {
            e = self.next(e);
        }
        s..e
    }

    fn move_to(&mut self, idx: usize, extend: bool) {
        self.cursor = idx;
        if !extend {
            self.anchor = idx;
        }
    }

    fn insert(&mut self, s: &str) {
        let r = self.selection();
        self.text.replace_range(r.clone(), s);
        self.cursor = r.start + s.len();
        self.anchor = self.cursor;
    }

    fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        self.insert("");
        true
    }

    /// What is painted. Hidden views never show a caret or selection.
    fn display(&self, focused: bool) -> String {
        if self.masked {
            return "•".repeat(self.text.chars().count());
        }
        if self.hidden_view(focused) {
            return match self.hide_mode {
                Hide::Full | Hide::Port => privacy::mask(),
                _ => privacy::redact(&self.text).into_owned(),
            };
        }
        self.text.clone()
    }

    /// Byte index in `text` for a window x coordinate.
    fn index_for_x(&self, window: &Window, x: Pixels) -> usize {
        let shown = self.display(true);
        if shown.is_empty() {
            return 0;
        }
        let run = TextRun {
            len: shown.len(),
            font: self.font.clone(),
            color: c(TEXT),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line =
            window
                .text_system()
                .shape_line(shown.clone().into(), self.font_size, &[run], None);
        let local = (x - self.text_bounds.get().origin.x).max(px(0.));
        let di = line.closest_index_for_x(local);
        if self.masked {
            // Each bullet stands for one char of the real text.
            let n = shown[..di.min(shown.len())].chars().count();
            self.text
                .char_indices()
                .nth(n)
                .map_or(self.text.len(), |(i, _)| i)
        } else {
            di.min(self.text.len())
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let before = self.text.clone();
        let k = &ev.keystroke;
        let m = &k.modifiers;
        let shift = m.shift;
        match k.key.as_str() {
            "a" if m.secondary() => self.select_all(),
            "c" if m.secondary() => {
                if self.has_selection() && !self.masked {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.selected_text().to_string(),
                    ));
                }
            }
            "x" if m.secondary() => {
                if self.has_selection() && !self.masked {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.selected_text().to_string(),
                    ));
                    self.delete_selection();
                }
            }
            "v" if m.secondary() => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.insert(&t.replace(['\n', '\r'], ""));
                }
            }
            "backspace" if m.secondary() => {
                if !self.delete_selection() {
                    let end = self.cursor;
                    self.text.drain(..end);
                    self.cursor = 0;
                    self.anchor = 0;
                }
            }
            "backspace" if m.alt => {
                if !self.delete_selection() {
                    let s = self.word_start(self.cursor);
                    self.text.drain(s..self.cursor);
                    self.cursor = s;
                    self.anchor = s;
                }
            }
            "backspace" => {
                if !self.delete_selection() {
                    let p = self.prev(self.cursor);
                    self.text.drain(p..self.cursor);
                    self.cursor = p;
                    self.anchor = p;
                }
            }
            "delete" => {
                if !self.delete_selection() {
                    let n = self.next(self.cursor);
                    self.text.drain(self.cursor..n);
                }
            }
            "left" if m.secondary() => self.move_to(0, shift),
            "right" if m.secondary() => self.move_to(self.text.len(), shift),
            "left" if m.alt => self.move_to(self.word_start(self.cursor), shift),
            "right" if m.alt => self.move_to(self.word_end(self.cursor), shift),
            "left" => {
                if self.has_selection() && !shift {
                    self.move_to(self.selection().start, false);
                } else {
                    self.move_to(self.prev(self.cursor), shift);
                }
            }
            "right" => {
                if self.has_selection() && !shift {
                    self.move_to(self.selection().end, false);
                } else {
                    self.move_to(self.next(self.cursor), shift);
                }
            }
            "home" => self.move_to(0, shift),
            "end" => self.move_to(self.text.len(), shift),
            // Tab, enter, escape and other shortcuts belong to the parent.
            "tab" | "enter" | "escape" => {
                cx.propagate();
                return;
            }
            _ => match super::typed(&k.key, k.key_char.as_deref()) {
                Some(ch)
                    if crate::platform::types_char(*m) && !ch.chars().any(char::is_control) =>
                {
                    self.insert(ch);
                }
                _ => {
                    cx.propagate();
                    return;
                }
            },
        }
        if self.text != before {
            cx.emit(InputEvent::Changed);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let was_focused = self.focus.is_focused(window);
        window.focus(&self.focus);
        if self.hidden_view(was_focused) {
            // The text was masked, so there is nothing to aim at: just reveal and edit.
            self.move_to(self.text.len(), false);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let idx = self.index_for_x(window, ev.position.x);
        match ev.click_count {
            0 | 1 => {
                self.move_to(idx, ev.modifiers.shift);
                self.dragging = true;
            }
            2 => {
                let r = self.word_range_at(idx);
                self.anchor = r.start;
                self.cursor = r.end;
                self.dragging = false;
            }
            _ => {
                self.anchor = 0;
                self.cursor = self.text.len();
                self.dragging = false;
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.dragging && ev.pressed_button == Some(MouseButton::Left) {
            let idx = self.index_for_x(window, ev.position.x);
            self.move_to(idx, true);
            cx.notify();
        }
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
        let style = window.text_style();
        self.font = style.font();
        self.font_size = style.font_size.to_pixels(window.rem_size());

        let hidden = self.hidden_view(focused);
        let shown = self.display(focused);
        // Byte offsets into `shown` (bullets are 3 bytes each).
        let to_shown = |byte: usize| -> usize {
            if self.masked {
                self.text[..byte].chars().count() * '•'.len_utf8()
            } else {
                byte
            }
        };
        // A hidden view paints masked text whose byte offsets differ from `text`, so it must
        // never be sliced with cursor or selection offsets.
        let (s, e, caret_at) = if hidden {
            (0, 0, 0)
        } else {
            let sel = self.selection();
            (
                to_shown(sel.start),
                to_shown(sel.end),
                to_shown(self.cursor),
            )
        };
        let caret = || {
            div().w(px(1.)).h(px(13.)).flex_none().bg(c(if focused {
                accent_text()
            } else {
                ACTIVE
            }))
        };

        let mut row = div().flex().items_center().whitespace_nowrap();
        if hidden {
            row = row.child(shown);
        } else if self.has_selection() {
            row = row.child(shown[..s].to_string());
            if caret_at == s && focused {
                row = row.child(caret());
            }
            row = row.child(div().bg(accent_alpha(0.4)).child(shown[s..e].to_string()));
            if caret_at == e && focused {
                row = row.child(caret());
            }
            row = row.child(shown[e..].to_string());
        } else {
            row = row
                .child(shown[..caret_at].to_string())
                .child(caret())
                .child(shown[caret_at..].to_string());
        }
        if self.text.is_empty() && !focused {
            row = div()
                .text_color(c(PLACEHOLDER))
                .child(self.placeholder.clone());
        }

        // Invisible element that records where the text starts, for click positioning.
        let bounds = self.text_bounds.clone();
        let recorder = canvas(
            move |b, _, _| {
                bounds.set(Bounds::new(
                    point(b.origin.x, b.origin.y),
                    size(b.size.width, b.size.height),
                ));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.dragging = false),
            )
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .overflow_hidden()
            .relative()
            .cursor_text()
            .rounded(px(2.))
            .bg(c(ELEMENT))
            .border_2()
            .border_color(if focused {
                accent_alpha(0.4)
            } else {
                c(BORDER)
            })
            .text_color(c(TEXT))
            .child(div().relative().child(recorder).child(row))
    }
}
