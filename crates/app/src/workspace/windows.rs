//! More than one window: moving a tab into a window of its own or back, and closing or
//! quitting with sessions open in any of them.

use gpui::{Bounds, WindowBounds, WindowHandle, WindowOptions, size};

use super::*;

impl Workspace {
    pub(super) fn watch_terminal(
        &mut self,
        view: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.observe(view, |_, _, cx| cx.notify()).detach();
        cx.subscribe_in(
            view,
            window,
            |this, view, ev: &TerminalEvent, window, cx| {
                this.on_terminal_event(view.downgrade(), ev, window, cx)
            },
        )
        .detach();
    }

    pub(super) fn other_windows(&self, cx: &App) -> Vec<WindowHandle<Workspace>> {
        cx.windows()
            .into_iter()
            .filter(|w| w.window_id() != self.window.window_id())
            .filter_map(|w| w.downcast::<Workspace>())
            .collect()
    }

    pub(super) fn open_sessions(&self, cx: &Context<Self>) -> usize {
        let own = self
            .tabs
            .iter()
            .filter(|t| Self::holds_session(t, cx))
            .count();
        let others: usize = self
            .other_windows(cx)
            .iter()
            .filter_map(|w| w.read(cx).ok())
            .map(|w| w.tabs.iter().filter(|t| Self::holds_session(t, cx)).count())
            .sum();
        own + others
    }

    fn take_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) -> Option<Moved> {
        let ix = self.tabs.iter().position(|t| t.id == id)?;
        let tab = self.tabs.remove(ix);
        let mut accounts: Vec<String> = tab.account.iter().cloned().collect();
        if let TabKind::Terminal(t) = &tab.kind {
            accounts.extend(
                t.panes
                    .iter()
                    .filter_map(|p| p.account().map(str::to_string)),
            );
        }
        let live = accounts
            .iter()
            .filter_map(|a| {
                let l = self.conns.get(a)?;
                Some((
                    a.clone(),
                    Live {
                        conn: l.conn.clone(),
                        password: l.password.clone(),
                    },
                ))
            })
            .collect();
        let quick = accounts
            .iter()
            .filter_map(|a| Some((a.clone(), self.quick.get(a)?.clone())))
            .collect();
        if self.tabs.is_empty() && !self.other_windows(cx).is_empty() {
            window.remove_window();
        } else {
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            if self.tabs.is_empty() {
                self.show_page(Page::Home, window, cx);
            } else {
                self.select_tab(self.active, window, cx);
            }
        }
        Some(Moved { tab, live, quick })
    }

    fn adopt(&mut self, moved: Moved, window: &mut Window, cx: &mut Context<Self>) {
        let Moved {
            mut tab,
            live,
            quick,
        } = moved;
        for (account, l) in live {
            self.conns.entry(account).or_insert(l);
        }
        self.quick.extend(quick);
        if let TabKind::Terminal(t) = &tab.kind {
            let views: Vec<_> = t
                .panes
                .iter()
                .filter_map(|p| p.shell().map(|p| p.view.clone()))
                .collect();
            for view in &views {
                self.watch_terminal(view, window, cx);
            }
        }
        tab.id = self.next_tab;
        self.next_tab += 1;
        self.tabs.push(tab);
        self.select_tab(self.tabs.len() - 1, window, cx);
        window.activate_window();
    }

    pub(super) fn pop_out(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(moved) = self.take_tab(id, window, cx) else {
            return;
        };
        let bounds = Bounds::centered(None, size(px(1000.), px(650.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..crate::platform::window_options()
            },
            |window, cx| {
                let workspace = cx.new(|cx| {
                    let mut w = Workspace::new(window, cx);
                    w.adopt(moved, window, cx);
                    w
                });
                crate::platform::on_window_open(window, &workspace, cx);
                workspace
            },
        );
        if let Err(e) = opened {
            self.set_status(format!("Could not open a window: {e}"), true);
            cx.notify();
        }
    }

    pub(super) fn move_to(
        &mut self,
        id: u64,
        to: WindowHandle<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(moved) = self.take_tab(id, window, cx) else {
            return;
        };
        to.update(cx, |w, window, cx| w.adopt(moved, window, cx))
            .ok();
    }

    pub fn should_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.other_windows(cx).is_empty() {
            self.request_quit(window, cx);
            false
        } else {
            true
        }
    }

    pub(super) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close_window(window, cx) {
            window.remove_window();
        }
    }
}

struct Moved {
    tab: Tab,
    live: Vec<(String, Live)>,
    quick: Vec<(String, (Host, Credentials))>,
}
