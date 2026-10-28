//! Snippet picker: filter the saved snippets and pick one to paste into the terminal in front.

#[cfg(test)]
mod tests;

use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    SharedString, Window, div, prelude::*, px,
};
use ssh_core::config::{Snippet, Snippets};

use crate::privacy;
use crate::theme::*;
use crate::widgets::input::{InputEvent, TextInput};
use crate::widgets::{icon_button, keyboard};

pub enum SnippetEvent {
    /// The command to paste (without Enter, so it can be read first).
    Paste(String),
    Cancelled,
}

impl EventEmitter<SnippetEvent> for SnippetPicker {}

pub struct SnippetPicker {
    items: Vec<Snippet>,
    filter: Entity<TextInput>,
    /// Row highlighted among the matches.
    selected: usize,
    error: Option<SharedString>,
    focus: FocusHandle,
}

/// Indices of the snippets whose name or command contains `query`, ignoring case.
fn matching(items: &[Snippet], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            query.is_empty()
                || s.name.to_lowercase().contains(&query)
                || s.command.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}

impl SnippetPicker {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (items, error) = match Snippets::default_path().and_then(|p| Snippets::load(&p)) {
            Ok(s) => (s.items, None),
            Err(e) => (
                Vec::new(),
                Some(format!("Could not read snippets: {e}").into()),
            ),
        };
        let filter = cx.new(|cx| TextInput::new("Search snippets", false, cx));
        cx.subscribe(&filter, |t, _, _: &InputEvent, cx| {
            t.selected = 0;
            cx.notify();
        })
        .detach();
        Self {
            items,
            filter,
            selected: 0,
            error,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &gpui::App) {
        window.focus(&self.filter.focus_handle(cx));
    }

    fn matches(&self, cx: &gpui::App) -> Vec<usize> {
        matching(&self.items, self.filter.read(cx).value())
    }

    fn paste(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(&i) = self.matches(cx).get(row) {
            cx.emit(SnippetEvent::Paste(self.items[i].command.clone()));
        }
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        let last = self.matches(cx).len().saturating_sub(1);
        self.selected = if down {
            (self.selected + 1).min(last)
        } else {
            self.selected.saturating_sub(1)
        };
        cx.notify();
    }
}

impl Focusable for SnippetPicker {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SnippetPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let matches = self.matches(cx);
        let selected = self.selected.min(matches.len().saturating_sub(1));
        let rows = matches.iter().enumerate().map(|(row, &i)| {
            let s = &self.items[i];
            // Only the first line: the whole command is pasted anyway.
            let first = s.command.lines().next().unwrap_or_default();
            let more = s.command.lines().nth(1).is_some();
            keyboard(div().id(("snippet", row)))
                .px_2()
                .py_1()
                .rounded(px(3.))
                .border_1()
                .border_color(c(0x00000000))
                .cursor_pointer()
                .when(row == selected, |d| d.bg(c(ACTIVE)))
                .hover(|s| s.bg(c(HOVER)))
                .child(
                    div()
                        .truncate()
                        .child(privacy::redact(&s.name).into_owned()),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .font_family(mono())
                        .text_color(c(MUTED))
                        .child(format!(
                            "{}{}",
                            privacy::redact(first),
                            if more { " ..." } else { "" }
                        )),
                )
                .on_click(cx.listener(move |t, _, _, cx| t.paste(row, cx)))
        });
        div()
            .id("snippet-picker")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, _, cx| {
                match ev.keystroke.key.as_str() {
                    "enter" => t.paste(t.selected, cx),
                    "down" => t.step(true, cx),
                    "up" => t.step(false, cx),
                    "escape" => cx.emit(SnippetEvent::Cancelled),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .w(px(420.))
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
                    .child(div().text_sm().child("Snippets"))
                    .child(
                        icon_button("snip-close", "x", "Close (Esc)", MUTED)
                            .tab_index(1)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SnippetEvent::Cancelled))),
                    ),
            )
            .child(self.filter.clone())
            .child(
                div()
                    .id("snippet-list")
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows)
                    .when(matches.is_empty(), |d| {
                        d.child(
                            div()
                                .p_2()
                                .text_color(c(MUTED))
                                .child(if self.items.is_empty() {
                                    "No snippets yet. Add some under Settings, Snippets."
                                } else {
                                    "No snippets match."
                                }),
                        )
                    }),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(div().text_xs().text_color(c(RED)).child(e))
            })
            .child(div().text_xs().text_color(c(PLACEHOLDER)).child(
                "Enter pastes the command. It is not run until you press Enter in the terminal.",
            ))
    }
}
