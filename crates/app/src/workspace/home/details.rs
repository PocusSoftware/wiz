//! The right side of the hosts page: what can be opened on the picked host, its open
//! tabs, and how it connects.

use super::*;

impl Workspace {
    /// What the details show before any host is saved.
    pub(super) fn first_host(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .child(icon("server", 28., palette().active))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Add your first host"),
            )
            .child(
                div()
                    .text_color(c(palette().muted))
                    .child("Save a server to connect with one click, or type user@host to the left to connect once."),
            )
            .child(
                primary_button("first-host", "Add host")
                    .on_click(cx.listener(|t, _, w, cx| t.open_form(None, w, cx))),
            )
            .into_any_element()
    }

    /// The picked host: what can be opened on it, its open tabs, and how it connects.
    pub(super) fn details(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let h = &self.cfg.hosts[ix];
        let account = h.account();
        let live = self
            .conns
            .get(&account)
            .is_some_and(|l| !l.conn.is_closed());
        let subtitle = [
            logo(h, crate::prefs::get().show_os_logos()).map(|l| l.title.to_string()),
            h.group.as_deref().map(|g| privacy::redact(g).into_owned()),
            live.then(|| "Connected".to_string()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let heading = |text: &str| {
            div()
                .mt_5()
                .mb_1()
                .text_xs()
                .text_color(c(palette().placeholder))
                .child(text.to_string())
        };
        let open: Vec<AnyElement> = tabs_of(&self.tabs, &account)
            .into_iter()
            .map(|ti| self.open_tab_row(ti, cx))
            .collect();

        div()
            .p_5()
            .max_w(px(560.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(icon(
                        &card_icon(h, crate::prefs::get().show_os_logos()),
                        28.,
                        accent_text(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(privacy::redact(&h.name).into_owned()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_color(c(palette().muted))
                                    .child(subtitle),
                            ),
                    )
                    .child(
                        icon_button(
                            "d-pin",
                            "pin",
                            if h.pinned { "Unpin" } else { "Pin to top" },
                            if h.pinned {
                                accent_text()
                            } else {
                                palette().muted
                            },
                        )
                        .on_click(cx.listener(move |t, _, _, cx| t.toggle_pin(ix, cx))),
                    )
                    .child(
                        icon_button("d-edit", "pencil", "Edit host", palette().muted)
                            .on_click(cx.listener(move |t, _, w, cx| t.open_form(Some(ix), w, cx))),
                    )
                    .child(
                        icon_button("d-copy", "copy", "Duplicate host", palette().muted)
                            .on_click(cx.listener(move |t, _, w, cx| t.duplicate_host(ix, w, cx))),
                    )
                    .child(
                        icon_button("d-del", "trash", "Delete host", palette().red)
                            .on_click(cx.listener(move |t, _, w, cx| t.ask_delete_host(ix, w, cx))),
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        action("d-shell", "terminal", "New terminal", true).on_click(
                            cx.listener(move |t, _, w, cx| t.open_host(ix, Open::Shell, w, cx)),
                        ),
                    )
                    .child(
                        action("d-docker", "container", "Docker containers", false).on_click(
                            cx.listener(move |t, _, w, cx| t.open_host(ix, Open::Docker, w, cx)),
                        ),
                    )
                    .child(action("d-files", "folder", "Files (SFTP)", false).on_click(
                        cx.listener(move |t, _, w, cx| t.open_host(ix, Open::Sftp, w, cx)),
                    )),
            )
            .when(!open.is_empty(), |d| {
                d.child(heading("Open now"))
                    .child(div().flex().flex_col().gap_1().children(open))
            })
            .child(heading("Connection"))
            .child(self.connection(h))
            .into_any_element()
    }

    /// One open tab of the picked host. A click switches to it.
    fn open_tab_row(&self, ti: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[ti];
        let kind = match tab.kind {
            TabKind::Terminal(_) => "terminal",
            TabKind::Docker(_) => "container",
            TabKind::Sftp(_) => "folder",
            TabKind::Logs(_) => "logs",
        };
        let name = tab.name.clone().unwrap_or_else(|| tab.label.clone());
        keyboard(div().id(("open-tab", ti)))
            .h(px(30.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .border_1()
            .border_color(c(palette().border_variant))
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .on_click(cx.listener(move |t, _, w, cx| t.select_tab(ti, w, cx)))
            .child(icon(kind, 13., palette().green))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(privacy::redact(&name).into_owned()),
            )
            .child(div().text_color(c(palette().muted)).child("Switch to"))
            .child(icon("arrow-right", 12., palette().muted))
            .into_any_element()
    }

    /// Address, sign-in and jump hosts of `h`, as label and value rows.
    fn connection(&self, h: &Host) -> impl IntoElement {
        let sign_in = match &h.auth {
            AuthMethod::Password => "Password".to_string(),
            AuthMethod::Key { path } => {
                privacy::redact(&format!("Key, {}", path.display())).into_owned()
            }
            AuthMethod::Agent => "SSH agent".to_string(),
        };
        let through = h.jump.as_ref().map(|_| match self.cfg.jump_chain(h) {
            Ok(chain) => (
                chain
                    .iter()
                    .map(|hop| privacy::redact(&hop.name))
                    .chain([privacy::redact(&h.name)])
                    .collect::<Vec<_>>()
                    .join(" › "),
                palette().muted,
            ),
            Err(e) => (e, palette().red),
        });
        let row = |name: &str, value: String, color: u32| {
            div()
                .flex()
                .gap_2()
                .py_1()
                .child(
                    div()
                        .w(px(90.))
                        .flex_none()
                        .text_color(c(palette().placeholder))
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(c(color))
                        .child(value),
                )
        };
        div()
            .flex()
            .flex_col()
            .child(row("Address", address(h), palette().text))
            .child(row("Sign in", sign_in, palette().muted))
            .when_some(through, |d, (path, color)| {
                d.child(row("Through", path, color))
            })
    }
}

/// A big icon button for what can be opened on the host. The terminal, the usual pick,
/// is filled with the accent.
fn action(id: &'static str, name: &str, tooltip: &str, main: bool) -> gpui::Stateful<gpui::Div> {
    let (bg, fg, hover) = if main {
        (accent(), on_accent(), accent_text())
    } else {
        (palette().active, palette().text, palette().active_hover)
    };
    keyboard(div().id(id))
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .border_1()
        .border_color(c(palette().border))
        .bg(c(bg))
        .cursor_pointer()
        .hover(move |s| s.bg(c(hover)))
        .tooltip(crate::widgets::tip(tooltip.to_string()))
        .child(icon(name, 15., fg))
}
