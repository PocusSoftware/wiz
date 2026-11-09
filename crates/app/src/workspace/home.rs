//! The hosts page: a searchable list of hosts, and the details of the one picked.

use gpui::KeyDownEvent;

use super::hosts::{card_icon, logo};
use super::*;

/// Below this page width the details hide, and a click on a host opens a terminal.
const WIDE_W: f32 = 640.;
/// Width of the host list next to the details.
const LIST_W: f32 = 260.;

/// Where a host is listed on the hosts page.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Section {
    Pinned,
    Group(String),
    /// Neither pinned nor in a group.
    Other,
}

/// Whether `h` matches the search: its name, address or group contains `query`, ignoring
/// case. An empty search matches every host.
pub(super) fn matches(h: &Host, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || [Some(&h.name), Some(&h.account()), h.group.as_ref()]
            .into_iter()
            .flatten()
            .any(|s| s.to_lowercase().contains(&q))
}

/// The hosts matching `query` as indices into `hosts`, by section: pinned first, then
/// each group by name, then the rest. Empty sections are left out.
pub(super) fn sections(hosts: &[Host], query: &str) -> Vec<(Section, Vec<usize>)> {
    let section = |h: &Host| match (&h.group, h.pinned) {
        (_, true) => Section::Pinned,
        (Some(g), false) => Section::Group(g.clone()),
        (None, false) => Section::Other,
    };
    let shown: Vec<usize> = (0..hosts.len())
        .filter(|&i| matches(&hosts[i], query))
        .collect();
    let mut groups: Vec<&String> = shown
        .iter()
        .filter(|&&i| !hosts[i].pinned)
        .filter_map(|&i| hosts[i].group.as_ref())
        .collect();
    groups.sort_by_key(|g| g.to_lowercase());
    groups.dedup();
    std::iter::once(Section::Pinned)
        .chain(groups.into_iter().map(|g| Section::Group(g.clone())))
        .chain(std::iter::once(Section::Other))
        .filter_map(|s| {
            let members: Vec<usize> = shown
                .iter()
                .copied()
                .filter(|&i| section(&hosts[i]) == s)
                .collect();
            (!members.is_empty()).then_some((s, members))
        })
        .collect()
}

/// The host to show details for: the one with account `selected` if it is listed, else
/// the first one listed.
pub(super) fn pick(hosts: &[Host], shown: &[usize], selected: Option<&str>) -> Option<usize> {
    shown
        .iter()
        .copied()
        .find(|&i| Some(hosts[i].account().as_str()) == selected)
        .or(shown.first().copied())
}

/// Tabs showing host `account`: terminals with a pane on it, and its Docker and files tabs.
pub(super) fn tabs_of(tabs: &[Tab], account: &str) -> Vec<usize> {
    tabs.iter()
        .enumerate()
        .filter(|(_, t)| match &t.kind {
            TabKind::Terminal(term) => term.panes.iter().any(|p| p.account == account),
            _ => t.account.as_deref() == Some(account),
        })
        .map(|(i, _)| i)
        .collect()
}

/// `user@host:port`, masked by "hide sensitive information".
fn address(h: &Host) -> String {
    privacy::redact(&format!("{}@{}:{}", h.user, h.host, privacy::port(h.port))).into_owned()
}

impl Workspace {
    /// The saved hosts the page lists, in order, after the search.
    pub(super) fn shown(&self, cx: &App) -> Vec<usize> {
        sections(&self.cfg.hosts, self.search.read(cx).value())
            .into_iter()
            .flat_map(|(_, ixs)| ixs)
            .collect()
    }

