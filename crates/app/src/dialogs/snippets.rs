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
use crate::widgets::{button, close_button, dialog, icon_button, keyboard, label, primary_button};

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
    form: Option<Form>,
    name: Entity<TextInput>,
    command: Entity<TextInput>,
    focus: FocusHandle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Form {
    New,
    Edit(usize),
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
        let name = cx.new(|cx| TextInput::new("Name", false, cx));
        let command = cx.new(|cx| TextInput::new("Command", false, cx));
        Self {
            items,
            filter,
            selected: 0,
            error,
            form: None,
            name,
            command,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut gpui::App) {
        window.focus(&self.filter.focus_handle(cx), cx);
    }

    fn matches(&self, cx: &gpui::App) -> Vec<usize> {
        matching(&self.items, self.filter.read(cx).value())
    }

    fn paste(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(&i) = self.matches(cx).get(row) {
            cx.emit(SnippetEvent::Paste(self.items[i].command.clone()));
        }
    }

    fn open_form(&mut self, form: Form, window: &mut Window, cx: &mut Context<Self>) {
        let (name, command) = match form {
            Form::New => (String::new(), String::new()),
            Form::Edit(i) => {
                let Some(s) = self.items.get(i) else {
                    return;
                };
                (s.name.clone(), s.command.clone())
            }
        };
        self.name.update(cx, |i, _| i.set_value(&name));
        self.command.update(cx, |i, _| i.set_value(&command));
        self.form = Some(form);
        self.error = None;
        window.focus(&self.name.focus_handle(cx), cx);
        cx.notify();
    }

    fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        self.error = None;
        window.focus(&self.filter.focus_handle(cx), cx);
        cx.notify();
    }

    fn save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snippet = Snippet {
            name: self.name.read(cx).value().trim().to_string(),
            command: self.command.read(cx).value().trim().to_string(),
        };
        if let Err(e) = snippet.validate() {
            self.error = Some(e.into());
            cx.notify();
            return;
        }
        let mut items = self.items.clone();
        match self.form {
            Some(Form::Edit(i)) if i < items.len() => items[i] = snippet,
            _ => items.push(snippet),
        }
        self.store(items, window, cx);
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Form::Edit(i)) = self.form else {
            return;
        };
        let mut items = self.items.clone();
        if i < items.len() {
            items.remove(i);
        }
        self.store(items, window, cx);
    }

    fn store(&mut self, items: Vec<Snippet>, window: &mut Window, cx: &mut Context<Self>) {
        let list = Snippets { items };
        if let Err(e) = Snippets::default_path().and_then(|p| list.save(&p)) {
            self.error = Some(format!("Could not save snippets: {e}").into());
            cx.notify();
            return;
        }
        self.items = list.items;
        self.close_form(window, cx);
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
        let form = self.form;
        let selected = self.selected.min(matches.len().saturating_sub(1));
        let rows = matches.iter().enumerate().map(|(row, &i)| {
            let s = &self.items[i];
            let first = s.command.lines().next().unwrap_or_default();
            let multiline = s.command.contains('\n');
            keyboard(div().id(("snippet", row)))
                .group("snippet-row")
                .flex()
                .items_center()
                .gap_2()
                .pl_2()
                .pr_1()
                .py(px(5.))
                .rounded(px(3.))
                .border_1()
                .border_color(gpui::transparent_black())
                .cursor_pointer()
                .when(row == selected, |d| d.bg(c(palette().active)))
                .hover(|s| s.bg(c(palette().hover)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
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
                                .text_color(c(palette().placeholder))
                                .child(format!(
                                    "{}{}",
                                    privacy::redact(first),
                                    if multiline { " ..." } else { "" }
                                )),
                        ),
                )
                .when(!multiline, |d| {
                    d.child(
                        icon_button(("snip-edit", i), "pencil", "Edit", palette().muted)
                            .when(row != selected, |b| {
                                b.opacity(0.).group_hover("snippet-row", |s| s.opacity(1.))
                            })
                            .on_click(cx.listener(move |t, _, window, cx| {
                                cx.stop_propagation();
                                t.open_form(Form::Edit(i), window, cx);
                            })),
                    )
                })
                .on_click(cx.listener(move |t, _, _, cx| t.paste(row, cx)))
        });
        div()
            .id("snippet-picker")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, window, cx| {
                match (t.form.is_some(), ev.keystroke.key.as_str()) {
                    (true, "enter") => t.save_form(window, cx),
                    (true, "escape") => t.close_form(window, cx),
                    (false, "enter") => t.paste(t.selected, cx),
                    (false, "down") => t.step(true, cx),
                    (false, "up") => t.step(false, cx),
                    (false, "escape") => cx.emit(SnippetEvent::Cancelled),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .map(|d| dialog(d, 440.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().flex_1().text_sm().child(match form {
                        None => "Snippets",
                        Some(Form::New) => "New snippet",
                        Some(Form::Edit(_)) => "Edit snippet",
                    }))
                    .when(form.is_none(), |d| {
                        d.child(
                            icon_button("snip-new", "plus", "New snippet", palette().muted)
                                .tab_index(1)
                                .on_click(cx.listener(|t, _, window, cx| {
                                    t.open_form(Form::New, window, cx)
                                })),
                        )
                    })
                    .child(
                        close_button("snip-close", "Close (Esc)")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SnippetEvent::Cancelled))),
                    ),
            )
            .when(form.is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(label("Name"))
                        .child(self.name.clone()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(label("Command"))
                        .child(self.command.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(matches!(form, Some(Form::Edit(_))), |d| {
                            d.child(
                                button("snip-del", "Delete")
                                    .text_color(c(danger()))
                                    .on_click(cx.listener(|t, _, window, cx| t.delete(window, cx))),
                            )
                        })
                        .child(div().flex_1())
                        .child(
                            button("snip-cancel", "Cancel")
                                .on_click(cx.listener(|t, _, window, cx| t.close_form(window, cx))),
                        )
                        .child(
                            primary_button("snip-save", "Save")
                                .on_click(cx.listener(|t, _, window, cx| t.save_form(window, cx))),
                        ),
                )
            })
            .when(form.is_none(), |d| {
                d.child(self.filter.clone()).child(
                    div()
                        .id("snippet-list")
                        .max_h(px(320.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .children(rows)
                        .when(matches.is_empty(), |d| {
                            d.child(div().p_2().text_color(c(palette().muted)).child(
                                if self.items.is_empty() {
                                    "No snippets yet. Add one with +."
                                } else {
                                    "No snippets match."
                                },
                            ))
                        }),
                )
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(div().text_xs().text_color(c(palette().red)).child(e))
            })
            .when(form.is_none() && !matches.is_empty(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(c(palette().placeholder))
                        .child("Enter pastes without running it."),
                )
            })
    }
}
