//! The tab strip (also the title bar), opening, selecting and closing tabs, and quitting.

use gpui::WindowControlArea::{self, Close, Max, Min};
use gpui::{KeyDownEvent, MouseButton, MouseDownEvent, point};
use ssh_core::config::WindowControls;

use super::tab_menu::DraggedTab;
use super::*;

/// Width of the hosts tab at the start of the strip.
const HOME_W: f32 = 38.;

impl Workspace {
    pub(super) fn push_tab(
        &mut self,
        label: String,
        kind: TabKind,
        account: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = self.new_tab(label, kind, account, cx);
        self.tabs.push(tab);
        self.select_tab(self.tabs.len() - 1, window, cx);
    }

    pub(super) fn new_tab(
        &mut self,
        label: String,
        kind: TabKind,
        account: Option<String>,
        cx: &mut Context<Self>,
    ) -> Tab {
        let id = self.next_tab;
        self.next_tab += 1;
        Tab {
            id,
            label: label.into(),
            kind,
            pinned: false,
            name: None,
            focus: cx.focus_handle(),
            account,
        }
    }

    pub(super) fn select_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // Re-selecting the current page only refocuses it; the enter animation is for changes.
        if self.page != Page::Tab || ix != self.active {
            self.epoch += 1;
        }
        self.active = ix;
        self.page = Page::Tab;
        self.tab_scroll.scroll_to_item(ix);
        match &self.tabs[ix].kind {
            TabKind::Terminal(t) => {
                if let Some(p) = t.panes.get(t.focused).or(t.panes.first()) {
                    window.focus(&p.focus_handle(cx), cx)
                }
            }
            TabKind::Sftp(v) => {
                let h = v.read(cx).focus_handle(cx);
                window.focus(&h, cx)
            }
            TabKind::Logs(v) => {
                let h = v.read(cx).focus_handle(cx);
                window.focus(&h, cx)
            }
            _ => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// Show the hosts or settings page.
    pub(super) fn show_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            self.epoch += 1;
        }
        match (page, &self.settings) {
            (Page::Settings, Some(v)) => window.focus(&v.read(cx).focus_handle(cx), cx),
            // Typing on the hosts page searches.
            (Page::Home, _) => window.focus(&self.search.focus_handle(cx), cx),
            _ => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    pub(super) fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // Dropping the view drops the shell input sender, which closes the channel.
        self.tabs.remove(ix);
        if self.page != Page::Tab {
            // Closed from the tab strip while another page shows: stay there.
            if ix < self.active || self.active >= self.tabs.len() {
                self.active = self.active.saturating_sub(1);
            }
            cx.notify();
        } else if self.tabs.is_empty() {
            self.active = 0;
            self.show_page(Page::Home, window, cx);
        } else {
            let next = if ix < self.active {
                self.active - 1
            } else {
                self.active
            };
            self.select_tab(next.min(self.tabs.len() - 1), window, cx);
        }
    }

