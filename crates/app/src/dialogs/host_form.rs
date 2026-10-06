//! Add/edit host dialog. Secrets go to the secret store (Keychain or the encrypted file),
//! never to the config file.

use std::path::PathBuf;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    PathPromptOptions, Render, SharedString, Window, div, prelude::*, px,
};
use ssh_core::config::{AuthMethod, Host};
use ssh_core::secrets::{self, Kind};

use crate::theme::*;
use crate::widgets::input::{Hide, TextInput};
use crate::widgets::{button, icon_button, label, primary_button};

pub enum FormEvent {
    Saved { host: Host, index: Option<usize> },
    Cancelled,
}

impl EventEmitter<FormEvent> for HostForm {}

pub struct HostForm {
    index: Option<usize>,
    old: Option<Host>,
    /// Accounts (user@host:port) of the other saved hosts, to refuse duplicates.
    taken: Vec<String>,
    /// Existing group names, so "prod" joins an existing "Prod" instead of making a twin.
    groups: Vec<String>,
    name: Entity<TextInput>,
    group: Entity<TextInput>,
    host: Entity<TextInput>,
    port: Entity<TextInput>,
    user: Entity<TextInput>,
    password: Entity<TextInput>,
    key_path: Entity<TextInput>,
    passphrase: Entity<TextInput>,
    key_mode: bool,
    error: Option<SharedString>,
    focus: FocusHandle,
}

impl HostForm {
    pub fn new(
        existing: Option<(usize, Host)>,
        taken: Vec<String>,
        groups: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| TextInput::new("My server", false, cx));
        let group = cx.new(|cx| TextInput::new("Optional, e.g. Production", false, cx));
        let host = cx.new(|cx| TextInput::new("example.com or 10.0.0.5", false, cx));
        let port = cx.new(|cx| TextInput::new("22", false, cx));
        let user = cx.new(|cx| TextInput::new("root", false, cx));
        // Host, user and port are bullets unless the field is being edited.
        host.update(cx, |t, _| t.set_hide_mode(Hide::Full));
        user.update(cx, |t, _| t.set_hide_mode(Hide::Full));
        port.update(cx, |t, _| t.set_hide_mode(Hide::Port));
        let editing = existing.is_some();
        let secret_hint = if editing { "(unchanged)" } else { "" };
        let password = cx.new(|cx| TextInput::new(secret_hint, true, cx));
        let key_path = cx.new(|cx| TextInput::new("~/.ssh/id_ed25519", false, cx));
        let passphrase =
            cx.new(|cx| TextInput::new(if editing { "(unchanged)" } else { "optional" }, true, cx));
        let mut key_mode = false;
        let (index, old) = match existing {
            Some((i, h)) => {
                name.update(cx, |t, _| t.set_value(&h.name));
                group.update(cx, |t, _| {
                    t.set_value(h.group.as_deref().unwrap_or_default())
                });
                host.update(cx, |t, _| t.set_value(&h.host));
                port.update(cx, |t, _| t.set_value(&h.port.to_string()));
                user.update(cx, |t, _| t.set_value(&h.user));
                if let AuthMethod::Key { path } = &h.auth {
                    key_mode = true;
                    key_path.update(cx, |t, _| t.set_value(&path.to_string_lossy()));
                }
                (Some(i), Some(h))
            }
            None => (None, None),
        };
        Self {
            index,
            old,
            taken,
            groups,
            name,
            group,
            host,
            port,
            user,
            password,
            key_path,
            passphrase,
            key_mode,
            error: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &gpui::App) {
        window.focus(&self.name.focus_handle(cx));
    }

    fn fields(&self) -> Vec<Entity<TextInput>> {
        let mut v = vec![
            self.name.clone(),
            self.group.clone(),
            self.host.clone(),
            self.port.clone(),
            self.user.clone(),
        ];
        if self.key_mode {
            v.push(self.key_path.clone());
            v.push(self.passphrase.clone());
        } else {
            v.push(self.password.clone());
        }
        v
    }

    fn cycle_focus(&self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let f = self.fields();
        let cur = f.iter().position(|e| e.focus_handle(cx).is_focused(window));
        let next = match (cur, back) {
            (None, _) => 0,
            (Some(i), false) => (i + 1) % f.len(),
            (Some(i), true) => (i + f.len() - 1) % f.len(),
        };
        window.focus(&f[next].focus_handle(cx));
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        match self.try_save(cx) {
            Ok(host) => cx.emit(FormEvent::Saved {
                host,
                index: self.index,
            }),
            Err(e) => {
                self.error = Some(e.into());
                cx.notify();
            }
        }
    }

