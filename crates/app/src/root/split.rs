//! Split terminals: up to four shells of one connection in a tab, laid out for the space.

use super::*;

impl TermTab {
    /// Switch to `layout`, sharing the space evenly whenever its shape changes.
    pub(super) fn set_layout(&mut self, layout: Vec<Vec<usize>>) {
        if layout == self.layout {
            return;
        }
        self.cols = vec![1.0 / layout.len() as f32; layout.len()];
        self.rows = layout
            .iter()
            .map(|col| vec![1.0 / col.len() as f32; col.len()])
            .collect();
        self.layout = layout;
    }
}

/// Most terminals in one tab.
const MAX_PANES: usize = 4;

/// Smallest usable pane (header included). Splitting is refused when another pane
/// would not get at least this much room.
const MIN_PANE_W: f32 = 320.;
const MIN_PANE_H: f32 = 150.;

/// Narrowest a pane may be dragged, as a share of the tab.
const MIN_PANE: f32 = 0.12;

/// Columns of pane indices for `n` panes in a `w` x `h` area, or `None` when they can't all
/// get a usable size. Two panes sit side by side, a third goes below the first, a fourth
/// below the second (a 2x2 grid), so existing panes never jump around. When the window is
/// too short for a grid but wide enough, panes go side by side instead (or stacked when it
/// is tall and narrow).
pub(super) fn layout(n: usize, w: f32, h: f32) -> Option<Vec<Vec<usize>>> {
    let cols_fit = |k: usize| w / k as f32 >= MIN_PANE_W;
    let rows_fit = |k: usize| h / k as f32 >= MIN_PANE_H;
    let side_by_side = || (0..n).map(|i| vec![i]).collect();
    let stacked = || vec![(0..n).collect()];
    match n {
        0 | 1 => Some(vec![vec![0]]),
        2 if cols_fit(2) => Some(side_by_side()),
        2 if rows_fit(2) => Some(stacked()),
        3 | 4 if cols_fit(2) && rows_fit(2) => Some(grid_layout(n)),
        3 | 4 if cols_fit(n) => Some(side_by_side()),
        3 | 4 if rows_fit(n) => Some(stacked()),
        _ => None,
    }
}

/// The grid arrangement, also used when a window shrinks below what fits: panes are never
/// dropped, they just get small.
pub(super) fn grid_layout(n: usize) -> Vec<Vec<usize>> {
    match n {
        0 | 1 => vec![vec![0]],
        2 => vec![vec![0], vec![1]],
        3 => vec![vec![0, 2], vec![1]],
        _ => vec![vec![0, 2], vec![1, 3]],
    }
}

impl Root {
    /// Lay split terminals out for the space they have now.
    pub(super) fn arrange_splits(&mut self, window: &Window) {
        let (w, h) = self.area(window);
        for tab in &mut self.tabs {
            if let TabKind::Terminal(t) = &mut tab.kind {
                let n = t.panes.len();
                t.set_layout(layout(n, w, h).unwrap_or_else(|| grid_layout(n)));
            }
        }
    }

    /// Index of the pane holding keyboard focus in terminal tab `t`.
    pub(super) fn focused_pane(t: &TermTab, window: &Window, cx: &App) -> usize {
        t.panes
            .iter()
            .position(|p| p.focus_handle(cx).contains_focused(window, cx))
            .unwrap_or(t.focused.min(t.panes.len().saturating_sub(1)))
    }

    /// Open another shell on the same server, beside the current one.
    pub(super) fn split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Tab {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let TabKind::Terminal(t) = &tab.kind else {
            return;
        };
        if t.panes.len() >= MAX_PANES {
            self.set_status(format!("A tab holds at most {MAX_PANES} terminals."), true);
            cx.notify();
            return;
        }
        let area = self.area(window);
        if layout(t.panes.len() + 1, area.0, area.1).is_none() {
            self.set_status(
                "The window is too small for another terminal. Make it bigger or close a pane.",
                true,
            );
            cx.notify();
            return;
        }
        if t.conn.is_closed() {
            self.set_status("The connection for this tab has closed.", true);
            cx.notify();
            return;
        }
        let (id, label, conn, pw) = (
            tab.id,
            tab.label.to_string(),
            t.conn.clone(),
            t.password.clone(),
        );
        self.spawn_shell(label, conn, pw, None, Some(id), window, cx);
    }