    /// Keys on the hosts page that no control took. Up and Down pick a host, Enter opens a
    /// terminal on it (or quick connects to what was typed when no host matches), Escape
    /// clears the search. True when the key was used.
    pub(super) fn home_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if ev.keystroke.modifiers.modified() {
            return false;
        }
        let shown = self.shown(cx);
        let current = pick(&self.cfg.hosts, &shown, self.selected.as_deref());
        match ev.keystroke.key.as_str() {
            key @ ("up" | "down") => {
                let Some(cur) = current else {
                    return false;
                };
                let pos = shown.iter().position(|&i| i == cur).unwrap_or(0);
                let next = if key == "up" {
                    pos.saturating_sub(1)
                } else {
                    (pos + 1).min(shown.len() - 1)
                };
                self.selected = Some(self.cfg.hosts[shown[next]].account());
                cx.notify();
            }
            "enter" => match current {
                Some(ix) => self.open_host(ix, Open::Shell, window, cx),
                None => {
                    let typed = self.search.read(cx).value().trim().to_string();
                    if typed.is_empty() {
                        return false;
                    }
                    self.clear_search(cx);
                    self.open_quick(window, cx);
                    if let Some(d) = &self.quick_form {
                        d.update(cx, |d, cx| d.set_target(&typed, cx));
                    }
                }
            },
            "escape" if !self.search.read(cx).value().is_empty() => self.clear_search(cx),
            _ => return false,
        }
        true
    }

    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| {
            s.set_value("");
            cx.notify();
        });
        cx.notify();
    }

    /// The hosts list beside the details of the picked host. A narrow window shows only
    /// the list, and a click opens a terminal right away.
    pub(super) fn home_view(&self, cx: &mut Context<Self>) -> AnyElement {
        // Zero before the first frame is drawn: assume there is room.
        let width = self.content_bounds.get().size.width;
        let wide = width == px(0.) || width >= px(WIDE_W);
        let query = self.search.read(cx).value().trim().to_string();
        let sections = sections(&self.cfg.hosts, &query);
        let shown: Vec<usize> = sections.iter().flat_map(|(_, ixs)| ixs.clone()).collect();
        let picked = pick(&self.cfg.hosts, &shown, self.selected.as_deref());

        let mut rows: Vec<AnyElement> = Vec::new();
        let titled = sections.len() > 1;
        for (section, ixs) in &sections {
            let title = match section {
                Section::Pinned => "Pinned".to_string(),
                Section::Group(g) => privacy::redact(g).into_owned(),
                Section::Other if titled => "Hosts".to_string(),
                Section::Other => String::new(),
            };
            if !title.is_empty() {
                rows.push(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(c(PLACEHOLDER))
                        .child(title)
                        .into_any_element(),
                );
            }
            for &ix in ixs {
                let row = self.host_row(ix, wide && picked == Some(ix), wide, cx);
                // Cascade only on launch; coming back later just fades the page.
                rows.push(if self.epoch == 0 {
                    stagger_in(row, ("row", ix), rows.len()).into_any_element()
                } else {
                    row.into_any_element()
                });
            }
        }
        if shown.is_empty() && !query.is_empty() {
            rows.push(
                div()
                    .p_2()
                    .text_color(c(PLACEHOLDER))
                    .child(
                        privacy::redact(&format!(
                            "No hosts match. Press Enter to connect to {query}."
                        ))
                        .into_owned(),
                    )
                    .into_any_element(),
            );
        }

        let list = div()
            .h_full()
            .flex()
            .flex_col()
            .when(wide, |d| {
                d.w(px(LIST_W))
                    .flex_none()
                    .border_r_1()
                    .border_color(c(BORDER_VARIANT))
            })
            .when(!wide, Styled::flex_1)
            .child(
                div()
                    .p_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().flex_1().min_w_0().child(self.search.clone()))
                    .child(
                        icon_button("add-host", "plus", "Add host", MUTED)
                            .on_click(cx.listener(|t, _, w, cx| t.open_form(None, w, cx))),
                    ),
            )
            .child(
                div()
                    .id("home-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_1()
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .child(self.local_row(cx));

        div()
            .id("home")
            .size_full()
            .flex()
            .child(list)
            .when(wide, |d| {
                d.child(
                    div()
                        .id("home-details")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .child(match picked {
                            Some(ix) => self.details(ix, cx),
                            None if self.cfg.hosts.is_empty() => self.first_host(cx),
                            None => div().into_any_element(),
                        }),
                )
            })
            .into_any_element()
    }

    /// One host in the list: logo, name, address, and a dot while it is connected.
    fn host_row(
        &self,
        ix: usize,
        picked: bool,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let h = &self.cfg.hosts[ix];
        let account = h.account();
        let live = self
            .conns
            .get(&account)
            .is_some_and(|l| !l.conn.is_closed());
        let connecting = self.pending.iter().any(|p| p.account == account);
        let open = tabs_of(&self.tabs, &account).len();
        div()
            .id(("host", ix))
            .h(px(42.))
            .px_2()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .border_l_2()
            .border_color(if picked {
                c(accent())
            } else {
                gpui::transparent_black()
            })
            .when(picked, |d| d.bg(c(ACTIVE)))
            .cursor_pointer()
            .hover(|s| s.bg(c(HOVER)))
            .on_click(cx.listener(move |t, ev: &ClickEvent, w, cx| {
                if !wide || ev.click_count() >= 2 {
                    t.open_host(ix, Open::Shell, w, cx);
                } else {
                    t.selected = Some(account.clone());
                    cx.notify();
                }
            }))
            .child(icon(
                &card_icon(h, crate::prefs::get().show_os_logos()),
                16.,
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
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(h.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(c(PLACEHOLDER))
                            .child(address(h)),
                    ),
            )
            .when(live || connecting, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(c(MUTED))
                        .when(open > 0, |d| d.child(open.to_string()))
                        .child(div().size(px(7.)).bg(c(dot_color(if live {
                            GREEN
                        } else {
                            YELLOW
                        })))),
                )
            })
    }

    /// The computer Wiz runs on, for its Docker containers, at the bottom of the list.
    fn local_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        keyboard(div().id("local-docker"))
            .m_1()
            .h(px(36.))
            .px_2()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .hover(|s| s.bg(c(HOVER)))
            .on_click(cx.listener(|t, _, w, cx| t.open_local_docker(w, cx)))
            .child(icon("container", 16., MUTED))
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(crate::platform::THIS_COMPUTER),
            )
            .child(div().text_xs().text_color(c(PLACEHOLDER)).child("Docker"))
    }

    /// What the details show before any host is saved.
    fn first_host(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .child(icon("server", 28., ACTIVE))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Add your first host"),
            )
            .child(
                div()
                    .text_color(c(MUTED))
                    .child("Save a server to connect with one click, or type user@host to the left to connect once."),
            )
            .child(
                primary_button("first-host", "Add host")
                    .on_click(cx.listener(|t, _, w, cx| t.open_form(None, w, cx))),
            )
            .into_any_element()
    }

    /// The picked host: what can be opened on it, its open tabs, and how it connects.
    fn details(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
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
                .text_color(c(PLACEHOLDER))
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
                                    .child(h.name.clone()),
                            )
                            .child(div().truncate().text_color(c(MUTED)).child(subtitle)),
                    )
                    .child(
                        icon_button(
                            "d-pin",
                            "pin",
                            if h.pinned { "Unpin" } else { "Pin to top" },
                            if h.pinned { accent_text() } else { MUTED },
                        )
                        .on_click(cx.listener(move |t, _, _, cx| t.toggle_pin(ix, cx))),
                    )
                    .child(
                        icon_button("d-edit", "pencil", "Edit host", MUTED)
                            .on_click(cx.listener(move |t, _, w, cx| t.open_form(Some(ix), w, cx))),
                    )
                    .child(
                        icon_button("d-del", "trash", "Delete host", RED)
                            .on_click(cx.listener(move |t, _, w, cx| t.ask_delete_host(ix, w, cx))),
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(primary_button("d-shell", "New terminal").on_click(
                        cx.listener(move |t, _, w, cx| t.open_host(ix, Open::Shell, w, cx)),
                    ))
                    .child(button("d-docker", "Docker").on_click(
                        cx.listener(move |t, _, w, cx| t.open_host(ix, Open::Docker, w, cx)),
                    ))
                    .child(button("d-files", "Files").on_click(
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
            .border_color(c(BORDER_VARIANT))
            .cursor_pointer()
            .hover(|s| s.bg(c(HOVER)))
            .on_click(cx.listener(move |t, _, w, cx| t.select_tab(ti, w, cx)))
            .child(icon(kind, 13., GREEN))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(privacy::redact(&name).into_owned()),
            )
            .child(div().text_color(c(MUTED)).child("Switch to"))
            .child(icon("arrow-right", 12., MUTED))
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
                    .map(|hop| hop.name.as_str())
                    .chain([h.name.as_str()])
                    .collect::<Vec<_>>()
                    .join(" › "),
                MUTED,
            ),
            Err(e) => (e, RED),
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
                        .text_color(c(PLACEHOLDER))
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
            .child(row("Address", address(h), TEXT))
            .child(row("Sign in", sign_in, MUTED))
            .when_some(through, |d, (path, color)| {
                d.child(row("Through", path, color))
            })
    }
}
