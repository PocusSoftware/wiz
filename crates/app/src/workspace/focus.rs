//! Keyboard focus: Tab between controls (kept inside an open dialog or menu), and the arrow
//! keys, Delete and the menu key on the tab strip.

use gpui::{KeyDownEvent, Point};

use super::*;

/// Upper bound on Tab steps while looking for a control inside the open dialog.
const MAX_STEPS: usize = 500;

impl Workspace {
    pub(super) fn on_root_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Reaches here only when nothing focused took it: the shell and the file
        // panes keep Tab.
        let m = &ev.keystroke.modifiers;
        if ev.keystroke.key == "tab" && !(m.control || m.platform || m.alt) {
            self.move_focus(m.shift, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.renaming.is_some() {
            match ev.keystroke.key.as_str() {
                "enter" => self.finish_rename(true, window, cx),
                "escape" => self.finish_rename(false, window, cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if self.menu_open() {
            match ev.keystroke.key.as_str() {
                "escape" => self.close_tab_menu(window, cx),
                "up" => self.move_focus(true, window, cx),
                "down" => self.move_focus(false, window, cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if self.quit_notice.is_some() {
            if matches!(ev.keystroke.key.as_str(), "enter" | "escape") {
                self.dismiss_quit_notice(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        let key = ev.keystroke.key.as_str();
        match self.overlay {
            Some(Overlay::Help) => {
                if key == "escape" {
                    self.close_help(window, cx);
                    cx.stop_propagation();
                }
                return;
            }
            Some(Overlay::ConfirmDelete(_)) => {
                match key {
                    "enter" => self.answer_delete_host(true, window, cx),
                    "escape" => self.answer_delete_host(false, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
                return;
            }
            Some(Overlay::ConfirmClose(..)) => {
                match key {
                    "enter" => self.answer_close(true, window, cx),
                    "escape" => self.answer_close(false, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
                return;
            }
            _ => {}
        }
        // No Enter here: trusting a new key takes a deliberate press of the button.
        if !self.trust.is_empty() && ev.keystroke.key == "escape" {
            self.answer_trust(false, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.page == Page::Home && !self.modal_open() && self.home_key(ev, window, cx) {
            cx.stop_propagation();
        }
    }

    /// A dialog or the tab menu is open, so Tab stays inside it.
    pub(super) fn modal_open(&self) -> bool {
        self.menu_open()
            || !self.trust.is_empty()
            || self.code.is_some()
            || self.overlay.is_some()
            || self.quit_notice.is_some()
    }

    /// Tab and Shift+Tab: focus the next or previous control. With a dialog open, controls
    /// behind it are skipped.
    pub(super) fn move_focus(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let modal = self.modal_open();
        for _ in 0..MAX_STEPS {
            if back {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
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
            None => window.focus(&self.home_focus, cx),
            Some(ix) => {
                let Some(tab) = self.tabs.get(ix) else {
                    return;
                };
                window.focus(&tab.focus, cx);
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
                window.focus(&self.modal_focus, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}
