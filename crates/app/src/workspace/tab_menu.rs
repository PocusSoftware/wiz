//! Rearranging tabs: the right-click menu (pin, duplicate, close) and dragging.

use gpui::{MouseButton, Point};

use super::*;

/// A tab being dragged to a new place; also draws itself under the mouse.
#[derive(Clone)]
pub(super) struct DraggedTab {
    pub(super) id: u64,
    pub(super) label: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py(px(3.))
            .rounded(px(2.))
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .text_color(c(TEXT))
            .font_family(ui())
            .text_size(px(12.))
            .child(self.label.clone())
    }
}

/// Width of the menu, also used to keep it inside the window.
const MENU_W: f32 = 180.;

impl Workspace {
    pub(super) fn open_tab_menu(
        &mut self,
        id: u64,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_menu = Some((id, at));
        // Take keyboard focus from the terminal so Escape reaches the menu.
        window.focus(&self.focus);
        cx.notify();
    }

    pub(super) fn close_tab_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tab_menu = None;
        if self.page == Page::Tab {
            self.select_tab(self.active, window, cx);
        } else {
            cx.notify();
        }
    }

    /// Pin or unpin a tab. Pinned tabs stay together at the front: a pinned tab goes after
    /// the other pinned ones, an unpinned one right after them.
    pub(super) fn toggle_pin_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        self.reorder(id, cx, |tab, others| {
            tab.pinned = !tab.pinned;
            others.iter().take_while(|t| t.pinned).count()
        });
    }

    /// Move a dragged tab to where tab `to` is, without crossing between pinned and
    /// unpinned tabs.
    pub(super) fn move_tab(&mut self, id: u64, to: usize, cx: &mut Context<Self>) {
        self.reorder(id, cx, |tab, others| {
            let pinned = others.iter().take_while(|t| t.pinned).count();
            if tab.pinned {
                to.min(pinned)
            } else {
                to.clamp(pinned, others.len())
            }
        });
    }

    /// Take tab `id` out, let `place` change it and pick its new index among the others,
    /// and keep the selected tab selected.
    fn reorder(
        &mut self,
        id: u64,
        cx: &mut Context<Self>,
        place: impl FnOnce(&mut Tab, &[Tab]) -> usize,
    ) {
        let Some(ix) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let active = self.tabs.get(self.active).map(|t| t.id);
        let mut tab = self.tabs.remove(ix);
        let to = place(&mut tab, &self.tabs);
        self.tabs.insert(to, tab);
        if let Some(i) = active.and_then(|a| self.tabs.iter().position(|t| t.id == a)) {
            self.active = i;
        }
        self.tab_scroll.scroll_to_item(self.active);
        cx.notify();
    }

    /// Open another shell on the same connection as a terminal tab, running the same command
    /// (`docker exec`) as its focused pane.
    fn duplicate_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Tab {
            label,
            kind: TabKind::Terminal(t),
            ..
        }) = self.tabs.iter().find(|t| t.id == id)
        else {
            return;
        };
        let command = t
            .panes
            .get(t.focused)
            .and_then(|p| p.read(cx).command.clone());
        let (label, conn, password) = (label.to_string(), t.conn.clone(), t.password.clone());
        self.open_shell(label, conn, password, command, window, cx);
    }

    /// The open menu, drawn over everything. A click anywhere else closes it.
    pub(super) fn tab_menu(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, at) = self.tab_menu?;
        let tab = self.tabs.iter().find(|t| t.id == id)?;
        let item = |name: &'static str, text: &'static str| {
            div()
                .id(name)
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .rounded(px(2.))
                .cursor_pointer()
                .hover(|s| s.bg(c(HOVER)))
                .child(text)
        };
        let menu = div()
            .absolute()
            .left((at.x).min(window.viewport_size().width - px(MENU_W + 4.)))
            .top(at.y)
            .w(px(MENU_W))
            .p_1()
            .flex()
            .flex_col()
            .rounded(px(3.))
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .text_color(c(TEXT))
            .child(
                item("tm-pin", if tab.pinned { "Unpin tab" } else { "Pin tab" }).on_click(
                    cx.listener(move |t, _, w, cx| {
                        t.toggle_pin_tab(id, cx);
                        t.close_tab_menu(w, cx);
                    }),
                ),
            )
            .when(matches!(tab.kind, TabKind::Terminal(_)), |d| {
                d.child(
                    item("tm-dup", "Duplicate").on_click(cx.listener(move |t, _, w, cx| {
                        t.tab_menu = None;
                        t.duplicate_tab(id, w, cx);
                    })),
                )
            })
            .child(
                item("tm-close", "Close").on_click(cx.listener(move |t, _, w, cx| {
                    t.tab_menu = None;
                    if let Some(ix) = t.tabs.iter().position(|t| t.id == id) {
                        t.request_close(ix, None, w, cx);
                    }
                })),
            );
        Some(
            div()
                .id("tab-menu")
                .absolute()
                .size_full()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|t, _, w, cx| t.close_tab_menu(w, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|t, _, w, cx| t.close_tab_menu(w, cx)),
                )
                .child(fade_in(
                    menu.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                    ("tab-menu", id as usize),
                ))
                .into_any_element(),
        )
    }
}
