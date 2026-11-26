//! The round trip to the server of the focused terminal, shown in the tab bar, so lag can be
//! told apart from the network.

use super::*;

/// How often the focused connection is pinged.
const EVERY: Duration = Duration::from_secs(3);
/// From here on the number shows in yellow.
const SLOW_MS: u32 = 150;

impl Workspace {
    /// The connection of the focused terminal pane, when a terminal tab is in front.
    fn focused_conn(&self) -> Option<Arc<Connection>> {
        if self.page != Page::Tab {
            return None;
        }
        match &self.tabs.get(self.active)?.kind {
            TabKind::Terminal(t) => t
                .panes
                .get(t.focused)
                .and_then(PaneView::shell)
                .map(|p| p.conn.clone()),
            _ => None,
        }
    }

    /// Ping the focused connection every few seconds, for as long as the window lives. Only
    /// one connection at a time, and only a changed number repaints.
    pub(super) fn watch_latency(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(EVERY).await;
                let Ok(conn) = this.update(cx, |t, _| {
                    t.focused_conn()
                        .filter(|_| crate::prefs::get().show_latency())
                }) else {
                    return; // the window closed
                };
                let ms = match conn.clone() {
                    Some(c) => runtime::spawn(async move { c.ping().await })
                        .await
                        .map(|d| d.as_millis().min(u32::MAX as u128) as u32),
                    None => None,
                };
                let shown = this.update(cx, |t, cx| {
                    // Another tab may be in front by now: its number is not this one.
                    let same = match (t.focused_conn(), &conn) {
                        (Some(a), Some(b)) => Arc::ptr_eq(&a, b),
                        _ => false,
                    };
                    let ms = ms.filter(|_| same);
                    if ms != t.latency {
                        t.latency = ms;
                        cx.notify();
                    }
                });
                if shown.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// "42 ms" in the tab bar, for the terminal in front.
    pub(super) fn latency_label(&self) -> Option<AnyElement> {
        let ms = self
            .latency
            .filter(|_| crate::prefs::get().show_latency() && self.focused_conn().is_some())?;
        Some(
            div()
                .id("latency")
                .px(px(6.))
                .text_xs()
                .text_color(c(if ms >= SLOW_MS {
                    palette().yellow
                } else {
                    palette().muted
                }))
                .tooltip(crate::widgets::tip(
                    "Round trip to this terminal's server. Typing feels at least this slow.",
                ))
                .child(if ms == 0 {
                    "<1 ms".to_string()
                } else {
                    format!("{ms} ms")
                })
                .into_any_element(),
        )
    }
}
