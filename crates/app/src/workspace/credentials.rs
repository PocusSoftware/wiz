//! The secret store from the UI (master password, switching stores) and quick connect.

use super::*;

impl Workspace {
    /// Run `then` now if secrets can be read, otherwise after the master password is entered.
    pub(super) fn with_secrets(
        &mut self,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if secrets::ready() {
            then(self, window, cx);
            return;
        }
        self.after_unlock = Some(Box::new(then));
        self.show_unlock(UnlockMode::for_file(), window, cx);
    }

    pub(super) fn show_unlock(
        &mut self,
        mode: UnlockMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| UnlockDialog::new(mode, cx));
        cx.subscribe_in(
            &dialog,
            window,
            move |this, _, ev: &UnlockEvent, window, cx| {
                this.unlock = None;
                let then = this.after_unlock.take();
                window.focus(&this.focus);
                if let UnlockEvent::Done = ev {
                    if mode == UnlockMode::Change {
                        this.set_status("Master password changed.", false);
                    }
                    if let Some(f) = then {
                        f(this, window, cx);
                    }
                }
                this.settings_changed(cx);
            },
        )
        .detach();
        dialog.read(cx).focus_first(window, cx);
        self.unlock = Some(dialog);
        cx.notify();
    }

    /// Move every saved host's secrets to the other store, then switch the setting. If the
    /// move fails part way, what moved is moved back and the setting stays.
    pub(super) fn switch_store(&mut self, to: Store, window: &mut Window, cx: &mut Context<Self>) {
        let from = secrets::store();
        if from == to {
            return;
        }
        let go = move |this: &mut Self, _: &mut Window, cx: &mut Context<Self>| {
            let accounts: Vec<String> = this.cfg.hosts.iter().map(Host::account).collect();
            let r = if from == Store::File && !secrets::file_exists() {
                Ok(0)
            } else {
                secrets::migrate(&accounts, from, to)
            };
            match r {
                Ok(n) => match crate::prefs::update(|s| s.secret_store = Some(to.setting().into()))
                {
                    Ok(()) => this.set_status(
                        format!(
                            "Passwords are now kept in the {}. Moved {n} secret{}.",
                            to.label(),
                            if n == 1 { "" } else { "s" }
                        ),
                        false,
                    ),
                    Err(e) => this.set_status(format!("Could not save settings: {e}"), true),
                },
                Err(e) => {
                    let _ = secrets::migrate(&accounts, to, from);
                    this.set_status(
                        format!("Could not move secrets to the {}: {e}", to.label()),
                        true,
                    );
                }
            }
            this.settings_changed(cx);
        };
        let needs_file = to == Store::File || secrets::file_exists();
        if needs_file && !secrets::is_unlocked() {
            self.after_unlock = Some(Box::new(go));
            self.show_unlock(UnlockMode::for_file(), window, cx);
        } else {
            go(self, window, cx);
        }
    }

    pub(super) fn open_quick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(QuickDialog::new);
        cx.subscribe_in(&dialog, window, |this, _, ev: &QuickEvent, window, cx| {
            this.quick_form = None;
            window.focus(&this.focus);
            if let QuickEvent::Connect { host, creds, save } = ev {
                this.quick_connect(host.clone(), creds.clone(), *save, window, cx);
            }
            cx.notify();
        })
        .detach();
        dialog.read(cx).focus_first(window, cx);
        self.quick_form = Some(dialog);
        cx.notify();
    }

    pub(super) fn quick_connect(
        &mut self,
        host: Host,
        creds: Credentials,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let account = host.account();
        let saved = self.cfg.hosts.iter().any(|h| h.account() == account);
        if save && !saved {
            self.with_secrets(
                move |this, window, cx| {
                    let r = match &creds {
                        Credentials::Password(p) => secrets::set(Kind::Password, &account, p),
                        Credentials::Key {
                            passphrase: Some(p),
                            ..
                        } => secrets::set(Kind::KeyPassphrase, &account, p),
                        Credentials::Key { .. } => Ok(()),
                    };
                    match r {
                        Ok(()) => {
                            this.cfg.hosts.push(host.clone());
                            this.save_config();
                        }
                        Err(e) => {
                            // Still connect, as an unsaved quick host.
                            this.set_status(
                                format!("Not saved, the secret could not be stored: {e}"),
                                true,
                            );
                            this.quick.insert(account, (host.clone(), creds.clone()));
                            this.refresh_tokens();
                        }
                    }
                    this.connect(host, Some(creds), Open::Shell, window, cx);
                },
                window,
                cx,
            );
            return;
        }
        if !saved {
            self.quick.insert(account, (host.clone(), creds.clone()));
            self.refresh_tokens();
        } else if save {
            self.set_status("A saved host already uses this user, host and port.", false);
        }
        self.connect(host, Some(creds), Open::Shell, window, cx);
    }
}
