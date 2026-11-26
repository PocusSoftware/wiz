//! The hosts page: a searchable list of hosts, and the details of the one picked.

mod details;

use std::collections::HashSet;

use gpui::{KeyDownEvent, MouseButton, MouseDownEvent};

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
    /// A folder, by name.
    Group(String),
    /// Neither pinned nor in a folder.
    Other,
}

/// Whether `h` matches the search: its name, address, folder or a tag contains `query`, ignoring
/// case. An empty search matches every host.
pub(super) fn matches(h: &Host, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || [Some(&h.name), Some(&h.account()), h.group.as_ref()]
            .into_iter()
            .flatten()
            .chain(&h.tags)
            .any(|s| s.to_lowercase().contains(&q))
}

/// The hosts matching `query` as indices into `hosts`, by section: pinned first, then
/// each folder by name, then the rest. Empty sections are left out.
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

/// Whether `section` shows only its title: a folded folder, unless a search is showing what
/// matches inside it.
pub(super) fn is_folded(section: &Section, folded: &HashSet<String>, query: &str) -> bool {
    query.trim().is_empty() && matches!(section, Section::Group(g) if folded.contains(g))
}

/// The hosts listed, in order: those of `sections` that are not folded away.
pub(super) fn listed(
    sections: &[(Section, Vec<usize>)],
    folded: &HashSet<String>,
    query: &str,
) -> Vec<usize> {
    sections
        .iter()
        .filter(|(s, _)| !is_folded(s, folded, query))
        .flat_map(|(_, ixs)| ixs.iter().copied())
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
            TabKind::Terminal(term) => term.panes.iter().any(|p| p.account() == Some(account)),
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
    /// The saved hosts the page lists, in order, after the search and folding.
    pub(super) fn shown(&self, cx: &App) -> Vec<usize> {
        let query = self.search.read(cx).value();
        listed(&sections(&self.cfg.hosts, query), &self.folded, query)
    }

    /// Fold folder `name` down to its title, or unfold it.
    fn toggle_folder(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.folded.remove(name) {
            self.folded.insert(name.to_string());
        }
        cx.notify();
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
    pub(super) fn home_view(&self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let wide = width >= px(WIDE_W);
        let query = self.search.read(cx).value().trim().to_string();
        let sections = sections(&self.cfg.hosts, &query);
        let shown = listed(&sections, &self.folded, &query);
        let picked = pick(&self.cfg.hosts, &shown, self.selected.as_deref());

        let rows = self.host_rows(&sections, &query, picked, wide, cx);

        let list = div()
            .h_full()
            .flex()
            .flex_col()
            .when(wide, |d| {
                d.w(px(LIST_W))
                    .flex_none()
                    .border_r_1()
                    .border_color(c(palette().border_variant))
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
                        icon_button("add-host", "plus", "Add host", palette().muted)
                            .on_click(cx.listener(|t, _, w, cx| t.open_form(None, w, cx))),
                    ),
            )
            .child(
                div()
                    .id("home-list")
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|t, ev: &MouseDownEvent, w, cx| {
                            t.open_hosts_menu(ev.position, None, w, cx)
                        }),
                    )
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

    /// The list under the search: section and folder titles and host rows, or what Enter
    /// would do when nothing matches.
    fn host_rows(
        &self,
        sections: &[(Section, Vec<usize>)],
        query: &str,
        picked: Option<usize>,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows: Vec<AnyElement> = Vec::new();
        let titled = sections.len() > 1;
        for (section, ixs) in sections {
            if let Section::Group(g) = section {
                let folded = is_folded(section, &self.folded, query);
                rows.push(
                    self.folder_title(g, ixs.len(), folded, cx)
                        .into_any_element(),
                );
                if folded {
                    continue;
                }
            }
            let title = match section {
                Section::Pinned => "Pinned",
                Section::Other if titled => "Hosts",
                _ => "",
            };
            if !title.is_empty() {
                rows.push(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(c(palette().placeholder))
                        .child(title)
                        .into_any_element(),
                );
            }
            let nested = matches!(section, Section::Group(_));
            for &ix in ixs {
                let row = self
                    .host_row(ix, wide && picked == Some(ix), wide, cx)
                    .when(nested, |d| d.pl(px(22.)));
                // Cascade only on launch; coming back later just fades the page.
                rows.push(if self.epoch == 0 {
                    stagger_in(row, ("row", ix), rows.len()).into_any_element()
                } else {
                    row.into_any_element()
                });
            }
        }
        if sections.is_empty() && !query.is_empty() {
            rows.push(
                div()
                    .p_2()
                    .text_color(c(palette().placeholder))
                    .child(
                        privacy::redact(&format!(
                            "No hosts match. Press Enter to connect to {query}."
                        ))
                        .into_owned(),
                    )
                    .into_any_element(),
            );
        }
        rows
    }

    /// The title of a folder, folding or unfolding it on click.
    fn folder_title(
        &self,
        name: &str,
        count: usize,
        folded: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let key = name.to_string();
        div()
            .id(SharedString::from(format!("folder-{name}")))
            .mt_1()
            .h(px(26.))
            .px_2()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .text_xs()
            .text_color(c(palette().muted))
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .on_click(cx.listener(move |t, _, _, cx| t.toggle_folder(&key, cx)))
            .child(icon(
                if folded {
                    "chevron-right"
                } else {
                    "chevron-down"
                },
                10.,
                palette().placeholder,
            ))
            .child(icon("folder", 12., palette().muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(privacy::redact(name).into_owned()),
            )
            .child(
                div()
                    .text_color(c(palette().placeholder))
                    .child(count.to_string()),
            )
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
            .when(picked, |d| d.bg(c(palette().active)))
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .on_mouse_down(MouseButton::Right, {
                let account = account.clone();
                cx.listener(move |t, ev: &MouseDownEvent, w, cx| {
                    cx.stop_propagation();
                    t.selected = Some(account.clone());
                    t.open_hosts_menu(ev.position, Some(account.clone()), w, cx);
                })
            })
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
                            .child(privacy::redact(&h.name).into_owned()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(c(palette().placeholder))
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
                        .text_color(c(palette().muted))
                        .when(open > 0, |d| d.child(open.to_string()))
                        .child(div().size(px(7.)).bg(c(dot_color(if live {
                            palette().green
                        } else {
                            palette().yellow
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
            .hover(|s| s.bg(c(palette().hover)))
            .on_click(cx.listener(|t, _, w, cx| t.open_local_docker(w, cx)))
            .child(icon("container", 16., palette().muted))
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Local Docker"),
            )
    }
}
