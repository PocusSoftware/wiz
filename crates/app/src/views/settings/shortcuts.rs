//! The shortcuts section: one row per command, click or Enter to type a new keystroke.

use gpui::Keystroke;

use super::*;
use crate::shortcuts::{self, COMMANDS};

/// Commands by section, in the order shown. Every id in `COMMANDS` appears once.
pub(super) const GROUPS: &[(&str, &[&str])] = &[
    ("Splits", &["split", "split_menu", "next_pane", "prev_pane"]),
    (
        "Tabs",
        &[
            "tab_menu",
            "next_tab",
            "prev_tab",
            "focus_tabs",
            "close_tab",
        ],
    ),
    ("App", &["quick_connect", "new_host", "settings"]),
];

/// A shortcut as one cap per key: "⇧⌘D" gives ⇧ ⌘ D, "Ctrl+Shift+D" gives Ctrl Shift D.
pub(super) fn keycaps(label: &str) -> Vec<String> {
    if !label.contains('+') {
        return label.chars().map(String::from).collect();
    }
    let mut caps: Vec<String> = label
        .split('+')
        .filter(|k| !k.is_empty())
        .map(String::from)
        .collect();
    // "Ctrl++" is Ctrl and the plus key.
    if label.ends_with('+') {
        caps.push("+".into());
    }
    caps
}

fn keycap(text: String, lit: bool) -> gpui::Div {
    div()
        .min_w(px(24.))
        .h(px(24.))
        .px_1p5()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_b_2()
        .border_color(c(if lit { accent() } else { BORDER }))
        .bg(c(ELEMENT))
        .text_sm()
        .text_color(c(if lit { accent_text() } else { TEXT }))
        .child(text)
}

impl SettingsView {
    pub(super) fn shortcut_rows(&self, cx: &mut Context<Self>) -> gpui::Div {
        let saved = prefs::get().shortcuts();
        let query = self.shortcut_search.read(cx).value().trim().to_lowercase();
        let matches = |id: &str, name: &str| {
            query.is_empty()
                || name.to_lowercase().contains(&query)
                || shortcuts::label(id).to_lowercase().contains(&query)
        };
        let mut any = false;
        let sections = GROUPS.iter().filter_map(|&(group, ids)| {
            let rows: Vec<_> = ids
                .iter()
                .filter_map(|&id| COMMANDS.iter().position(|c| c.0 == id))
                .filter(|&i| matches(COMMANDS[i].0, COMMANDS[i].1))
                .collect();
            any |= !rows.is_empty();
            (!rows.is_empty()).then(|| {
                let last = rows.len() - 1;
                let items = rows.into_iter().enumerate().map(|(n, i)| {
                    let (id, name) = COMMANDS[i];
                    self.shortcut_row(i, id, name, saved.contains_key(id), n < last, cx)
                });
                div().child(Self::group(group)).child(
                    div()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(c(BORDER_VARIANT))
                        .bg(c(PANEL))
                        .overflow_hidden()
                        .children(items),
                )
            })
        });
        let sections: Vec<_> = sections.collect();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(c(PLACEHOLDER))
                    .child("Click a shortcut, then press the new keys. Escape cancels."),
            )
            .child(div().py_2().child(self.shortcut_search.clone()))
            .children(sections)
            .when(!any, |d| {
                d.child(
                    div()
                        .py_2()
                        .text_color(c(MUTED))
                        .child("No shortcuts match."),
                )
            })
            .when_some(self.shortcut_note.clone(), |d, n| {
                d.child(div().mt_2().text_color(c(RED)).child(n))
            })
    }

    /// One command: its name, then the keycaps (click to rebind) and Reset once changed.
    fn shortcut_row(
        &self,
        i: usize,
        id: &'static str,
        name: &str,
        changed: bool,
        divider: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let recording = self.recording.as_ref().is_some_and(|(r, _)| *r == id);
        let caps = if recording {
            div()
                .flex()
                .child(keycap("Type the shortcut".into(), true).px_2())
        } else {
            div().flex().gap_1().children(
                keycaps(&shortcuts::label(id))
                    .into_iter()
                    .map(|k| keycap(k, false)),
            )
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .hover(|s| s.bg(c(HOVER)))
            .when(divider, |d| d.border_b_1().border_color(c(BORDER_VARIANT)))
            .child(div().flex_1().min_w_0().child(name.to_string()))
            .when(changed, |d| {
                d.child(
                    keyboard(div().id(("sc-reset", i)))
                        .px_1()
                        .border_1()
                        .border_color(c(0x00000000))
                        .rounded(px(3.))
                        .text_xs()
                        .text_color(c(MUTED))
                        .cursor_pointer()
                        .hover(|s| s.text_color(c(TEXT)))
                        .child("Reset")
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.apply(cx, |s| {
                                if let Some(m) = &mut s.shortcuts {
                                    m.remove(id);
                                }
                            })
                        })),
                )
            })
            .child(
                keyboard(div().id(("sc", i)))
                    .p(px(2.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(c(0x00000000))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(ACTIVE)))
                    .child(caps)
                    .on_click(cx.listener(move |t, _, _, cx| t.record_for(id, cx))),
            )
    }

    /// Wait for the next keystroke to become the shortcut of command `id`. The keystroke is
    /// swallowed meanwhile, so typing a shortcut that is already in use does not run it.
    fn record_for(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let swallow = cx.intercept_keystrokes(move |ev, _, cx| {
            this.update(cx, |t, cx| t.record(&ev.keystroke, cx)).ok();
            cx.stop_propagation();
        });
        self.recording = Some((id, swallow));
        self.shortcut_note = None;
        cx.notify();
    }

    fn record(&mut self, k: &Keystroke, cx: &mut Context<Self>) {
        let Some((id, _)) = &self.recording else {
            return;
        };
        let id = *id;
        let m = k.modifiers;
        if matches!(
            k.key.as_str(),
            "shift" | "control" | "alt" | "platform" | "function"
        ) {
            return;
        }
        if k.key == "escape" && !m.modified() {
            self.recording = None;
            cx.notify();
            return;
        }
        if !(m.platform || m.control || m.alt) {
            self.shortcut_note = Some(
                format!(
                    "Hold {} with the key, or Escape to cancel.",
                    crate::platform::MODIFIER_HINT
                )
                .into(),
            );
            cx.notify();
            return;
        }
        let keys = k.unparse();
        if let Some(other) = shortcuts::taken_by(&keys, id) {
            self.shortcut_note = Some(
                format!(
                    "{} is already used by {other}.",
                    crate::platform::format_keys(&keys)
                )
                .into(),
            );
            cx.notify();
            return;
        }
        self.recording = None;
        self.apply(cx, move |s| {
            s.shortcuts
                .get_or_insert_default()
                .insert(id.to_string(), keys);
        });
    }
}
