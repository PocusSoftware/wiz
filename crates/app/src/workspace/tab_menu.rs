//! Rearranging tabs: the right-click menu (pin, rename, duplicate, close) and dragging.

use gpui::{Div, ElementId, MouseButton, Point, anchored, point};

use super::*;

/// A tab being dragged to a new place; also draws itself under the mouse.
#[derive(Clone)]
pub(super) struct DraggedTab {
    pub(super) id: u64,
    pub(super) label: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::widgets::drag_chip(self.label.clone())
    }
}

/// Width of the menu.
const MENU_W: f32 = 180.;

/// Where a menu opened from the keyboard starts: just under the tab bar.
const MENU_TOP: f32 = 34.;
const MENU_LEFT: f32 = 120.;

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

    /// The tab menu of the open tab from the keyboard, where F10 is out of reach.
    pub(super) fn open_active_tab_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Tab {
            return;
        }
        if let Some(id) = self.tabs.get(self.active).map(|t| t.id) {
            self.open_tab_menu(id, point(px(MENU_LEFT), px(MENU_TOP)), window, cx);
        }
    }

    pub(super) fn close_tab_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tab_menu = None;
        self.split_menu = None;
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

    /// Show a name field in place of the tab's title, with the current name selected.
    pub(super) fn start_rename(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
            return;
        };
        let current = tab.name.clone().unwrap_or_else(|| tab.label.clone());
        let input = cx.new(|cx| {
            let mut i = TextInput::new("Tab name", false, cx);
            i.set_value(&current);
            i.select_all();
            i
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus);
        // Clicking anywhere else keeps the name, like Enter.
        let blur = cx.on_blur(&focus, window, |t, w, cx| t.finish_rename(true, w, cx));
        self.renaming = Some((id, input, blur));
        cx.notify();
    }

    /// Close the name field, keeping what was typed when `save`. An empty name goes back
    /// to the default title.
    pub(super) fn finish_rename(
        &mut self,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((id, input, _blur)) = self.renaming.take() else {
            return;
        };
        if save && let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            let name = input.read(cx).value().trim();
            tab.name = (!name.is_empty()).then(|| name.to_string().into());
        }
        if self.page == Page::Tab {
            self.select_tab(self.active, window, cx);
        } else {
            cx.notify();
        }
    }

    /// The name field of the tab being renamed, if it is `id`. Clicks stay in the field
    /// instead of selecting or dragging the tab.
    pub(super) fn rename_field(&self, id: u64) -> Option<impl IntoElement> {
        let (_, input, _) = self.renaming.as_ref().filter(|(r, _, _)| *r == id)?;
        Some(
            div()
                .flex_1()
                .min_w_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(input.clone()),
        )
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
            .and_then(|p| p.view.read(cx).command.clone());
        let Some(pane) = t.panes.get(t.focused) else {
            return;
        };
        let (label, conn, password) = (label.to_string(), pane.conn.clone(), pane.password.clone());
        self.open_shell(label, conn, password, command, window, cx);
    }

    /// Whether a right-click or split menu is open.
    pub(super) fn menu_open(&self) -> bool {
        self.tab_menu.is_some() || self.split_menu.is_some()
    }

    /// The split button's menu: which server the new pane opens on.
    /// `at` is where the mouse was; the keyboard opens it at the top right, under the button.
    pub(super) fn open_split_menu(
        &mut self,
        at: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_terminal = matches!(
            self.tabs.get(self.active).map(|t| &t.kind),
            Some(TabKind::Terminal(_))
        );
        if self.page != Page::Tab || !is_terminal {
            return;
        }
        self.split_menu =
            Some(at.unwrap_or_else(|| point(window.viewport_size().width, px(MENU_TOP))));
        window.focus(&self.focus);
        cx.notify();
    }

    /// A row of a menu.
    fn menu_item(id: impl Into<ElementId>, text: impl Into<SharedString>) -> gpui::Stateful<Div> {
        keyboard(div().id(id))
            .focus(|s| s.bg(c(HOVER)))
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(2.))
            .cursor_pointer()
            .hover(|s| s.bg(c(HOVER)))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(text.into())
    }

    /// `items` in a menu at `at`, drawn over everything. A click anywhere else closes it.
    fn menu_overlay(
        &self,
        key: usize,
        at: Point<Pixels>,
        items: Vec<gpui::Stateful<Div>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let menu = div()
            .w(px(MENU_W))
            .p_1()
            .flex()
            .flex_col()
            .rounded(px(3.))
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .text_color(c(TEXT))
            .children(items);
        div()
            .id("tab-menu")
            .track_focus(&self.modal_focus)
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
            // Anchored keeps the menu inside the window, flipping it up near the bottom.
            .child(
                anchored()
                    .position(at)
                    .snap_to_window_with_margin(px(4.))
                    .child(fade_in(
                        menu.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                        ("tab-menu", key),
                    )),
            )
            .into_any_element()
    }

    /// The open menu: the split button's, or the right-click menu of a tab.
    pub(super) fn tab_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(at) = self.split_menu {
            return Some(self.split_menu_view(at, cx));
        }
        let (id, at) = self.tab_menu?;
        let tab = self.tabs.iter().find(|t| t.id == id)?;
        let mut items = vec![
            Self::menu_item("tm-pin", if tab.pinned { "Unpin tab" } else { "Pin tab" }).on_click(
                cx.listener(move |t, _, w, cx| {
                    t.toggle_pin_tab(id, cx);
                    t.close_tab_menu(w, cx);
                }),
            ),
            Self::menu_item("tm-rename", "Rename").on_click(cx.listener(move |t, _, w, cx| {
                t.tab_menu = None;
                t.start_rename(id, w, cx);
            })),
        ];
        if matches!(tab.kind, TabKind::Terminal(_)) {
            items.push(Self::menu_item("tm-dup", "Duplicate").on_click(cx.listener(
                move |t, _, w, cx| {
                    t.tab_menu = None;
                    t.duplicate_tab(id, w, cx);
                },
            )));
            // Put this tab's terminals into the split of another terminal tab.
            let others = self
                .tabs
                .iter()
                .filter(|o| o.id != id && matches!(o.kind, TabKind::Terminal(_)));
            for o in others {
                let into = o.id;
                let name = o.name.clone().unwrap_or_else(|| o.label.clone());
                items.push(
                    Self::menu_item(
                        ("tm-merge", into as usize),
                        format!("Merge into {}", privacy::redact(&name)),
                    )
                    .on_click(cx.listener(move |t, _, w, cx| {
                        t.tab_menu = None;
                        t.merge_tab(id, into, w, cx);
                    })),
                );
            }
        }
        items.push(Self::menu_item("tm-close", "Close").on_click(cx.listener(
            move |t, _, w, cx| {
                t.tab_menu = None;
                if let Some(ix) = t.tabs.iter().position(|t| t.id == id) {
                    t.request_close(ix, None, w, cx);
                }
            },
        )));
        Some(self.menu_overlay(id as usize, at, items, cx))
    }

    /// Where the next pane opens: the same server, or any saved host.
    fn split_menu_view(&self, at: Point<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let mut items = vec![
            Self::menu_item("sm-same", "Same server").on_click(cx.listener(|t, _, w, cx| {
                t.close_tab_menu(w, cx);
                t.split(w, cx);
            })),
        ];
        for (ix, h) in self.cfg.hosts.iter().enumerate() {
            items.push(
                Self::menu_item(("sm-host", ix), h.name.clone()).on_click(cx.listener(
                    move |t, _, w, cx| {
                        t.close_tab_menu(w, cx);
                        t.split_with(ix, w, cx);
                    },
                )),
            );
        }
        self.menu_overlay(usize::MAX, at, items, cx)
    }
}
