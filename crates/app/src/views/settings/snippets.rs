//! The snippets page: add, edit and delete the commands the snippet picker pastes.

use ssh_core::config::{Snippet, Snippets};

use super::*;

impl SettingsView {
    /// The saved snippets, or none (with the reason in `error`) when the file is unreadable.
    pub(super) fn load_snippets(&mut self) {
        match Snippets::default_path().and_then(|p| Snippets::load(&p)) {
            Ok(s) => self.snippets = s.items,
            Err(e) => self.error = Some(format!("Could not read snippets: {e}").into()),
        }
    }

    /// Write the list. On failure the change stays in memory and `error` says so.
    fn save_snippets(&mut self, cx: &mut Context<Self>) {
        let list = Snippets {
            items: self.snippets.clone(),
        };
        self.error = Snippets::default_path()
            .and_then(|p| list.save(&p))
            .err()
            .map(|e| format!("Could not save snippets: {e}").into());
        cx.notify();
    }

    fn clear_snippet_form(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self.snippet_name.update(cx, |i, _| i.set_value(""));
        self.snippet_command.update(cx, |i, _| i.set_value(""));
    }

    /// Add the typed snippet, or replace the one being edited.
    fn submit_snippet(&mut self, cx: &mut Context<Self>) {
        let snippet = Snippet {
            name: self.snippet_name.read(cx).value().trim().to_string(),
            command: self.snippet_command.read(cx).value().trim().to_string(),
        };
        if let Err(e) = snippet.validate() {
            self.error = Some(e.into());
            cx.notify();
            return;
        }
        match self.editing.and_then(|i| self.snippets.get_mut(i)) {
            Some(slot) => *slot = snippet,
            None => self.snippets.push(snippet),
        }
        self.clear_snippet_form(cx);
        self.save_snippets(cx);
    }

    fn edit_snippet(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(s) = self.snippets.get(ix) else {
            return;
        };
        let (name, command) = (s.name.clone(), s.command.clone());
        self.snippet_name.update(cx, |i, _| i.set_value(&name));
        self.snippet_command
            .update(cx, |i, _| i.set_value(&command));
        self.editing = Some(ix);
        self.error = None;
        cx.notify();
    }

    fn delete_snippet(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.snippets.len() {
            return;
        }
        self.snippets.remove(ix);
        self.clear_snippet_form(cx);
        self.save_snippets(cx);
    }

    pub(super) fn snippet_rows(&self, cx: &mut Context<Self>) -> gpui::Div {
        let editing = self.editing.is_some();
        let rows = self.snippets.iter().enumerate().map(|(ix, s)| {
            let first = s.command.lines().next().unwrap_or_default();
            // The command field is one line, so a hand-written multi-line command is
            // changed in the file instead.
            let multiline = s.command.contains('\n');
            div()
                .flex()
                .items_center()
                .gap_3()
                .py_1()
                .border_b_1()
                .border_color(c(BORDER_VARIANT))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
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
                                .text_color(c(PLACEHOLDER))
                                .child(format!(
                                    "{}{}",
                                    privacy::redact(first),
                                    if multiline { " ..." } else { "" }
                                )),
                        ),
                )
                .when(!multiline, |d| {
                    d.child(
                        button(("snip-edit", ix), "Edit")
                            .on_click(cx.listener(move |t, _, _, cx| t.edit_snippet(ix, cx))),
                    )
                })
                .child(
                    button(("snip-del", ix), "Delete")
                        .on_click(cx.listener(move |t, _, _, cx| t.delete_snippet(ix, cx))),
                )
        });
        div()
            .flex()
            .flex_col()
            .child(
                div().pt_2().text_xs().text_color(c(PLACEHOLDER)).child(format!(
                    "Pick one with {} while a terminal is in front. It is pasted without Enter. For several lines, edit snippets.toml.",
                    crate::shortcuts::label("snippets")
                )),
            )
            .child(self.text("Name", "Shown in the picker.", &self.snippet_name))
            .child(self.text("Command", "Pasted into the terminal.", &self.snippet_command))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .py_2()
                    .when(editing, |d| {
                        d.child(button("snip-cancel", "Cancel").on_click(cx.listener(
                            |t, _, _, cx| {
                                t.clear_snippet_form(cx);
                                cx.notify();
                            },
                        )))
                    })
                    .child(
                        primary_button("snip-save", if editing { "Save" } else { "Add" })
                            .on_click(cx.listener(|t, _, _, cx| t.submit_snippet(cx))),
                    ),
            )
            .child(Self::group("Saved"))
            .children(rows)
            .when(self.snippets.is_empty(), |d| {
                d.child(div().py_2().text_color(c(MUTED)).child("No snippets yet."))
            })
    }
}
