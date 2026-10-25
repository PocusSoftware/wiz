//! Keyboard focus: Tab between controls (kept inside an open dialog or menu), and the arrow
//! keys, Delete and the menu key on the tab strip.

use gpui::{KeyDownEvent, Point};

use super::*;

/// Upper bound on Tab steps while looking for a control inside the open dialog.
const MAX_STEPS: usize = 500;

impl Workspace {
    /// A dialog or the tab menu is open, so Tab stays inside it.
    pub(super) fn modal_open(&self) -> bool {
        self.menu_open()
            || !self.trust.is_empty()
            || self.form.is_some()
            || self.quick_form.is_some()
            || self.unlock.is_some()
            || self.confirm.is_some()
            || self.confirm_delete.is_some()
            || self.quit_notice.is_some()
    }

    /// Tab and Shift+Tab: focus the next or previous control. With a dialog open, controls
    /// behind it are skipped.
    pub(super) fn move_focus(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let modal = self.modal_open();
        for _ in 0..MAX_STEPS {
            if back {
                window.focus_prev();
            } else {
                window.focus_next();
            }
            if !modal || self.modal_focus.contains_focused(window, cx) {
                break;
            }
        }
        cx.notify();
    }

    /// Focus a place on the tab strip: 0 is the hosts tab, then the tabs in order.
    pub(super) fn focus_strip(&mut self, pos: usize, window: &mut Window, cx: &mut Context<Self>) {
        match pos.checked_sub(1) {
            None => window.focus(&self.home_focus),
            Some(ix) => {
                let Some(tab) = self.tabs.get(ix) else {
                    return;
                };
                window.focus(&tab.focus);
                self.tab_scroll.scroll_to_item(ix);
            }
        }
        cx.notify();
    }

    /// The Focus Tabs shortcut: the way out of the terminal and file panes, which keep Tab.
    pub(super) fn focus_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        let pos = match self.page {
            Page::Tab => self.active + 1,
            _ => 0,
        };
        self.focus_strip(pos, window, cx);
    }

    /// Keys on a focused place of the tab strip (`pos` as in `focus_strip`). Enter and
    /// Space select, through the click handler. `menu_at` is where the tab menu opens.
    pub(super) fn strip_key(
        &mut self,
        pos: usize,
        menu_at: Point<Pixels>,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = pos
            .checked_sub(1)
            .and_then(|ix| self.tabs.get(ix).map(|t| (ix, t.id, t.pinned)));
        // Not for keys from the rename field inside the tab.
        let own = match tab {
            Some((ix, ..)) => &self.tabs[ix].focus,
            None => &self.home_focus,
        };
        if !own.is_focused(window) || self.modal_open() {
            return;
        }
        let k = &ev.keystroke;
        match (k.key.as_str(), k.modifiers.shift) {
            ("left", false) => self.focus_strip(pos.saturating_sub(1), window, cx),
            ("right", false) => self.focus_strip((pos + 1).min(self.tabs.len()), window, cx),
            ("home", _) => self.focus_strip(0, window, cx),
            ("end", _) => self.focus_strip(self.tabs.len(), window, cx),
            // Shift with an arrow moves the tab, like dragging it.
            (dir @ ("left" | "right"), true) => {
                let Some((ix, id, _)) = tab else {
                    return;
                };
                let to = if dir == "left" {
                    ix.saturating_sub(1)
                } else {
                    ix + 1
                };
                self.move_tab(id, to, cx);
                if let Some(now) = self.tabs.iter().position(|t| t.focus.is_focused(window)) {
                    self.tab_scroll.scroll_to_item(now);
                }
            }
            // Delete closes, like a middle click; pinned tabs close only from the menu.
            ("delete" | "backspace", false) => {
                let Some((ix, _, false)) = tab else {
                    return;
                };
                self.request_close(ix, None, window, cx);
            }
            ("f10", true) => {
                let Some((_, id, _)) = tab else {
                    return;
                };
                self.open_tab_menu(id, menu_at, window, cx);
                window.focus(&self.modal_focus);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}
