//! Container logs: opens at the newest entry, optional auto-scroll that refreshes every
//! two seconds, and text selection (drag, double-click word, triple-click line, select all and copy).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Bounds, ClipboardItem, Context, FocusHandle, Focusable, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollStrategy, ScrollWheelEvent,
    SharedString, UniformListScrollHandle, Window, canvas, div, font, prelude::*, px, uniform_list,
};
use ssh_core::docker::Backend;

use crate::runtime;
use crate::theme::*;
use crate::widgets::{checkbox, icon_button};

const ROW_H: f32 = 18.0;
const PAD_X: f32 = 12.0;
const FONT_SIZE: f32 = 12.0;

/// (line, column in chars)
type Pos = (usize, usize);

pub struct LogView {
    backend: Backend,
    id: String,
    lines: Vec<String>,
    auto: bool,
    error: Option<SharedString>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    anchor: Option<Pos>,
    head: Option<Pos>,
    dragging: bool,
    /// Left edge of the text area, captured during paint.
    list_left: Rc<Cell<Pixels>>,
    cell_w: Pixels,
}

/// Remove ANSI escape sequences (colors, cursor moves) that container logs often contain.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if ch == '\r' {
            continue;
        }
        out.push(if ch == '\t' { ' ' } else { ch });
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