    fn try_save(&mut self, cx: &mut Context<Self>) -> Result<Host, String> {
        let get = |e: &Entity<TextInput>| e.read(cx).value().trim().to_string();
        let host = get(&self.host);
        // Accept a pasted bracketed IPv6 address ([::1]).
        let host = match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            Some(inner) => inner.to_string(),
            None => host,
        };
        let user = get(&self.user);
        let port_s = get(&self.port);
        let port: u16 = if port_s.is_empty() {
            22
        } else {
            port_s.parse().map_err(|_| "Port must be 1-65535.")?
        };
        let group = Some(get(&self.group)).filter(|g| !g.is_empty()).map(|g| {
            self.groups
                .iter()
                .find(|e| e.to_lowercase() == g.to_lowercase())
                .cloned()
                .unwrap_or(g)
        });
        let name = get(&self.name);
        let auth = if self.key_mode {
            let p = get(&self.key_path);
            if p.is_empty() {
                return Err("Choose a private key file.".into());
            }
            let p = match p.strip_prefix("~/") {
                Some(rest) => dirs::home_dir()
                    .map(|h| h.join(rest))
                    .unwrap_or(PathBuf::from(&p)),
                None => PathBuf::from(&p),
            };
            if !p.is_file() {
                return Err("Private key file not found.".into());
            }
            AuthMethod::Key { path: p }
        } else {
            AuthMethod::Password
        };
        let new = Host {
            name,
            host,
            port,
            user,
            group,
            pinned: self.old.as_ref().is_some_and(|o| o.pinned),
            auth,
        };
        new.validate()?;
        // Filled in after validating: the length limit is for names the user typed.
        let mut new = new;
        if new.name.is_empty() {
            new.name = format!("{}@{}", new.user, new.host);
        }
        let acct = new.account();
        if self.taken.contains(&acct) {
            return Err("Another saved host already uses this user, host and port.".into());
        }
        // Every check above runs before the secret store is touched.
        let (pw, pass) = (
            self.password.read(cx).value().to_string(),
            self.passphrase.read(cx).value().to_string(),
        );

        // Keep secrets attached to the right account if user/host/port changed.
        if let Some(old) = &self.old {
            let oa = old.account();
            if oa != acct {
                for kind in [Kind::Password, Kind::KeyPassphrase] {
                    if let Some(s) = secrets::get(kind, &oa).map_err(|e| e.to_string())? {
                        secrets::set(kind, &acct, &s).map_err(|e| e.to_string())?;
                    }
                    secrets::delete(kind, &oa).map_err(|e| e.to_string())?;
                }
            }
        }
        if self.key_mode {
            secrets::delete(Kind::Password, &acct).map_err(|e| e.to_string())?;
            if !pass.is_empty() {
                secrets::set(Kind::KeyPassphrase, &acct, &pass).map_err(|e| e.to_string())?;
            }
        } else {
            secrets::delete(Kind::KeyPassphrase, &acct).map_err(|e| e.to_string())?;
            if !pw.is_empty() {
                secrets::set(Kind::Password, &acct, &pw).map_err(|e| e.to_string())?;
            } else if secrets::get(Kind::Password, &acct)
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err(format!(
                    "Enter a password (it is stored in the {}).",
                    secrets::store().label()
                ));
            }
        }
        Ok(new)
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose private key".into()),
        });
        let input = self.key_path.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.first()
            {
                let s = p.to_string_lossy().to_string();
                input
                    .update(cx, |t, cx| {
                        t.set_value(&s);
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
    }
}

impl Focusable for HostForm {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

fn field(name: &str, input: &Entity<TextInput>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(label(name))
        .child(input.clone())
}

impl Render for HostForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key_mode = self.key_mode;
        let tab = |id: &'static str, text: &str, active: bool, mode: bool| {
            button(id, text)
                .when(active, |b| b.bg(c(ACCENT)).text_color(c(0xffffff)))
                .on_click(cx.listener(move |t, _: &ClickEvent, _, cx| {
                    t.key_mode = mode;
                    cx.notify();
                }))
        };
        div()
            .id("host-form")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "enter" => t.submit(cx),
                    "escape" => cx.emit(FormEvent::Cancelled),
                    "tab" => t.cycle_focus(ev.keystroke.modifiers.shift, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .w(px(340.))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(3.))
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().child(if self.index.is_some() {
                        "Edit host"
                    } else {
                        "Add host"
                    }))
                    .child(
                        icon_button("form-close", "x", "Close (Esc)", MUTED)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(FormEvent::Cancelled))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(field("Name", &self.name).flex_1())
                    .child(field("Group", &self.group).flex_1()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(field("Host", &self.host).flex_1())
                    .child(field("Port", &self.port).w(px(80.))),
            )
            .child(field("User", &self.user))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(tab("auth-pw", "Password", !key_mode, false))
                    .child(tab("auth-key", "SSH key", key_mode, true)),
            )
            .when(!key_mode, |d| {
                d.child(field(
                    &format!("Password (stored in the {})", secrets::store().label()),
                    &self.password,
                ))
            })
            .when(key_mode, |d| {
                d.child(
                    div()
                        .flex()
                        .gap_2()
                        .items_end()
                        .child(field("Private key", &self.key_path).flex_1())
                        .child(
                            button("browse", "Browse")
                                .on_click(cx.listener(|t, _, _, cx| t.browse(cx))),
                        ),
                )
                .child(field(
                    &format!(
                        "Key passphrase (stored in the {})",
                        secrets::store().label()
                    ),
                    &self.passphrase,
                ))
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(div().text_xs().text_color(c(RED)).child(e))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("cancel", "Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(FormEvent::Cancelled))),
                    )
                    .child(
                        primary_button("save", "Save")
                            .on_click(cx.listener(|t, _, _, cx| t.submit(cx))),
                    ),
            )
    }
}
