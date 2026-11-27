//! Rearranging tabs: the right-click menu (pin, rename, duplicate, close) and dragging.
//! Also the other menus drawn the same way: the split button's and the host list's.

use gpui::{Anchor, Div, ElementId, MouseButton, Point, anchored, point};

use super::home::address;
use super::hosts::card_icon;
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
/// Width of the split menu, which also shows each host's address.
const SPLIT_MENU_W: f32 = 280.;

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
        window.focus(&self.focus, cx);
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
        self.hosts_menu = None;
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
        window.focus(&focus, cx);
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
        let Some(pane) = t.focused_shell() else {
            return;
        };
        let command = pane.view.read(cx).command.clone();
        let (label, conn, password) = (label.to_string(), pane.conn.clone(), pane.password.clone());
        self.open_shell(label, conn, password, command, window, cx);
    }

    /// Whether a right-click or split menu is open.
    pub(super) fn menu_open(&self) -> bool {
        self.tab_menu.is_some() || self.split_menu.is_some() || self.hosts_menu.is_some()
    }

    /// The right-click menu of the host list at `at`: of the host with `account`, or of the
    /// empty part of the list.
    pub(super) fn open_hosts_menu(
        &mut self,
        at: Point<Pixels>,
        account: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hosts_menu = Some((at, account));
        // Out of the search field, so Escape reaches the menu.
        window.focus(&self.focus, cx);
        cx.notify();
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
        // Under the tab bar, its right edge at the button, so it never covers the bar.
        let x = at.map_or(window.viewport_size().width - px(4.), |p| p.x);
        self.split_menu = Some(point(x, px(MENU_TOP)));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// A row of a menu.
    fn menu_item(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
        keyboard(div().id(id))
            .focus(|s| s.bg(c(palette().hover)))
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(2.))
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(text.into())
    }

    /// `items` in a menu `width` wide with its `corner` at `at`, drawn over everything. A
    /// click anywhere else closes it.
    fn menu_overlay(
        &self,
        key: usize,
        at: Point<Pixels>,
        corner: Anchor,
        width: f32,
        items: impl IntoIterator<Item = impl IntoElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let menu = div()
            .w(px(width))
            .p_1()
            .flex()
            .flex_col()
            .rounded(px(3.))
            .bg(c(palette().elevated))
            .border_1()
            .border_color(c(palette().border))
            .text_color(c(palette().text))
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
                    .anchor(corner)
                    .position(at)
                    .snap_to_window_with_margin(px(4.))
                    .child(fade_in(
                        menu.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                        ("tab-menu", key),
                    )),
            )
            .into_any_element()
    }

    /// The open menu: the split button's, the host list's, or the right-click menu of a tab.
    pub(super) fn tab_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(at) = self.split_menu {
            return Some(self.split_menu_view(at, cx));
        }
        if let Some((at, account)) = &self.hosts_menu {
            return self.hosts_menu_view(*at, account.as_deref(), cx);
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
        }
        // Put this tab's panes into the split of another tab.
        for o in self.tabs.iter().filter(|o| o.id != id) {
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
        items.push(
            Self::menu_item("tm-window", "Open in new window").on_click(cx.listener(
                move |t, _, w, cx| {
                    t.tab_menu = None;
                    t.pop_out(id, w, cx);
                },
            )),
        );
        if let Some(main) = self.other_windows(cx).into_iter().next() {
            items.push(
                Self::menu_item("tm-back", "Move to other window").on_click(cx.listener(
                    move |t, _, w, cx| {
                        t.tab_menu = None;
                        t.move_to(id, main, w, cx);
                    },
                )),
            );
        }
        items.push(Self::menu_item("tm-close", "Close").on_click(cx.listener(
            move |t, _, w, cx| {
                t.tab_menu = None;
                if let Some(ix) = t.tabs.iter().position(|t| t.id == id) {
                    t.request_close(ix, None, w, cx);
                }
            },
        )));
        Some(self.menu_overlay(id as usize, at, Anchor::TopLeft, MENU_W, items, cx))
    }

    /// The host list's menu: what can be done with the host with `account`, or adding a
    /// host when it was opened on the empty part of the list.
    fn hosts_menu_view(
        &self,
        at: Point<Pixels>,
        account: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Some(account) = account else {
            let add = Self::icon_row(
                "hm-add",
                "plus",
                "Add host",
                crate::shortcuts::label("new_host"),
            )
            .on_click(cx.listener(|t, _, w, cx| {
                t.close_tab_menu(w, cx);
                t.open_form(None, w, cx);
            }));
            return Some(self.menu_overlay(usize::MAX - 1, at, Anchor::TopLeft, MENU_W, [add], cx));
        };
        let ix = self.cfg.hosts.iter().position(|h| h.account() == account)?;
        let pinned = self.cfg.hosts[ix].pinned;
        let open = |id: &'static str, icon_name: &str, text: &str, what: fn() -> Open| {
            Self::icon_row(id, icon_name, text.to_string(), "")
                .on_click(cx.listener(move |t, _, w, cx| {
                    t.close_tab_menu(w, cx);
                    t.open_host(ix, what(), w, cx);
                }))
                .into_any_element()
        };
        let items = [
            open("hm-shell", "terminal", "New terminal", || Open::Shell),
            open("hm-files", "folder", "Files (SFTP)", || Open::Sftp),
            open("hm-docker", "container", "Docker containers", || {
                Open::Docker
            }),
            div()
                .my_1()
                .h(px(1.))
                .bg(c(palette().border_variant))
                .into_any_element(),
            Self::icon_row(
                "hm-pin",
                "pin",
                if pinned { "Unpin" } else { "Pin to top" },
                "",
            )
            .on_click(cx.listener(move |t, _, w, cx| {
                t.close_tab_menu(w, cx);
                t.toggle_pin(ix, cx);
            }))
            .into_any_element(),
            Self::icon_row("hm-edit", "pencil", "Edit", "")
                .on_click(cx.listener(move |t, _, w, cx| {
                    t.close_tab_menu(w, cx);
                    t.open_form(Some(ix), w, cx);
                }))
                .into_any_element(),
            Self::icon_row("hm-copy", "copy", "Duplicate", "")
                .on_click(cx.listener(move |t, _, w, cx| {
                    t.close_tab_menu(w, cx);
                    t.duplicate_host(ix, w, cx);
                }))
                .into_any_element(),
            Self::icon_row("hm-del", "trash", "Delete", "")
                .text_color(c(danger()))
                .on_click(cx.listener(move |t, _, w, cx| {
                    t.close_tab_menu(w, cx);
                    t.ask_delete_host(ix, w, cx);
                }))
                .into_any_element(),
        ];
        Some(self.menu_overlay(usize::MAX - 1, at, Anchor::TopLeft, MENU_W, items, cx))
    }

    /// Where the next pane opens: the same server, or any saved host. A row opens a
    /// terminal, its folder button the files.
    fn split_menu_view(&self, at: Point<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let same = Self::split_row(
            "sm-same",
            icon("columns", 14., palette().muted),
            "Same server".into(),
            None,
            crate::shortcuts::label("split").into(),
            Self::files_button("sm-files", "Files on same server".into()).on_click(cx.listener(
                |t, _, w, cx| {
                    cx.stop_propagation();
                    t.close_tab_menu(w, cx);
                    t.split_files(w, cx);
                },
            )),
        )
        .on_click(cx.listener(|t, _, w, cx| {
            t.close_tab_menu(w, cx);
            t.split(w, cx);
        }));
        let mut items = vec![same.into_any_element()];
        if !self.cfg.hosts.is_empty() {
            items.push(
                div()
                    .my_1()
                    .h(px(1.))
                    .bg(c(palette().border_variant))
                    .into_any_element(),
            );
            items.push(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(c(palette().placeholder))
                    .child("Other hosts")
                    .into_any_element(),
            );
        }
        let logos = crate::prefs::get().show_os_logos();
        for (ix, h) in self.cfg.hosts.iter().enumerate() {
            let name = privacy::redact(&h.name).into_owned();
            let files =
                Self::files_button(("sm-host-files", ix), format!("Files on {name}").into())
                    .on_click(cx.listener(move |t, _, w, cx| {
                        cx.stop_propagation();
                        t.close_tab_menu(w, cx);
                        t.split_files_with(ix, w, cx);
                    }));
            items.push(
                Self::split_row(
                    ("sm-host", ix),
                    icon(&card_icon(h, logos), 14., accent_text()),
                    name.into(),
                    Some(address(h).into()),
                    SharedString::default(),
                    files,
                )
                .on_click(cx.listener(move |t, _, w, cx| {
                    t.close_tab_menu(w, cx);
                    t.split_with(ix, w, cx);
                }))
                .into_any_element(),
            );
        }
        self.menu_overlay(usize::MAX, at, Anchor::TopRight, SPLIT_MENU_W, items, cx)
    }

    /// A split menu row: icon, name with the address under it, a muted shortcut, and the
    /// files button at the end.
    fn split_row(
        id: impl Into<ElementId>,
        lead: impl IntoElement,
        title: SharedString,
        subtitle: Option<SharedString>,
        shortcut: SharedString,
        files: Stateful<Div>,
    ) -> Stateful<Div> {
        keyboard(div().id(id))
            .focus(|s| s.bg(c(palette().hover)))
            .h(px(if subtitle.is_some() { 38. } else { 30. }))
            .pl_2()
            .pr_1()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(2.))
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .whitespace_nowrap()
            .child(lead)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().child(title))
                    .when_some(subtitle, |d, sub| {
                        d.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(c(palette().placeholder))
                                .child(sub),
                        )
                    }),
            )
            .when(!shortcut.is_empty(), |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(c(palette().placeholder))
                        .child(shortcut),
                )
            })
            .child(files)
    }

    /// A menu row: icon, text, and a muted note on the right.
    fn icon_row(
        id: impl Into<ElementId>,
        icon_name: &str,
        text: impl Into<SharedString>,
        note: impl Into<SharedString>,
    ) -> Stateful<Div> {
        keyboard(div().id(id))
            .focus(|s| s.bg(c(palette().hover)))
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(2.))
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .whitespace_nowrap()
            .child(icon(icon_name, 14., palette().muted))
            .child(div().flex_1().min_w_0().truncate().child(text.into()))
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(px(120.))
                    .truncate()
                    .text_xs()
                    .text_color(c(palette().placeholder))
                    .child(note.into()),
            )
    }

    fn files_button(id: impl Into<ElementId>, tooltip: SharedString) -> Stateful<Div> {
        icon_button(id, "folder", tooltip, palette().muted).flex_shrink_0()
    }
}
