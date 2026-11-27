//! Moving around: opening folders, history, the path field, selection and the keyboard.

use super::*;

impl SftpView {
    pub(super) fn go(&mut self, side: Side, path: String, how: Nav, cx: &mut Context<Self>) {
        let p = self.pane_mut(side);
        let source = p.source.clone();
        p.nav += 1;
        let nav = p.nav;
        p.loading = true;
        let prev = p.path.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = fetch(source, path).await;
            this.update(cx, |t, cx| {
                let p = t.pane_mut(side);
                if p.nav != nav {
                    return; // a newer navigation superseded this one
                }
                p.loading = false;
                match r {
                    Ok((canon, list)) => {
                        if !prev.is_empty() && prev != canon {
                            match how {
                                Nav::New => {
                                    p.history.push(prev);
                                    p.forward.clear();
                                }
                                Nav::Back => p.forward.push(prev),
                                Nav::Forward => p.history.push(prev),
                                Nav::Reload => {}
                            }
                        }
                        if p.path != canon {
                            p.filter = None;
                            p.query.clear();
                            p.deep = false;
                            p.stop_search();
                        }
                        p.path = canon.clone();
                        p.raw = list;
                        p.error = None;
                        p.selected.clear();
                        p.cursor = 0;
                        p.anchor = 0;
                        p.rebuild();
                        p.input.update(cx, |i, _| i.set_value(&canon));
                        p.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }
                    Err(e) => {
                        p.error = Some(e.to_string().into());
                        let cur = p.path.clone();
                        p.input.update(cx, |i, _| i.set_value(&cur));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn refresh(&mut self, side: Side, cx: &mut Context<Self>) {
        let path = self.pane(side).path.clone();
        if !path.is_empty() {
            self.go(side, path, Nav::Reload, cx);
        }
        if self.pane(side).found.is_some() {
            self.search(side, false, cx);
        }
    }

    pub(super) fn up(&mut self, side: Side, cx: &mut Context<Self>) {
        let p = self.pane(side);
        let parent = p.source.parent(&p.path);
        self.go(side, parent, Nav::New, cx);
    }

    pub(super) fn back(&mut self, side: Side, cx: &mut Context<Self>) {
        if let Some(prev) = self.pane_mut(side).history.pop() {
            self.go(side, prev, Nav::Back, cx);
        }
    }

    pub(super) fn forward(&mut self, side: Side, cx: &mut Context<Self>) {
        if let Some(next) = self.pane_mut(side).forward.pop() {
            self.go(side, next, Nav::Forward, cx);
        }
    }

    /// Enter a folder, or open a file in the local editor.
    pub(super) fn open_dir(&mut self, side: Side, ix: usize, cx: &mut Context<Self>) {
        let p = self.pane(side);
        let Some(e) = p.entries.get(ix) else {
            return;
        };
        let target = p.source.join(&p.path, &e.name);
        if e.is_dir {
            self.go(side, target, Nav::New, cx);
        } else {
            let (name, size) = (base_name(&e.name).to_string(), e.size);
            self.open_file(side, name, target, size, cx);
        }
    }

    pub(super) fn submit_path(&mut self, side: Side, cx: &mut Context<Self>) {
        let v = self.pane(side).input.read(cx).value().trim().to_string();
        if !v.is_empty() {
            self.go(side, v, Nav::New, cx);
        }
    }

    // ---------- selection ----------

    pub(super) fn select_row(&mut self, side: Side, ix: usize, shift: bool, toggle: bool) {
        let p = self.pane_mut(side);
        let Some(name) = p.entries.get(ix).map(|e| e.name.clone()) else {
            return;
        };
        if shift {
            let (a, b) = (p.anchor.min(ix), p.anchor.max(ix));
            p.selected = p.entries[a..=b].iter().map(|e| e.name.clone()).collect();
        } else if toggle {
            if !p.selected.remove(&name) {
                p.selected.insert(name);
            }
            p.anchor = ix;
        } else {
            p.selected.clear();
            p.selected.insert(name);
            p.anchor = ix;
        }
        p.cursor = ix;
    }

    fn move_cursor(&mut self, side: Side, delta: isize, extend: bool) {
        let n = self.pane(side).entries.len();
        if n == 0 {
            return;
        }
        let cur = self.pane(side).cursor as isize;
        let next = (cur + delta).clamp(0, n as isize - 1) as usize;
        self.select_row(side, next, extend, false);
        if extend {
            self.pane_mut(side).cursor = next;
        }
        self.ensure_visible(side, next);
    }

    /// Scroll just enough to show row `ix` (no jump when it is already visible).
    fn ensure_visible(&self, side: Side, ix: usize) {
        let h = &self.pane(side).scroll;
        let (top, rows) = {
            let st = h.0.borrow();
            let row_h = px(22.);
            let top = (-st.base_handle.offset().y / row_h).floor().max(0.0) as usize;
            let view = st.last_item_size.map(|s| s.item.height).unwrap_or(px(440.));
            (top, ((view / row_h) as usize).max(1))
        };
        if ix < top {
            h.scroll_to_item(ix, ScrollStrategy::Top);
        } else if ix >= top + rows {
            h.scroll_to_item(ix, ScrollStrategy::Bottom);
        }
    }

    pub(super) fn on_key(
        &mut self,
        side: Side,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keys from the pane's buttons and fields are not for the file list.
        if !self.pane(side).focus.is_focused(window) {
            cx.propagate();
            return;
        }
        let k = &ev.keystroke;
        let m = &k.modifiers;
        self.active = side;
        let cur = self.pane(side).cursor;
        match k.key.as_str() {
            "up" if m.secondary(cx) => self.up(side, cx),
            "down" if m.secondary(cx) => self.open_dir(side, cur, cx),
            "up" => self.move_cursor(side, -1, m.shift),
            "down" => self.move_cursor(side, 1, m.shift),
            "pageup" => self.move_cursor(side, -15, m.shift),
            "pagedown" => self.move_cursor(side, 15, m.shift),
            "home" => self.move_cursor(side, isize::MIN / 2, m.shift),
            "end" => self.move_cursor(side, isize::MAX / 2, m.shift),
            "enter" => self.open_dir(side, cur, cx),
            "backspace" if m.secondary(cx) => self.ask_delete(side, window, cx),
            "backspace" => self.up(side, cx),
            "delete" => self.ask_delete(side, window, cx),
            "left" if m.secondary(cx) => self.back(side, cx),
            "right" if m.secondary(cx) => self.forward(side, cx),
            // Tab goes on to the buttons, so the arrows switch panes.
            dir @ ("left" | "right") if !m.modified() => {
                let to = if dir == "left" {
                    Side::Left
                } else {
                    Side::Right
                };
                self.active = to;
                window.focus(&self.pane(to).focus, cx);
            }
            "escape" if self.menu.is_some() => self.menu = None,
            "escape" => {
                self.pane_mut(side).selected.clear();
            }
            "f" if m.secondary(cx) => self.open_filter(side, window, cx),
            "f2" => self.ask_rename(side, window, cx),
            "f5" => self.begin_transfer(side, cx),
            "r" if m.secondary(cx) => self.refresh(side, cx),
            "a" if m.secondary(cx) => {
                let p = self.pane_mut(side);
                p.selected = p.entries.iter().map(|e| e.name.clone()).collect();
            }
            "n" if m.secondary(cx) && m.shift => self.ask_new_folder(side, window, cx),
            _ => {
                // Type-ahead: jump to the first name starting with what was typed.
                match &k.key_char {
                    Some(ch)
                        if crate::platform::types_char(*m) && !ch.chars().any(char::is_control) =>
                    {
                        let p = self.pane_mut(side);
                        if p.typed.1.elapsed() > Duration::from_millis(900) {
                            p.typed.0.clear();
                        }
                        p.typed.0.push_str(&ch.to_lowercase());
                        p.typed.1 = Instant::now();
                        let q = p.typed.0.clone();
                        if let Some(ix) = p
                            .entries
                            .iter()
                            .position(|e| base_name(&e.name).to_lowercase().starts_with(&q))
                        {
                            self.select_row(side, ix, false, false);
                            self.pane(side)
                                .scroll
                                .scroll_to_item(ix, ScrollStrategy::Center);
                        }
                    }
                    _ => {
                        cx.propagate();
                        return;
                    }
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    // ---------- prompts ----------
}