    /// Close Tab: close the focused pane of a split, otherwise the whole tab.
    pub(super) fn close_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.active;
        let pane = match self.tabs.get(ix) {
            Some(Tab {
                kind: TabKind::Terminal(t),
                ..
            }) if t.panes.len() > 1 => Some(Self::focused_pane(t, window, cx)),
            _ => None,
        };
        self.request_close(ix, pane, window, cx);
    }

    /// Close a tab (or one pane of a split), first asking when the setting is on and it
    /// holds an SSH session.
    pub(super) fn request_close(
        &mut self,
        ix: usize,
        pane: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        if Self::holds_session(tab, cx) && crate::prefs::get().confirm_close() {
            self.confirm = Some((tab.id, pane));
            // Take keyboard focus from the terminal so Enter and Esc reach the dialog.
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        self.do_close(ix, pane, window, cx);
    }

    /// Tabs on a server (terminal, files, remote Docker) hold an SSH session.
    pub(super) fn holds_session(tab: &Tab, cx: &App) -> bool {
        match &tab.kind {
            TabKind::Terminal(t) => t.panes.iter().any(|p| match p {
                PaneView::Shell(_) => true,
                PaneView::View(tab) => Self::holds_session(tab, cx),
            }),
            TabKind::Sftp(_) => true,
            TabKind::Docker(v) => v.read(cx).is_remote(),
            TabKind::Logs(_) => false,
        }
    }

    /// Quit, unless the setting is on and sessions are still open.
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.open_sessions(cx);
        if open > 0 && crate::prefs::get().block_quit() {
            self.quit_notice = Some(open);
            window.focus(&self.focus, cx);
            cx.notify();
        } else {
            cx.quit();
        }
    }

    pub(super) fn dismiss_quit_notice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quit_notice = None;
        if self.page == Page::Tab {
            self.select_tab(self.active, window, cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn do_close(
        &mut self,
        ix: usize,
        pane: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match pane {
            Some(p) => self.close_pane(ix, p, window, cx),
            None => self.close_tab(ix, window, cx),
        }
    }

    /// Answer the close dialog.
    pub(super) fn answer_close(&mut self, yes: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, pane)) = self.confirm.take() else {
            return;
        };
        let Some(ix) = self.tabs.iter().position(|t| t.id == id) else {
            cx.notify();
            return;
        };
        if yes {
            self.do_close(ix, pane, window, cx);
        } else if self.page == Page::Tab {
            self.select_tab(self.active, window, cx);
        } else {
            cx.notify();
        }
    }

    /// The tab strip doubles as the title bar: empty space drags the window.
    pub(super) fn tab_bar(&self, window_w: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        // Share the space between the hat and the buttons on the right: tabs are 220 px
        // wide until they don't fit, then narrow down to 110 px and the strip scrolls.
        const RESERVED: f32 = HOME_W + 4. * 24. + 3. + 8. + 3. * 40. + 32.;
        let room = f32::from(window_w) - RESERVED;
        let tab_w = (room / self.tabs.len().max(1) as f32).clamp(110., 220.);
        let prefs = crate::prefs::get();
        let left = prefs.window_controls() == WindowControls::Left;
        // Where a tab starts, for the tab menu opened from the keyboard.
        let strip_x = if left { CONTROLS_W } else { 0. } + HOME_W;
        let scroll_x = self.tab_scroll.offset().x;
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let active = self.page == Page::Tab && ix == self.active;
            let mut text = privacy::redact(&tab.label).into_owned();
            if let Some(name) = &tab.name {
                text = privacy::redact(name).into_owned();
            } else if let TabKind::Terminal(t) = &tab.kind
                && let Some(p) = t.panes.get(t.focused).and_then(PaneView::shell)
                && let Some(title) = p.view.read(cx).title()
                && !title.is_empty()
            {
                text = format!("{text} · {}", privacy::redact(&title));
            }
            let panes = match &tab.kind {
                TabKind::Terminal(t) => t.panes.len(),
                _ => 1,
            };
            let tab_icon = match tab.kind {
                TabKind::Terminal(_) => "terminal",
                TabKind::Docker(_) => "container",
                TabKind::Sftp(_) => "folder",
                TabKind::Logs(_) => "logs",
            };
            let id = tab.id;
            let menu_at = point(px(strip_x + ix as f32 * tab_w) + scroll_x, px(TITLEBAR_H));
            div()
                .id(("tab", ix))
                .group(format!("tab-{ix}"))
                // Only the selected tab is a Tab stop; the arrow keys move along the strip.
                .track_focus(&tab.focus.clone().tab_stop(ix == self.active))
                .focus(|s| s.bg(c(palette().active)))
                .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, w, cx| {
                    t.strip_key(ix + 1, menu_at, ev, w, cx)
                }))
                .h_full()
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .flex_shrink_0()
                .w(px(tab_w))
                .border_r_1()
                .border_color(c(palette().border_variant))
                .relative()
                .when(active, |d| {
                    d.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(px(2.))
                            .bg(c(accent())),
                    )
                })
                .bg(c(bg()))
                .text_color(c(if active {
                    palette().text
                } else {
                    palette().muted
                }))
                .hover(|s| s.text_color(c(palette().text)))
                // Selecting moves focus to the tab's content; no flash of the focus look.
                .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                .on_click(cx.listener(move |t, _, w, cx| t.select_tab(ix, w, cx)))
                .on_drag(
                    DraggedTab {
                        id,
                        label: tab.label.clone(),
                    },
                    |d, _, _, cx| cx.new(|_| d.clone()),
                )
                .drag_over::<DraggedTab>(|s, _, _, _| s.bg(c(palette().active)))
                .on_drop(cx.listener(move |t, d: &DraggedTab, _, cx| t.move_tab(d.id, ix, cx)))
                // Middle click closes, like browsers; pinned tabs close only from the menu.
                .when(!tab.pinned, |d| {
                    d.on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |t, _, w, cx| t.request_close(ix, None, w, cx)),
                    )
                })
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |t, ev: &MouseDownEvent, w, cx| {
                        t.open_tab_menu(id, ev.position, w, cx)
                    }),
                )
                .child(icon(
                    tab_icon,
                    13.,
                    if active {
                        accent_text()
                    } else {
                        palette().muted
                    },
                ))
                .map(|d| match self.rename_field(id) {
                    Some(field) => d.child(field),
                    // Fills the tab, so the close button sits at its right edge.
                    None => d.child(div().flex_1().min_w_0().truncate().child(text)),
                })
                // Pane count of a split tab, outside the text so truncation never hides it.
                .when(panes > 1, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .px(px(4.))
                            .rounded(px(2.))
                            .bg(c(palette().active))
                            .text_xs()
                            .text_color(c(palette().muted))
                            .child(panes.to_string()),
                    )
                })
                // A pinned tab closes only from its menu.
                .when(tab.pinned, |d| d.child(icon("pin", 12., palette().muted)))
                .when(!tab.pinned, |d| {
                    d.child(
                        icon_button(
                            ("tab-close", ix),
                            "x",
                            format!("Close ({})", crate::shortcuts::label("close_tab")),
                            palette().muted,
                        )
                        .size(px(16.))
                        // Delete on the focused tab does the same.
                        .tab_stop(false)
                        .on_click(cx.listener(move |t, _, w, cx| {
                            cx.stop_propagation();
                            t.request_close(ix, None, w, cx)
                        })),
                    )
                })
        });
        // Room for the window buttons, which `window_controls` draws above everything.
        let room = || div().flex_none().w(px(CONTROLS_W));
        let (lead, trail) = if left {
            (Some(room()), None)
        } else {
            (None, Some(room()))
        };
        div()
            .h(px(TITLEBAR_H))
            .flex_none()
            .flex()
            .bg(c(bg()))
            .border_b_1()
            .border_color(c(palette().border_variant))
            .children(lead)
            .child(
                div()
                    .id("home-tab")
                    .track_focus(&self.home_focus)
                    .focus(|s| s.bg(c(palette().active)))
                    .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, w, cx| {
                        t.strip_key(0, point(px(0.), px(0.)), ev, w, cx)
                    }))
                    .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                    .flex_none()
                    .h_full()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .border_x_1()
                    .border_color(c(palette().border_variant))
                    .relative()
                    .when(self.page == Page::Home, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .h(px(2.))
                                .bg(c(accent())),
                        )
                    })
                    .hover(|s| s.bg(c(palette().panel)))
                    .tooltip(crate::widgets::tip("Hosts"))
                    .on_click(cx.listener(|t, _, w, cx| t.show_page(Page::Home, w, cx)))
                    .child(icon(
                        "hat",
                        16.,
                        if self.page == Page::Home {
                            accent_text()
                        } else {
                            palette().muted
                        },
                    )),
            )
            // Tabs shrink to a minimum width, then the strip scrolls sideways. It never
            // pushes the buttons on the right out of the window.
            .child(
                div()
                    .id("tabs")
                    .flex()
                    .h_full()
                    .min_w_0()
                    .flex_shrink(1.)
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs),
            )
            .child(crate::platform::title_bar(
                div().id("title-drag").flex_1().min_w(px(32.)).h_full(),
            ))
            .child(
                div()
                    .flex_none()
                    .h_full()
                    .px(px(4.))
                    .flex()
                    .items_center()
                    // Touching buttons share their edge pixel, and a click there hit both.
                    .gap(px(1.))
                    .children(self.latency_label())
                    .when(
                        prefs.show_split_button()
                            && self.page == Page::Tab
                            && matches!(
                                self.tabs.get(self.active).map(|t| &t.kind),
                                Some(TabKind::Terminal(_))
                            ),
                        |d| {
                            d.child(
                                icon_button(
                                    "split-btn",
                                    "columns",
                                    format!(
                                        "Split terminal ({} splits the same server, {} picks one)",
                                        crate::shortcuts::label("split"),
                                        crate::shortcuts::label("split_menu")
                                    ),
                                    palette().muted,
                                )
                                .on_click(cx.listener(
                                    |t, ev: &ClickEvent, w, cx| {
                                        t.open_split_menu(Some(ev.position()), w, cx)
                                    },
                                )),
                            )
                        },
                    )
                    .when(prefs.show_quick_connect_button(), |d| {
                        d.child(
                            icon_button(
                                "quick-btn",
                                "arrow-right",
                                format!(
                                    "Quick connect ({})",
                                    crate::shortcuts::label("quick_connect")
                                ),
                                palette().muted,
                            )
                            .on_click(cx.listener(|t, _, w, cx| t.open_quick(w, cx))),
                        )
                    })
                    .when(prefs.show_privacy_button(), |d| {
                        d.child(
                            icon_button(
                                "hide-btn",
                                if privacy::hide() { "eye-off" } else { "eye" },
                                "Hide sensitive information",
                                if privacy::hide() {
                                    accent_text()
                                } else {
                                    palette().muted
                                },
                            )
                            .on_click(cx.listener(|t, _, _, cx| t.toggle_hide(cx))),
                        )
                    })
                    .child(
                        icon_button(
                            "help-btn",
                            "keyboard",
                            format!("Keyboard shortcuts ({})", crate::shortcuts::label("help")),
                            if self.help {
                                accent_text()
                            } else {
                                palette().muted
                            },
                        )
                        .on_click(cx.listener(|t, _, w, cx| t.toggle_help(w, cx))),
                    )
                    .child(
                        icon_button(
                            "settings-btn",
                            "sliders",
                            if self.page == Page::Settings {
                                "Close settings (Esc)".to_string()
                            } else {
                                format!("Settings ({})", crate::shortcuts::label("settings"))
                            },
                            if self.page == Page::Settings {
                                accent_text()
                            } else {
                                palette().muted
                            },
                        )
                        .on_click(cx.listener(|t, _, w, cx| t.toggle_settings(w, cx))),
                    ),
            )
            .children(trail)
    }

    /// The window buttons, at the end of the tab bar picked in settings. They sit above the
    /// dialogs, so the window can still be minimized or closed while one is open. On the
    /// left they go close first, like macOS; on the right close last, like Windows.
    pub(super) fn window_controls() -> impl IntoElement {
        let left = crate::prefs::get().window_controls() == WindowControls::Left;
        let min = win_button("win-min", "minus", palette().active, Min, |w, _| {
            w.minimize_window()
        });
        let max = win_button("win-max", "maximize", palette().active, Max, |w, _| {
            w.zoom_window()
        });
        // Closing the last window quits, so it goes through the same check.
        let close = win_button("win-close", "x", palette().red, Close, |w, cx| {
            if let Some(Some(root)) = w.root::<Workspace>() {
                root.update(cx, |r, cx| r.close_window(w, cx));
            }
        });
        div()
            .absolute()
            .top_0()
            .map(|d| if left { d.left_0() } else { d.right_0() })
            .w(px(CONTROLS_W))
            // Above the tab bar's bottom border, not over it.
            .h(px(TITLEBAR_H - 1.))
            .flex()
            .map(|d| if left { d.border_r_1() } else { d.border_l_1() })
            .border_color(c(palette().border_variant))
            .bg(c(bg()))
            // A click here is not a click outside an open dialog.
            .occlude()
            .children(if left {
                [close, min, max]
            } else {
                [min, max, close]
            })
    }

    /// The page under the tab bar. `width` is the window's, for the hosts page layout.
    pub(super) fn content(&self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        match (self.page, &self.settings) {
            (Page::Settings, Some(v)) => return v.clone().into_any_element(),
            (Page::Tab, _) if !self.tabs.is_empty() => {}
            _ => return self.home_view(width, cx),
        }
        match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Terminal(t)) if t.panes.len() == 1 => t.panes[0].element(),
            Some(TabKind::Terminal(t)) => self.split_view(t, cx),
            Some(TabKind::Docker(v)) => v.clone().into_any_element(),
            Some(TabKind::Sftp(v)) => v.clone().into_any_element(),
            Some(TabKind::Logs(v)) => v.clone().into_any_element(),
            None => div()
                .size_full()
                .flex()
                .flex_col()
                .gap_2()
                .items_center()
                .justify_center()
                .text_color(c(palette().placeholder))
                .child(icon("terminal", 28., palette().active))
                .child("Pick a host to open a shell")
                .into_any_element(),
        }
    }
}

/// Square, full-height window control for the custom title bar.
fn win_button(
    id: &'static str,
    name: &str,
    hover: u32,
    area: WindowControlArea,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let el = keyboard(div().id(id))
        .focus(move |s| s.bg(c(hover)))
        .w(px(40.))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(c(hover)))
        // The buttons touch: on their shared edge pixel only the top one takes the click.
        .occlude()
        .child(icon(name, 14., palette().text));
    crate::platform::window_button(el, area, action)
}