impl LogView {
    pub fn new(backend: Backend, id: String, cx: &mut Context<Self>) -> Self {
        let this = Self {
            backend,
            id,
            lines: vec![],
            auto: true,
            error: None,
            scroll: UniformListScrollHandle::new(),
            // Reachable with Tab.
            focus: cx.focus_handle().tab_stop(true),
            anchor: None,
            head: None,
            dragging: false,
            list_left: Rc::new(Cell::new(px(0.))),
            cell_w: px(7.),
        };
        this.refresh(cx);
        cx.spawn(async move |this, cx| {
            loop {
                let secs = crate::prefs::get().log_refresh_secs().max(1) as u64;
                cx.background_executor()
                    .timer(Duration::from_secs(secs))
                    .await;
                if this
                    .update(cx, |t, cx| t.auto.then(|| t.refresh(cx)))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn refresh(&self, cx: &mut Context<Self>) {
        let (backend, id) = (self.backend.clone(), self.id.clone());
        let tail = crate::prefs::get().log_tail_lines().max(10);
        cx.spawn(async move |this, cx| {
            let r = runtime::spawn(async move { backend.logs(&id, tail).await }).await;
            this.update(cx, |t, cx| {
                match r {
                    Ok(text) => {
                        let lines: Vec<String> = text.lines().map(strip_ansi).collect();
                        t.error = None;
                        if lines != t.lines {
                            t.lines = lines;
                            // Selection positions may no longer exist.
                            if t.anchor.is_some_and(|(l, _)| l >= t.lines.len())
                                || t.head.is_some_and(|(l, _)| l >= t.lines.len())
                            {
                                t.anchor = None;
                                t.head = None;
                            }
                            if t.auto {
                                t.to_bottom();
                            }
                        }
                    }
                    Err(e) => t.error = Some(e.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn to_bottom(&self) {
        if let Some(last) = self.lines.len().checked_sub(1) {
            self.scroll.scroll_to_item(last, ScrollStrategy::Bottom);
        }
    }

    // ---------- selection ----------

    fn selection(&self) -> Option<(Pos, Pos)> {
        let (a, h) = (self.anchor?, self.head?);
        (a != h).then(|| if a <= h { (a, h) } else { (h, a) })
    }

    fn selected_text(&self) -> Option<String> {
        let ((l0, c0), (l1, c1)) = self.selection()?;
        let mut out = Vec::new();
        for (l, line) in self.lines.iter().enumerate().take(l1 + 1).skip(l0) {
            let chars: Vec<char> = line.chars().collect();
            let s = if l == l0 { c0.min(chars.len()) } else { 0 };
            let e = if l == l1 {
                c1.min(chars.len())
            } else {
                chars.len()
            };
            out.push(chars[s..e].iter().collect::<String>());
        }
        Some(out.join("\n"))
    }

    fn col_at(&self, line: usize, x: Pixels) -> usize {
        let len = self.lines.get(line).map_or(0, |l| l.chars().count());
        let rel = x - self.list_left.get() - px(PAD_X);
        ((rel / self.cell_w).round().max(0.0) as usize).min(len)
    }

    fn mouse_down(
        &mut self,
        line: usize,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        let col = self.col_at(line, ev.position.x);
        let chars: Vec<char> = self.lines[line].chars().collect();
        match ev.click_count {
            0 | 1 => {
                if ev.modifiers.shift && self.anchor.is_some() {
                    self.head = Some((line, col));
                } else {
                    self.anchor = Some((line, col));
                    self.head = Some((line, col));
                }
                self.dragging = true;
            }
            2 => {
                let at = col.min(chars.len().saturating_sub(1));
                let kind = chars.get(at).copied().map(is_word).unwrap_or(false);
                let mut s = at;
                while s > 0 && is_word(chars[s - 1]) == kind {
                    s -= 1;
                }
                let mut e = at;
                while e < chars.len() && is_word(chars[e]) == kind {
                    e += 1;
                }
                self.anchor = Some((line, s));
                self.head = Some((line, e));
                self.dragging = false;
            }
            _ => {
                self.anchor = Some((line, 0));
                self.head = Some((line, chars.len()));
                self.dragging = false;
            }
        }
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        if !k.modifiers.secondary() {
            cx.propagate();
            return;
        }
        match k.key.as_str() {
            "a" if !self.lines.is_empty() => {
                self.anchor = Some((0, 0));
                let last = self.lines.len() - 1;
                self.head = Some((last, self.lines[last].chars().count()));
            }
            "c" => {
                if let Some(t) = self.selected_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(t));
                }
            }
            _ => {
                cx.propagate();
                return;
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let sel = self.selection();
        range
            .filter_map(|ix| {
                self.lines
                    .get(ix)
                    .map(|l| (ix, crate::privacy::redact(l).into_owned()))
            })
            .map(|(ix, line)| {
                let chars: Vec<char> = line.chars().collect();
                // Selected column range on this line, if any.
                let hl = sel.and_then(|((l0, c0), (l1, c1))| {
                    if ix < l0 || ix > l1 {
                        return None;
                    }
                    let s = if ix == l0 { c0 } else { 0 };
                    let e = if ix == l1 { c1 } else { chars.len() + 1 };
                    Some((s.min(chars.len()), e.min(chars.len() + 1).max(s)))
                });
                let mut row = div()
                    .id(("log", ix))
                    .w_full()
                    .h(px(ROW_H))
                    .px(px(PAD_X))
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |t, ev: &MouseDownEvent, w, cx| {
                            t.mouse_down(ix, ev, w, cx)
                        }),
                    )
                    .on_mouse_move(cx.listener(move |t, ev: &MouseMoveEvent, _, cx| {
                        if t.dragging && ev.pressed_button == Some(MouseButton::Left) {
                            t.head = Some((ix, t.col_at(ix, ev.position.x)));
                            cx.notify();
                        }
                    }));
                row = match hl {
                    Some((s, e)) if e > s || chars.is_empty() => {
                        let end = e.min(chars.len());
                        row.child(chars[..s].iter().collect::<String>())
                            .child(
                                div()
                                    .bg(accent_alpha(0.4))
                                    // A selected line break shows as a trailing space.
                                    .child(format!(
                                        "{}{}",
                                        chars[s..end].iter().collect::<String>(),
                                        if e > chars.len() { " " } else { "" }
                                    )),
                            )
                            .child(chars[end..].iter().collect::<String>())
                    }
                    _ => row.child(line),
                };
                row
            })
            .collect()
    }
}

impl Focusable for LogView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ts = window.text_system();
        let fid = ts.resolve_font(&font(mono()));
        self.cell_w = ts
            .advance(fid, px(FONT_SIZE), 'm')
            .map(|s| s.width)
            .unwrap_or(px(7.));

        let left = self.list_left.clone();
        let recorder = canvas(
            move |b: Bounds<Pixels>, _, _| left.set(b.origin.x),
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let count = self.lines.len();

        div()
            .id("logs")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, _, cx| t.on_key(ev, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|t, _: &MouseUpEvent, _, _| t.dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|t, _: &MouseUpEvent, _, _| t.dragging = false),
            )
            // Scrolling up means "let me read": stop following.
            .on_scroll_wheel(cx.listener(|t, ev: &ScrollWheelEvent, _, cx| {
                if t.auto && ev.delta.pixel_delta(px(ROW_H)).y > px(0.) {
                    t.auto = false;
                    cx.notify();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(c(BORDER_VARIANT))
                    .child(
                        checkbox("autoscroll", self.auto, "Auto-scroll").on_click(cx.listener(
                            |t, _, _, cx| {
                                t.auto = !t.auto;
                                if t.auto {
                                    t.refresh(cx);
                                    t.to_bottom();
                                }
                                cx.notify();
                            },
                        )),
                    )
                    .child(div().text_color(c(PLACEHOLDER)).child(if self.auto {
                        "refreshing every 2s"
                    } else {
                        "paused"
                    }))
                    .child(div().flex_1())
                    .when_some(self.error.clone(), |d, e| {
                        d.child(div().text_color(c(RED)).child(e))
                    })
                    .child(
                        div()
                            .text_color(c(PLACEHOLDER))
                            .child(format!("{count} lines")),
                    )
                    .child(
                        icon_button("log-latest", "arrow-down", "Jump to latest", MUTED).on_click(
                            cx.listener(|t, _, _, cx| {
                                t.to_bottom();
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        icon_button("log-refresh", "refresh", "Refresh now", MUTED)
                            .on_click(cx.listener(|t, _, _, cx| t.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .font_family(mono())
                    .text_size(px(FONT_SIZE))
                    .child(recorder)
                    .child(
                        uniform_list(
                            "log-rows",
                            count,
                            cx.processor(|t: &mut LogView, range, _, cx| t.rows(range, cx)),
                        )
                        .track_scroll(self.scroll.clone())
                        .size_full(),
                    ),
            )
    }
}

#[cfg(test)]
mod tests;
