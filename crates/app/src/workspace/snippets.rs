//! The snippet picker: opens over a terminal and pastes the chosen command into it.

use super::*;
use crate::dialogs::snippets::SnippetEvent;

impl Workspace {
    pub(super) fn open_snippets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        if self.page != Page::Tab || !matches!(self.tabs[self.active].kind, TabKind::Terminal(_)) {
            self.set_status("Open a terminal to paste a snippet into.", false);
            cx.notify();
            return;
        }
        let picker = cx.new(SnippetPicker::new);
        cx.subscribe_in(&picker, window, |this, _, ev: &SnippetEvent, window, cx| {
            this.snippet_picker = None;
            this.select_tab(this.active, window, cx);
            if let SnippetEvent::Paste(command) = ev {
                this.paste_into_focused(command, window, cx);
            }
            cx.notify();
        })
        .detach();
        picker.update(cx, |d, cx| d.focus_first(window, cx));
        self.snippet_picker = Some(picker);
        cx.notify();
    }

    /// Paste into the pane of the terminal tab in front that has focus.
    fn paste_into_focused(&mut self, text: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(Tab {
            kind: TabKind::Terminal(t),
            ..
        }) = self.tabs.get(self.active)
        else {
            return;
        };
        let view = t.panes[Self::focused_pane(t, window, cx)].view.clone();
        view.update(cx, |v, cx| v.paste(text, cx));
    }
}
