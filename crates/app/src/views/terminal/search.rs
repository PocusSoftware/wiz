//! Find in the terminal (scrollback and screen): the bar, matches and jumping between them.

use super::*;

impl TerminalView {
    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_none() {
            let input = cx.new(|cx| TextInput::new("Find in scrollback", false, cx));
            cx.subscribe(&input, |t, _, _: &InputEvent, cx| t.research(cx, true))
                .detach();
            self.search = Some(Search {
                input,
                matches: vec![],
                cur: 0,
            });
        }
        if let Some(s) = &self.search {
            window.focus(&s.input.focus_handle(cx));
        }
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search = None;
        window.focus(&self.focus);
        cx.notify();
    }

    /// Recompute hits for the typed query. With `jump`, start at the newest one.
    pub(super) fn research(&mut self, cx: &mut Context<Self>, jump: bool) {
        let Some(s) = &self.search else {
            return;
        };
        let q = s.input.read(cx).value().to_string();
        let matches = self.term.find(&q);
        let s = self.search.as_mut().unwrap();
        s.cur = if jump {
            matches.len().saturating_sub(1)
        } else {
            s.cur.min(matches.len().saturating_sub(1))
        };
        s.matches = matches;
        if jump {
            self.show_match();
        }
        cx.notify();
    }

    pub(super) fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        let n = s.matches.len();
        if n == 0 {
            return;
        }
        s.cur = if forward {
            (s.cur + 1) % n
        } else {
            (s.cur + n - 1) % n
        };
        self.show_match();
        cx.notify();
    }

    /// Scroll so the current hit sits in the middle of the view.
    fn show_match(&mut self) {
        let Some(m) = self
            .search
            .as_ref()
            .and_then(|s| s.matches.get(s.cur))
            .copied()
        else {
            return;
        };
        let grid = self.term.term.grid();
        let want = (self.rows as i32 / 2 - m.line).clamp(0, grid.history_size() as i32);
        let delta = want - grid.display_offset() as i32;
        self.term.term.scroll_display(Scroll::Delta(delta));
    }

    pub(super) fn search_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let s = self.search.as_ref()?;
        let count = if s.input.read(cx).value().is_empty() {
            String::new()
        } else if s.matches.is_empty() {
            "No results".into()
        } else {
            format!("{}/{}", s.cur + 1, s.matches.len())
        };
        Some(
            div()
                .absolute()
                .top(px(8.))
                .right(px(16.))
                .w(px(360.))
                .h(px(30.))
                .px_1()
                .flex()
                .items_center()
                .gap_1()
                .bg(c(ELEVATED))
                .border_1()
                .border_color(c(BORDER))
                .rounded(px(2.))
                .text_xs()
                // Clicks here must not reach the terminal (it would take focus and select).
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    let m = &ev.keystroke.modifiers;
                    match ev.keystroke.key.as_str() {
                        "escape" => this.close_search(window, cx),
                        "enter" => this.step_match(!m.shift, cx),
                        "g" if m.platform => this.step_match(!m.shift, cx),
                        _ if m.platform => {
                            cx.propagate();
                            return;
                        }
                        _ => {}
                    }
                    // Keep search keys away from the terminal (they would reach the PTY).
                    cx.stop_propagation();
                }))
                .child(div().flex_1().min_w_0().child(s.input.clone()))
                .child(div().text_color(c(PLACEHOLDER)).child(count))
                .child(
                    icon_button("find-prev", "arrow-up", "Previous (⇧↵)", MUTED)
                        .on_click(cx.listener(|t, _, _, cx| t.step_match(false, cx))),
                )
                .child(
                    icon_button("find-next", "arrow-down", "Next (↵)", MUTED)
                        .on_click(cx.listener(|t, _, _, cx| t.step_match(true, cx))),
                )
                .child(
                    icon_button("find-close", "x", "Close (Esc)", MUTED)
                        .on_click(cx.listener(|t, _, w, cx| t.close_search(w, cx))),
                )
                .into_any_element(),
        )
    }
}
