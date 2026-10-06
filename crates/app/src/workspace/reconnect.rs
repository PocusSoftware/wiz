//! Terminals whose shell ended: close them, or reconnect when the connection dropped.

use super::*;

/// Seconds to wait before reconnect attempt `n` (0-based).
fn backoff(n: u32) -> u64 {
    [1, 2, 4, 8, 15, 30][n.min(5) as usize]
}

impl Workspace {
    pub(super) fn on_terminal_event(
        &mut self,
        view: gpui::WeakEntity<TerminalView>,
        ev: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            TerminalEvent::Ended(status) => self.on_shell_ended(view, *status, window, cx),
            TerminalEvent::Retry if !secrets::ready() => self.with_secrets(
                move |this, window, cx| this.start_reconnect(view, 0, None, true, window, cx),
                window,
                cx,
            ),
            TerminalEvent::Retry => self.start_reconnect(view, 0, None, true, window, cx),
            _ => {}
        }
    }

    /// A shell ended. If its connection dropped, reconnect; if the shell just exited (`exit`,
    /// Ctrl+D), close the terminal, or leave "Session closed" when that is turned off.
    fn on_shell_ended(
        &mut self,
        view: gpui::WeakEntity<TerminalView>,
        status: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            // A dropping connection can lag behind its channels. With an exit status the shell
            // most likely just exited, so look only briefly to keep `exit` snappy.
            let (polls, step) = match status {
                Some(_) => (3, Duration::from_millis(100)),
                None => (20, Duration::from_millis(250)),
            };
            let mut dropped = false;
            for _ in 0..polls {
                cx.background_executor().timer(step).await;
                dropped = this
                    .update(cx, |this, cx| this.connection_dropped(&view, cx))
                    .unwrap_or(false);
                if dropped {
                    break;
                }
            }
            this.update_in(cx, |this, window, cx| {
                if dropped {
                    this.start_reconnect(view, 0, None, false, window, cx);
                } else if crate::prefs::get().close_on_exit() {
                    this.close_ended(&view, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Close the pane (or tab) of a terminal whose shell exited. No question: it is over.
    fn close_ended(
        &mut self,
        view: &gpui::WeakEntity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let found = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(i, t)| match &t.kind {
                TabKind::Terminal(tt) => tt
                    .panes
                    .iter()
                    .position(|p| p.entity_id() == view.entity_id())
                    .map(|p| (i, p)),
                _ => None,
            });
        if let Some((ix, pane)) = found {
            self.do_close(ix, Some(pane), window, cx);
        }
    }

    /// Wait out the backoff for `attempt`, then try to reopen the terminal's shell. The
    /// terminal shows a countdown meanwhile.
    pub(super) fn start_reconnect(
        &mut self,
        view: gpui::WeakEntity<TerminalView>,
        attempt: u32,
        last_error: Option<String>,
        manual: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // With the setting off, only an explicit Enter reconnects.
        if !manual && !crate::prefs::get().auto_reconnect() {
            let text = match &last_error {
                Some(e) => format!("Connection lost ({e}). Press Enter to reconnect."),
                None => "Connection lost. Press Enter to reconnect.".into(),
            };
            view.update(cx, |v, cx| v.set_notice(Some(text), true, cx))
                .ok();
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            for left in (1..=backoff(attempt)).rev() {
                let text = match &last_error {
                    Some(e) => format!("Connection lost ({e}). Retrying in {left}s..."),
                    None => format!("Connection lost. Reconnecting in {left}s..."),
                };
                if view
                    .update(cx, |v, cx| v.set_notice(Some(text), false, cx))
                    .is_err()
                {
                    return; // the terminal was closed
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
            this.update_in(cx, |this, window, cx| {
                this.try_reattach(view, attempt, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// Open a fresh shell on `conn` for a terminal whose connection dropped.
    pub(super) fn reattach(
        &mut self,
        view: gpui::WeakEntity<TerminalView>,
        attempt: u32,
        conn: Arc<Connection>,
        password: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The terminal was closed while reconnecting: don't open a shell for nobody.
        let Ok(cmd) = view.read_with(cx, |v, _| v.command.clone()) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let c = conn.clone();
            let r =
                runtime::spawn(async move { c.open_shell(100, 30, cmd.as_deref()).await }).await;
            this.update_in(cx, |this, window, cx| match r {
                Ok(shell) => {
                    // Point the tab at the new connection.
                    for t in this.tabs.iter_mut() {
                        if let TabKind::Terminal(tt) = &mut t.kind
                            && tt.panes.iter().any(|p| p.entity_id() == view.entity_id())
                        {
                            tt.conn = conn.clone();
                            tt.password = password.clone();
                        }
                    }
                    view.update(cx, |v, cx| v.attach(shell, password.clone(), cx))
                        .ok();
                    cx.notify();
                }
                Err(e) => {
                    this.start_reconnect(view, attempt + 1, Some(e.to_string()), false, window, cx)
                }
            })
            .ok();
        })
        .detach();
    }

    /// The tab holding `view`: its index and host account.
    fn terminal_tab(
        &self,
        view: &gpui::WeakEntity<TerminalView>,
    ) -> Option<(usize, String, Arc<Connection>)> {
        self.tabs
            .iter()
            .enumerate()
            .find_map(|(i, t)| match &t.kind {
                TabKind::Terminal(tt)
                    if tt.panes.iter().any(|p| p.entity_id() == view.entity_id()) =>
                {
                    Some((i, tt.account.clone(), tt.conn.clone()))
                }
                _ => None,
            })
    }

    fn connection_dropped(
        &self,
        view: &gpui::WeakEntity<TerminalView>,
        _: &mut Context<Self>,
    ) -> bool {
        self.terminal_tab(view)
            .is_some_and(|(_, _, conn)| conn.is_closed())
    }

    fn try_reattach(
        &mut self,
        view: gpui::WeakEntity<TerminalView>,
        attempt: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, account, _)) = self.terminal_tab(&view) else {
            return; // closed meanwhile
        };
        let found = match self.cfg.hosts.iter().find(|h| h.account() == account) {
            Some(h) => Some((h.clone(), None)),
            None => self
                .quick
                .get(&account)
                .map(|(h, c)| (h.clone(), Some(c.clone()))),
        };
        let Some((host, creds)) = found else {
            view.update(cx, |v, cx| {
                v.set_notice(
                    Some("Connection lost. The host was removed.".into()),
                    false,
                    cx,
                )
            })
            .ok();
            return;
        };
        view.update(cx, |v, cx| {
            v.set_notice(Some("Reconnecting...".into()), false, cx)
        })
        .ok();
        self.connect(host, creds, Open::Reattach(view, attempt), window, cx);
    }
}
