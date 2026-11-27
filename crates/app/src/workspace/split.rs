//! Split terminals: up to four shells in a tab, on one server or several, laid out for the space.

use super::*;

impl TermTab {
    /// Switch to `layout`, sharing the space evenly whenever its shape changes.
    fn set_layout(&mut self, layout: Vec<Vec<usize>>) {
        if layout == self.layout {
            return;
        }
        self.cols = even(&self.cols, layout.len());
        self.rows = layout
            .iter()
            .enumerate()
            .map(|(i, col)| even(self.rows.get(i).map_or(&[], Vec::as_slice), col.len()))
            .collect();
        self.layout = layout;
    }

    pub(super) fn step(&mut self) -> bool {
        let now = Instant::now();
        let dt = self.ticked.map_or(0., |t| (now - t).as_secs_f32());
        let mut moving = false;
        for s in self.cols.iter_mut().chain(self.rows.iter_mut().flatten()) {
            moving |= s.step(dt);
        }
        self.ticked = moving.then_some(now);
        moving
    }
}

fn even(old: &[Spring], n: usize) -> Vec<Spring> {
    (0..n)
        .map(|i| Spring {
            target: 1.0 / n as f32,
            ..old.get(i).copied().unwrap_or(Spring::new(0.))
        })
        .collect()
}

/// A split pane being dragged by its header onto another pane of the same tab.
#[derive(Clone)]
struct DraggedPane {
    tab: u64,
    from: usize,
    title: SharedString,
}

impl Render for DraggedPane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::widgets::drag_chip(self.title.clone())
    }
}

/// Most terminals in one tab (Experimental setting).
pub(super) fn max_panes() -> usize {
    crate::prefs::get().max_splits() as usize
}

/// Smallest usable pane (header included). Splitting is refused when another pane
/// would not get at least this much room.
const MIN_PANE_W: f32 = 320.;
const MIN_PANE_H: f32 = 150.;

/// Narrowest a pane may be dragged, as a share of the tab.
const MIN_PANE: f32 = 0.12;

/// Columns of pane indices for `n` panes in a `w` x `h` area, or `None` when they can't all
/// get a usable size. Two panes sit side by side, more go in a grid that fills its columns
/// from the left (a third goes below the first, a fourth below the second), so existing
/// panes never jump around. When the window is too short for a grid but wide enough, panes
/// go side by side instead (or stacked when it is tall and narrow).
pub(super) fn layout(n: usize, w: f32, h: f32) -> Option<Vec<Vec<usize>>> {
    let cols_fit = |k: usize| w / k as f32 >= MIN_PANE_W;
    let rows_fit = |k: usize| h / k as f32 >= MIN_PANE_H;
    let side_by_side = || (0..n).map(|i| vec![i]).collect();
    let stacked = || vec![(0..n).collect()];
    let cols = grid_columns(n);
    match n {
        0 | 1 => Some(vec![vec![0]]),
        2 if cols_fit(2) => Some(side_by_side()),
        _ if n > 2 && cols_fit(cols) && rows_fit(n.div_ceil(cols)) => Some(grid_layout(n)),
        _ if cols_fit(n) => Some(side_by_side()),
        _ if rows_fit(n) => Some(stacked()),
        _ => None,
    }
}

/// Columns of the grid for `n` panes: 2 up to four panes, then as square as it gets.
fn grid_columns(n: usize) -> usize {
    (1..=n).find(|c| c * c >= n).unwrap_or(1)
}

/// The grid arrangement, also used when a window shrinks below what fits: panes are never
/// dropped, they just get small.
pub(super) fn grid_layout(n: usize) -> Vec<Vec<usize>> {
    let mut cols = vec![Vec::new(); grid_columns(n).max(1)];
    for i in 0..n.max(1) {
        let k = i % cols.len();
        cols[k].push(i);
    }
    cols
}

impl Workspace {
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

