//! The shortcuts section: one row per command, click or Enter to type a new keystroke.

use gpui::Keystroke;

use super::*;
use crate::shortcuts::{self, COMMANDS};

impl SettingsView {
    pub(super) fn shortcut_rows(&self, cx: &mut Context<Self>) -> gpui::Div {
        let saved = prefs::get().shortcuts();
        let rows = COMMANDS.iter().enumerate().map(|(i, &(id, name))| {
            let recording = self.recording.as_ref().is_some_and(|(r, _)| *r == id);
            let text = if recording {
                "Type the shortcut...".to_string()
            } else {
                shortcuts::label(id)
            };
            let pick = if recording {
                primary_button(("sc", i), &text)
            } else {
                button(("sc", i), &text)
            };
            Self::row(
                name,
                "",
                div()
                    .flex()
                    .gap_1()
                    .child(pick.on_click(cx.listener(move |t, _, _, cx| t.record_for(id, cx))))
                    .when(saved.contains_key(id), |d| {
                        d.child(button(("sc-reset", i), "Reset").on_click(cx.listener(
                            move |t, _, _, cx| {
                                t.apply(cx, |s| {
                                    if let Some(m) = &mut s.shortcuts {
                                        m.remove(id);
                                    }
                                })
                            },
                        )))
                    }),
            )
        });
        div()
            .flex()
            .flex_col()
            .children(rows)
            .when_some(self.shortcut_note.clone(), |d, n| {
                d.child(div().mt_1().text_color(c(RED)).child(n))
            })
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
