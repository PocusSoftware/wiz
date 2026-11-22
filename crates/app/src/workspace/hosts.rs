//! Adding, editing, pinning and deleting hosts.

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
            move |this, window, cx| {
                let start = existing
                    .and_then(|i| this.cfg.hosts.get(i).cloned().map(|h| Start::Edit(i, h)))
                    .unwrap_or(Start::New);
                this.show_form(start, window, cx)
            },
            window,
            cx,
        );
    }

    pub(super) fn duplicate_host(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_secrets(
            move |this, window, cx| {
                if let Some(h) = this.cfg.hosts.get(ix).cloned() {
                    this.show_form(Start::Copy(h), window, cx)
                }
            },
            window,
            cx,
        );
    }

    fn show_form(&mut self, start: Start, window: &mut Window, cx: &mut Context<Self>) {
        let existing = match start {
            Start::Edit(i, _) => Some(i),
            Start::New | Start::Copy(_) => None,
        };
        let others = self
            .cfg
            .hosts
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != existing)
            .map(|(_, h)| h.clone())
            .collect();
        let groups = self
            .cfg
            .hosts
            .iter()
            .filter_map(|h| h.group.clone())
            .collect();
        let form = cx.new(|cx| HostForm::new(start, others, groups, cx));
        cx.subscribe_in(&form, window, |this, _, ev: &FormEvent, window, cx| {
            match ev {
                FormEvent::Saved { host, index } => {
                    match index {
                        Some(i) if *i < this.cfg.hosts.len() => {
                            // Hosts that go through this one follow it to its new address.
                            let old = this.cfg.hosts[*i].account();
                            this.cfg.retarget_jumps(&old, &host.account());
                            this.cfg.hosts[*i] = (**host).clone();
                        }
                        _ => this.cfg.hosts.push((**host).clone()),
                    }
                    this.save_config(cx);
                }
                FormEvent::Cancelled => {}
            }
            this.form = None;
            window.focus(&this.focus, cx);
            cx.notify();
        })
        .detach();
        form.update(cx, |d, cx| d.focus_first(window, cx));
        self.form = Some(form);
        cx.notify();
    }

    pub(super) fn ask_delete_host(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_delete = Some(ix);
        // Take keyboard focus so Enter and Esc reach the dialog.
        window.focus(&self.focus, cx);
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
        // Hosts that went through it now connect directly; say so rather than fail later.
        let mut direct = Vec::new();
        for other in &mut self.cfg.hosts {
            if other.jump.as_ref() == Some(&acct) {
                other.jump = None;
                direct.push(other.name.clone());
            }
        }
        if !direct.is_empty() {
            let verb = if direct.len() == 1 {
                "connects"
            } else {
                "connect"
            };
            self.set_status(
                format!(
                    "{} now {verb} directly, without {}.",
                    direct.join(", "),
                    h.name
                ),
                false,
            );
        }
        self.save_config(cx);
        cx.notify();
    }

    /// Look up the operating system of saved host `account` on its new connection, for the
    /// logo on the hosts page. Silent when it can't be told, and saves only a change.
    pub(super) fn detect_os(&self, account: String, conn: Arc<Connection>, cx: &mut Context<Self>) {
        if !crate::prefs::get().show_os_logos()
            || !self.cfg.hosts.iter().any(|h| h.account() == account)
        {
            return;
        }
        cx.spawn(async move |this, cx| {
            let Some(os) = runtime::spawn(async move { ssh_core::os::detect(&conn).await }).await
            else {
                return;
            };
            this.update(cx, |t, cx| {
                // Looked up again: the host may have been edited or deleted meanwhile.
                let Some(h) = t.cfg.hosts.iter_mut().find(|h| h.account() == account) else {
                    return;
                };
                if h.os.as_deref() != Some(os) {
                    h.os = Some(os.into());
                    t.save_config(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn toggle_pin(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(h) = self.cfg.hosts.get_mut(ix) {
            h.pinned = !h.pinned;
            self.save_config(cx);
            cx.notify();
        }
    }
}

/// The icon of a host on the hosts page. A picked logo always shows, a detected one only when
/// `detected` (the setting) is on. Only logos from the table: hosts.toml may have been edited
/// by hand.
pub(super) fn card_icon(h: &Host, detected: bool) -> String {
    match logo(h, detected) {
        Some(l) => format!("os/{}", l.slug),
        None => match h.auth {
            AuthMethod::Password => "server",
            AuthMethod::Key { .. } | AuthMethod::Agent => "key",
        }
        .into(),
    }
}

/// The logo shown for a host: the picked one, else the detected one when `detected` is on.
pub(super) fn logo(h: &Host, detected: bool) -> Option<&'static ssh_core::os::Logo> {
    h.logo
        .as_deref()
        .or(h.os.as_deref().filter(|_| detected))
        .and_then(ssh_core::os::logo)
}