    /// Close one pane of a split terminal tab; the last pane closes the tab.
    pub(super) fn close_pane(
        &mut self,
        ix: usize,
        pane: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Tab {
            kind: TabKind::Terminal(t),
            ..
        }) = self.tabs.get_mut(ix)
        else {
            return;
        };
        if t.panes.len() <= 1 {
            self.close_tab(ix, window, cx);
            return;
        }
        if pane < t.panes.len() {
            t.panes.remove(pane);
            t.focused = pane.min(t.panes.len() - 1);
        }
        self.select_tab(ix, window, cx);
    }

    /// Size of the content area below the tab bar, from the last frame.
    pub(super) fn area(&self, window: &Window) -> (f32, f32) {
        let b = self.content_bounds.get();
        if b.size.width > px(0.) {
            (f32::from(b.size.width), f32::from(b.size.height))
        } else {
            let v = window.viewport_size();
            (f32::from(v.width), f32::from(v.height) - 36.)
        }
    }

    /// Move a split divider with the mouse.
    pub(super) fn drag_divider(
        &mut self,
        pos: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let (w, h) = self.area(window);
        let Some(drag) = self.pane_drag.as_mut() else {
            return;
        };
        let Some(Tab {
            kind: TabKind::Terminal(t),
            ..
        }) = self.tabs.get_mut(self.active)
        else {
            return;
        };
        let (sizes, now, span) = match drag.divider {
            Divider::Col(_) => (&mut t.cols, pos.x, w),
            Divider::Row { col, .. } => match t.rows.get_mut(col) {
                Some(r) => (r, pos.y, h),
                None => return,
            },
        };
        let a = match drag.divider {
            Divider::Col(n) | Divider::Row { n, .. } => n,
        };
        let b = a + 1;
        if b >= sizes.len() || span <= 0. {
            return;
        }
        let delta = f32::from(now - drag.last) / span;
        let pair = sizes[a] + sizes[b];
        let first = (sizes[a] + delta).clamp(MIN_PANE, pair - MIN_PANE);
        sizes[a] = first;
        sizes[b] = pair - first;
        drag.last = now;
        cx.notify();
    }

    /// A draggable split divider: vertical between columns, horizontal between rows.
    pub(super) fn divider(
        &self,
        which: Divider,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let dragging = self.pane_drag.as_ref().is_some_and(|d| d.divider == which);
        let line = c(if dragging { ACCENT } else { BORDER });
        let id = match which {
            Divider::Col(n) => ("divider", n),
            Divider::Row { col, n } => ("row-divider", col * 8 + n),
        };
        let d = div().id(id).flex_none().flex().hover(|s| s.bg(c(HOVER)));
        let d = match which {
            Divider::Col(_) => d
                .w(px(5.))
                .h_full()
                .justify_center()
                .cursor_col_resize()
                .child(div().w(px(1.)).h_full().bg(line)),
            Divider::Row { .. } => d
                .h(px(5.))
                .w_full()
                .flex_col()
                .justify_center()
                .cursor_row_resize()
                .child(div().h(px(1.)).w_full().bg(line)),
        };
        d.on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                this.pane_drag = Some(PaneDrag {
                    divider: which,
                    last: match which {
                        Divider::Col(_) => ev.position.x,
                        Divider::Row { .. } => ev.position.y,
                    },
                });
                cx.notify();
            }),
        )
    }

    /// One terminal of a split: a small header (title, close) above the terminal.
    pub(super) fn pane(
        &self,
        tab_ix: usize,
        i: usize,
        p: &Entity<TerminalView>,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let title = p
            .read(cx)
            .title()
            .filter(|t| !t.is_empty())
            .map(|t| privacy::redact(&t).into_owned())
            .unwrap_or_else(|| "shell".into());
        let pane = p.clone();
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(("pane-head", i))
                    .h(px(22.))
                    .flex_none()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_b_1()
                    .border_color(c(BORDER_VARIANT))
                    .bg(c(if focused { PANEL } else { BG }))
                    .text_xs()
                    .text_color(c(if focused { TEXT } else { PLACEHOLDER }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, w, cx| {
                        w.focus(&pane.focus_handle(cx));
                        cx.notify();
                    }))
                    .child(div().w(px(6.)).h(px(6.)).flex_none().bg(c(if focused {
                        ACCENT
                    } else {
                        ACTIVE
                    })))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(title),
                    )
                    .child(
                        icon_button(("pane-close", i), "x", "Close pane", MUTED)
                            .size(px(16.))
                            .on_click(cx.listener(move |this, _, w, cx| {
                                cx.stop_propagation();
                                this.request_close(tab_ix, Some(i), w, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    // Dim the panes that don't have the keyboard.
                    .when(!focused, |d| d.opacity(0.6))
                    .child(p.clone()),
            )
    }

    /// The terminals of a split tab in their columns and rows, with draggable dividers.
    pub(super) fn split_view(&self, t: &TermTab, cx: &mut Context<Self>) -> AnyElement {
        let tab_ix = self.active;
        let mut row = div()
            .id("split")
            .size_full()
            .flex()
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, w, cx| {
                if this.pane_drag.is_some() {
                    this.drag_divider(ev.position, w, cx);
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| {
                    if this.pane_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            );
        for (ci, col) in t.layout.iter().enumerate() {
            if ci > 0 {
                row = row.child(self.divider(Divider::Col(ci - 1), cx));
            }
            let col_share = t.cols.get(ci).copied().unwrap_or(0.);
            let mut column = div()
                .flex_basis(gpui::relative(col_share))
                .flex_shrink()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col();
            for (ri, &i) in col.iter().enumerate() {
                if ri > 0 {
                    column = column.child(self.divider(Divider::Row { col: ci, n: ri - 1 }, cx));
                }
                let share = t
                    .rows
                    .get(ci)
                    .and_then(|r| r.get(ri))
                    .copied()
                    .unwrap_or(0.);
                if let Some(p) = t.panes.get(i) {
                    column = column.child(
                        self.pane(tab_ix, i, p, i == t.focused, cx)
                            .flex_basis(gpui::relative(share))
                            .flex_shrink()
                            .min_h_0(),
                    );
                }
            }
            row = row.child(column);
        }
        row.into_any_element()
    }
}