    /// Focus the next or previous pane of a split terminal, in the order they were opened.
    pub(super) fn cycle_pane(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Tab || self.modal_open() {
            return;
        }
        let Some(Tab {
            kind: TabKind::Terminal(t),
            ..
        }) = self.tabs.get(self.active)
        else {
            return;
        };
        let n = t.panes.len();
        let cur = Self::focused_pane(t, window, cx);
        let next = (if back { cur + n - 1 } else { cur + 1 }) % n;
        window.focus(&t.panes[next].focus_handle(cx), cx);
        cx.notify();
    }

    /// Whether terminal tab `ix` has room for `extra` more panes. Says why when it does not.
    fn room_for(
        &mut self,
        ix: usize,
        extra: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.get(ix) else {
            return false;
        };
        let n = tab.pane_count() + extra;
        let area = self.area(window);
        if n > max_panes() {
            self.set_status(format!("A tab holds at most {} panes.", max_panes()), true);
        } else if layout(n, area.0, area.1).is_none() {
            self.set_status(
                "The window is too small for another pane. Make it bigger or close a pane.",
                true,
            );
        } else {
            return true;
        }
        cx.notify();
        false
    }

    /// Open another shell on the same server as the focused pane, beside it.
    pub(super) fn split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Tab || !self.room_for(self.active, 1, window, cx) {
            return;
        }
        let Some(Tab {
            id,
            label,
            kind: TabKind::Terminal(t),
            ..
        }) = self.tabs.get(self.active)
        else {
            return;
        };
        let Some(pane) = t.focused_shell() else {
            return;
        };
        if pane.conn.is_closed() {
            self.set_status("The connection for this pane has closed.", true);
            cx.notify();
            return;
        }
        let (id, label, conn, pw) = (
            *id,
            label.to_string(),
            pane.conn.clone(),
            pane.password.clone(),
        );
        self.spawn_shell(label, conn, pw, None, Some(id), window, cx);
    }

    pub(super) fn split_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Tab || !self.room_for(self.active, 1, window, cx) {
            return;
        }
        let Some(Tab {
            id,
            kind: TabKind::Terminal(t),
            ..
        }) = self.tabs.get(self.active)
        else {
            return;
        };
        let Some(pane) = t.focused_shell() else {
            return;
        };
        if pane.conn.is_closed() {
            self.set_status("The connection for this pane has closed.", true);
            cx.notify();
            return;
        }
        let (id, conn) = (*id, pane.conn.clone());
        let host = match self.cfg.hosts.iter().find(|h| h.account() == pane.account) {
            Some(h) => h.clone(),
            None => match self.quick.get(&pane.account) {
                Some((h, _)) => h.clone(),
                None => return,
            },
        };
        self.open_sftp(host, conn, Some(id), window, cx);
    }

    /// Open a shell on saved host `host` as a new pane of the active tab.
    pub(super) fn split_with(&mut self, host: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Tab || !self.room_for(self.active, 1, window, cx) {
            return;
        }
        if let Some(id) = self.tabs.get(self.active).map(|t| t.id) {
            self.open_host(host, Open::SplitShell(id), window, cx);
        }
    }

    /// Move the panes of terminal tab `id` into the split of tab `into`, and close `id`.
    pub(super) fn merge_tab(
        &mut self,
        id: u64,
        into: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = |tabs: &[Tab], id| tabs.iter().position(|t| t.id == id);
        let (Some(from), Some(to)) = (at(&self.tabs, id), at(&self.tabs, into)) else {
            return;
        };
        if from == to || !self.room_for(to, self.tabs[from].pane_count(), window, cx) {
            return;
        }
        let src = self.tabs.remove(from).into_panes();
        let Some(to) = at(&self.tabs, into) else {
            return;
        };
        if !matches!(self.tabs[to].kind, TabKind::Terminal(_)) {
            let old = self.tabs.remove(to);
            let (id, label, pinned, name) =
                (old.id, old.label.clone(), old.pinned, old.name.clone());
            self.tabs.insert(
                to,
                Tab {
                    id,
                    label,
                    kind: TabKind::Terminal(TermTab::new(old.into_panes())),
                    pinned,
                    name,
                    focus: cx.focus_handle(),
                    account: None,
                },
            );
        }
        if let TabKind::Terminal(t) = &mut self.tabs[to].kind {
            t.focused = t.panes.len();
            t.panes.extend(src);
        }
        self.select_tab(to, window, cx);
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
        if let [PaneView::View(_)] = t.panes.as_slice()
            && let Some(PaneView::View(only)) = t.panes.pop()
        {
            self.tabs[ix] = *only;
        }
        self.select_tab(ix, window, cx);
    }

    /// Swap two panes of a split tab. The dragged pane keeps the keyboard.
    pub(super) fn swap_panes(
        &mut self,
        ix: usize,
        from: usize,
        to: usize,
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
        if from == to || from.max(to) >= t.panes.len() {
            return;
        }
        t.panes.swap(from, to);
        t.focused = to;
        self.select_tab(ix, window, cx);
    }

    /// Size of the content area below the tab bar, from the last frame.
    fn area(&self, window: &Window) -> (f32, f32) {
        let (b, painted_in) = self.content_bounds.get();
        let v = window.viewport_size();
        if b.size.width > px(0.) {
            (
                f32::from(b.size.width + v.width - painted_in.width),
                f32::from(b.size.height + v.height - painted_in.height),
            )
        } else {
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
        let pair = sizes[a].target + sizes[b].target;
        let first = (sizes[a].target + delta).clamp(MIN_PANE, pair - MIN_PANE);
        sizes[a] = Spring::new(first);
        sizes[b] = Spring::new(pair - first);
        drag.last = now;
        cx.notify();
    }

    /// A draggable split divider: vertical between columns, horizontal between rows.
    pub(super) fn divider(&self, which: Divider, cx: &mut Context<Self>) -> Stateful<Div> {
        let dragging = self.pane_drag.as_ref().is_some_and(|d| d.divider == which);
        let line = c(if dragging { accent() } else { palette().border });
        let id = match which {
            Divider::Col(n) => ("divider", n),
            Divider::Row { col, n } => ("row-divider", col * 8 + n),
        };
        let d = div()
            .id(id)
            .flex_none()
            .flex()
            .hover(|s| s.bg(c(palette().hover)));
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
        p: &PaneView,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let title = p.title(cx);
        let focus = p.focus_handle(cx);
        let tab = self.tabs[tab_ix].id;
        div()
            .min_w_0()
            .flex()
            .flex_col()
            // Transparent until another pane is dragged over it.
            .border_1()
            .border_color(gpui::transparent_black())
            .drag_over::<DraggedPane>(|s, _, _, _| s.border_color(c(accent())))
            .on_drop(cx.listener(move |this, d: &DraggedPane, w, cx| {
                if d.tab == tab {
                    this.swap_panes(tab_ix, d.from, i, w, cx);
                }
            }))
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
                    .border_color(c(palette().border_variant))
                    .bg(c(if focused { palette().panel } else { bg() }))
                    .text_xs()
                    .text_color(c(if focused {
                        palette().text
                    } else {
                        palette().placeholder
                    }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, w, cx| {
                        w.focus(&focus, cx);
                        cx.notify();
                    }))
                    // Drag the header onto another pane to swap the two.
                    .on_drag(
                        DraggedPane {
                            tab,
                            from: i,
                            title: title.clone().into(),
                        },
                        |d, _, _, cx| cx.new(|_| d.clone()),
                    )
                    .child(div().w(px(6.)).h(px(6.)).flex_none().bg(c(if focused {
                        accent()
                    } else {
                        palette().active
                    })))
                    .child(div().flex_1().min_w_0().truncate().child(title))
                    .child(
                        icon_button(("pane-close", i), "x", "Close pane", palette().muted)
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
                    .child(p.element()),
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
            let col_share = t.cols.get(ci).map_or(0., |s| s.value);
            let mut column = div()
                .flex_basis(gpui::relative(col_share))
                .flex_shrink(1.)
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
                    .map_or(0., |s| s.value);
                if let Some(p) = t.panes.get(i) {
                    column = column.child(
                        self.pane(tab_ix, i, p, i == t.focused, cx)
                            .flex_basis(gpui::relative(share))
                            .flex_shrink(1.)
                            .min_h_0(),
                    );
                }
            }
            row = row.child(column);
        }
        row.into_any_element()
    }
}
