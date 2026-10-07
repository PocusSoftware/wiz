//! The hosts page and adding, editing, pinning and deleting hosts.

use super::*;

impl Workspace {
    /// Add or edit a host. Saving touches secrets, so a locked secrets file is unlocked first.
    pub(super) fn open_form(
        &mut self,
        existing: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_secrets(
            move |this, window, cx| this.show_form(existing, window, cx),
            window,
            cx,
        );
    }

    fn show_form(&mut self, existing: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let ex = existing.and_then(|i| self.cfg.hosts.get(i).cloned().map(|h| (i, h)));
        let taken = self
            .cfg
            .hosts
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != existing)
            .map(|(_, h)| h.account())
            .collect();
        let groups = self
            .cfg
            .hosts
            .iter()
            .filter_map(|h| h.group.clone())
            .collect();
        let form = cx.new(|cx| HostForm::new(ex, taken, groups, cx));
        cx.subscribe_in(&form, window, |this, _, ev: &FormEvent, window, cx| {
            match ev {
                FormEvent::Saved { host, index } => {
                    match index {
                        Some(i) if *i < this.cfg.hosts.len() => this.cfg.hosts[*i] = host.clone(),
                        _ => this.cfg.hosts.push(host.clone()),
                    }
                    this.save_config();
                }
                FormEvent::Cancelled => {}
            }
            this.form = None;
            window.focus(&this.focus);
            cx.notify();
        })
        .detach();
        form.read(cx).focus_first(window, cx);
        self.form = Some(form);
        cx.notify();
    }

    fn ask_delete_host(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_delete = Some(ix);
        // Take keyboard focus so Enter and Esc reach the dialog.
        window.focus(&self.focus);
        cx.notify();
    }

    pub(super) fn answer_delete_host(
        &mut self,
        yes: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.confirm_delete.take() else {
            return;
        };
        if yes {
            // Its secrets go too, so the secrets file must be open.
            let acct = self.cfg.hosts.get(ix).map(Host::account);
            self.with_secrets(
                move |this, _, cx| {
                    // Find it again: the list may have changed while the dialog was up.
                    if let Some(ix) = this
                        .cfg
                        .hosts
                        .iter()
                        .position(|h| Some(h.account()) == acct)
                    {
                        this.delete_host(ix, cx);
                    }
                },
                window,
                cx,
            );
        }
        cx.notify();
    }

    fn delete_host(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.cfg.hosts.len() {
            return;
        }
        let h = self.cfg.hosts.remove(ix);
        let acct = h.account();
        self.cancel_connect(&acct, cx);
        for kind in [Kind::Password, Kind::KeyPassphrase] {
            if let Err(e) = secrets::delete(kind, &acct) {
                self.set_status(format!("Could not remove saved secrets: {e}"), true);
            }
        }
        self.conns.remove(&acct);
        self.save_config();
        cx.notify();
    }

    fn toggle_pin(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(h) = self.cfg.hosts.get_mut(ix) {
            h.pinned = !h.pinned;
            self.save_config();
            cx.notify();
        }
    }

    /// One host as a big square card. `ix` is the index in `cfg.hosts`, whatever section the
    /// card is shown in.
    fn host_card(&self, ix: usize, h: &Host, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let account = h.account();
        let live = self
            .conns
            .get(&account)
            .is_some_and(|l| !l.conn.is_closed());
        let connecting = self.pending.iter().any(|p| p.account == account);
        let kind_icon = match h.auth {
            AuthMethod::Password => "server",
            AuthMethod::Key { .. } => "key",
        };
        let dot = if live {
            GREEN
        } else if connecting {
            YELLOW
        } else {
            ACTIVE
        };
        div()
            .id(("host", ix))
            .w(px(210.))
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(2.))
            .border_2()
            .border_color(c(BORDER))
            .bg(c(PANEL))
            .cursor_pointer()
            .hover(|s| s.bg(c(ACTIVE)).border_color(accent_alpha(0.4)))
            .on_click(
                cx.listener(move |t, _: &ClickEvent, w, cx| t.open_host(ix, Open::Shell, w, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(kind_icon, 16., accent_text()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .child(h.name.clone()),
                    )
                    .child(div().size(px(7.)).bg(c(dot))),
            )
            .child(
                div().text_color(c(MUTED)).whitespace_nowrap().child(
                    privacy::redact(&format!("{}@{}:{}", h.user, h.host, privacy::port(h.port)))
                        .into_owned(),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        icon_button(("h-shell", ix), "terminal", "Open shell", accent_text())
                            .on_click(cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.open_host(ix, Open::Shell, w, cx)
                            })),
                    )
                    .child(
                        icon_button(("h-docker", ix), "container", "Docker containers", MUTED)
                            .on_click(cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.open_host(ix, Open::Docker, w, cx)
                            })),
                    )
                    .child(
                        icon_button(("h-sftp", ix), "folder", "Files (SFTP)", MUTED).on_click(
                            cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.open_host(ix, Open::Sftp, w, cx)
                            }),
                        ),
                    )
                    .child(div().flex_1())
                    .child(
                        icon_button(
                            ("h-pin", ix),
                            "pin",
                            if h.pinned { "Unpin" } else { "Pin to top" },
                            if h.pinned { accent_text() } else { MUTED },
                        )
                        .on_click(cx.listener(move |t, _, _, cx| {
                            cx.stop_propagation();
                            t.toggle_pin(ix, cx)
                        })),
                    )
                    .child(
                        icon_button(("h-edit", ix), "pencil", "Edit host", MUTED).on_click(
                            cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.open_form(Some(ix), w, cx)
                            }),
                        ),
                    )
                    .child(
                        icon_button(("h-del", ix), "trash", "Delete host", RED).on_click(
                            cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.ask_delete_host(ix, w, cx)
                            }),
                        ),
                    ),
            )
    }

    /// Hosts as big square cards: pinned ones first, then one section per group, then the
    /// ungrouped ones. With a handful of servers this beats a list.
    pub(super) fn home_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let hosts: Vec<(usize, &Host)> = self.cfg.hosts.iter().enumerate().collect();
        let pinned: Vec<usize> = hosts
            .iter()
            .filter(|(_, h)| h.pinned)
            .map(|(i, _)| *i)
            .collect();
        let mut groups: Vec<String> = hosts
            .iter()
            .filter(|(_, h)| !h.pinned)
            .filter_map(|(_, h)| h.group.clone())
            .collect();
        groups.sort_by_key(|g| g.to_lowercase());
        groups.dedup();
        let ungrouped: Vec<usize> = hosts
            .iter()
            .filter(|(_, h)| !h.pinned && h.group.is_none())
            .map(|(i, _)| *i)
            .collect();
        let sectioned = !pinned.is_empty() || !groups.is_empty();

        // Cascade only on launch. Coming back home later just fades the page,
        // so it shows up as fast as any other page.
        let mut n = 0;
        let mut cards = |ixs: &[usize], cx: &mut Context<Self>| -> Vec<AnyElement> {
            ixs.iter()
                .map(|&ix| {
                    let card = self.host_card(ix, &self.cfg.hosts[ix], cx);
                    n += 1;
                    if self.epoch == 0 {
                        stagger_in(card, ("card", ix), n - 1).into_any_element()
                    } else {
                        card.into_any_element()
                    }
                })
                .collect()
        };
        let grid = || div().flex().flex_wrap().gap_2();
        let header = |text: String| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c(MUTED))
                .child(text)
        };

        let mut page = div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_3();

        if !pinned.is_empty() {
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(header("PINNED".into()).child(icon("pin", 11., MUTED)))
                    .child(grid().children(cards(&pinned, cx))),
            );
        }
        for (gi, g) in groups.iter().enumerate() {
            let members: Vec<usize> = hosts
                .iter()
                .filter(|(_, h)| !h.pinned && h.group.as_ref() == Some(g))
                .map(|(i, _)| *i)
                .collect();
            let open = !self.collapsed.contains(g);
            let key = g.clone();
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        header(String::new())
                            .id(("group", gi))
                            .cursor_pointer()
                            .hover(|s| s.text_color(c(TEXT)))
                            .on_click(cx.listener(move |t, _, _, cx| {
                                if !t.collapsed.remove(&key) {
                                    t.collapsed.insert(key.clone());
                                }
                                cx.notify();
                            }))
                            .child(icon(
                                if open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                12.,
                                MUTED,
                            ))
                            .child(icon("folder", 12., MUTED))
                            .child(privacy::redact(&g.to_uppercase()).into_owned())
                            .child(
                                div()
                                    .text_color(c(PLACEHOLDER))
                                    .font_weight(FontWeight::NORMAL)
                                    .child(members.len().to_string()),
                            ),
                    )
                    .when(open, |d| d.child(grid().children(cards(&members, cx)))),
            );
        }

        let small_card = |id: &'static str, ic: &'static str, text: &'static str| {
            div()
                .id(id)
                .w(px(210.))
                .p_2()
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .rounded(px(2.))
                .border_2()
                .border_color(c(BORDER_VARIANT))
                .text_color(c(MUTED))
                .cursor_pointer()
                .hover(|s| s.bg(c(PANEL)).text_color(c(TEXT)).border_color(c(BORDER)))
                .child(icon(ic, 14., MUTED))
                .child(text)
        };
        page.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .when(sectioned && !ungrouped.is_empty(), |d| {
                    d.child(header("HOSTS".into()))
                })
                .child(
                    grid()
                        .children(cards(&ungrouped, cx))
                        .child(
                            small_card("add-host", "plus", "Add host")
                                .on_click(cx.listener(|t, _, w, cx| t.open_form(None, w, cx))),
                        )
                        .child(
                            small_card("quick-connect", "arrow-right", "Quick connect")
                                .on_click(cx.listener(|t, _, w, cx| t.open_quick(w, cx))),
                        )
                        .child(
                            small_card("local-docker", "container", "Local Docker")
                                .on_click(cx.listener(|t, _, w, cx| t.open_local_docker(w, cx))),
                        ),
                ),
        )
        .into_any_element()
    }
}
