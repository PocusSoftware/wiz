//! Quick connect: open a shell on `user@host:port` without adding a host first. The
//! credentials stay in memory for the session unless "Save as host" is ticked.

use std::path::PathBuf;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    Render, SharedString, Window, div, prelude::*, px,
};
use ssh_core::config::{AuthMethod, Host, parse_target};
use ssh_core::secrets::Secret;
use ssh_core::session::Credentials;

use crate::theme::*;
use crate::widgets::input::{Hide, TextInput};
use crate::widgets::{button, checkbox, icon_button, label, primary_button};

pub enum QuickEvent {
    Connect {
        host: Host,
        creds: Credentials,
        save: bool,
    },
    Cancelled,
}

impl EventEmitter<QuickEvent> for QuickConnect {}

/// Tried in order when key auth is picked and no key file is given.
const DEFAULT_KEYS: &[&str] = &["id_ed25519", "id_ecdsa", "id_rsa"];

pub struct QuickConnect {
    target: Entity<TextInput>,
    password: Entity<TextInput>,
    key_path: Entity<TextInput>,
    passphrase: Entity<TextInput>,
    key_mode: bool,
    save: bool,
    error: Option<SharedString>,
    focus: FocusHandle,
}

fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or(PathBuf::from(p)),
        None => PathBuf::from(p),
    }
}

impl QuickConnect {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let target =
            cx.new(|cx| TextInput::new("user@host:port or ssh -p 2222 user@host", false, cx));
        target.update(cx, |t, _| t.set_hide_mode(Hide::Redact));
        Self {
            target,
            password: cx.new(|cx| TextInput::new("", true, cx)),
            key_path: cx
                .new(|cx| TextInput::new("~/.ssh/id_ed25519 (default keys if empty)", false, cx)),
            passphrase: cx.new(|cx| TextInput::new("optional", true, cx)),
            key_mode: false,
            save: false,
            error: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &gpui::App) {
        window.focus(&self.target.focus_handle(cx));
    }

    fn fields(&self) -> Vec<Entity<TextInput>> {
        if self.key_mode {
            vec![
                self.target.clone(),
                self.key_path.clone(),
                self.passphrase.clone(),
            ]
        } else {
            vec![self.target.clone(), self.password.clone()]
        }
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
        match self.build(cx) {
            Ok((host, creds)) => cx.emit(QuickEvent::Connect {
                host,
                creds,
                save: self.save,
            }),
            Err(e) => {
                self.error = Some(e.into());
                cx.notify();
            }
        }
    }

    fn build(&self, cx: &Context<Self>) -> Result<(Host, Credentials), String> {
        let me = crate::platform::local_user().unwrap_or_else(|| "root".into());
        let (user, host, port) = parse_target(self.target.read(cx).value(), &me)?;
        let (auth, creds) = if self.key_mode {
            let typed = self.key_path.read(cx).value().trim().to_string();
            let path = if typed.is_empty() {
                let ssh = dirs::home_dir().map(|h| h.join(".ssh")).unwrap_or_default();
                DEFAULT_KEYS
                    .iter()
                    .map(|k| ssh.join(k))
                    .find(|p| p.is_file())
                    .ok_or("No key in ~/.ssh (id_ed25519, id_ecdsa, id_rsa). Pick one.")?
            } else {
                expand(&typed)
            };
            if !path.is_file() {
                return Err("Private key file not found.".into());
            }
            let pass = self.passphrase.read(cx).value();
            let passphrase = if pass.is_empty() {
                None
            } else {
                Some(Secret::new(pass).map_err(|e| e.to_string())?)
            };
            (
                AuthMethod::Key { path: path.clone() },
                Credentials::Key { path, passphrase },
            )
        } else {
            let pw = self.password.read(cx).value();
            if pw.is_empty() {
                return Err("Enter a password, or switch to SSH key.".into());
            }
            let pw = Secret::new(pw).map_err(|e| e.to_string())?;
            (AuthMethod::Password, Credentials::Password(pw))
        };
        let host = Host {
            name: format!("{user}@{host}"),
            host,
            port,
            user,
            group: None,
            pinned: false,
            auth,
        };
        host.validate()?;
        Ok((host, creds))
    }
}

impl Focusable for QuickConnect {
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

impl Render for QuickConnect {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key_mode = self.key_mode;
        let tab = |id: &'static str, text: &str, active: bool, mode: bool| {
            button(id, text)
                .when(active, |b| b.bg(c(accent())).text_color(c(0xffffff)))
                .on_click(cx.listener(move |t, _: &ClickEvent, _, cx| {
                    t.key_mode = mode;
                    cx.notify();
                }))
        };
        let store = ssh_core::secrets::store().label();
        div()
            .id("quick-connect")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "enter" => t.submit(cx),
                    "escape" => cx.emit(QuickEvent::Cancelled),
                    "tab" => t.cycle_focus(ev.keystroke.modifiers.shift, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .w(px(380.))
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
                    .child(div().text_sm().child("Quick connect"))
                    .child(
                        icon_button("qc-close", "x", "Close (Esc)", MUTED)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(QuickEvent::Cancelled))),
                    ),
            )
            .child(field("Target", &self.target))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(tab("qc-pw", "Password", !key_mode, false))
                    .child(tab("qc-key", "SSH key", key_mode, true)),
            )
            .when(!key_mode, |d| d.child(field("Password", &self.password)))
            .when(key_mode, |d| {
                d.child(field("Private key", &self.key_path))
                    .child(field("Key passphrase", &self.passphrase))
            })
            .child(
                checkbox(
                    "qc-save",
                    self.save,
                    &format!("Save as host (secret goes to the {store})"),
                )
                .on_click(cx.listener(|t, _, _, cx| {
                    t.save = !t.save;
                    cx.notify();
                })),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(div().text_xs().text_color(c(RED)).child(e))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("qc-cancel", "Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(QuickEvent::Cancelled))),
                    )
                    .child(
                        primary_button("qc-go", "Connect")
                            .on_click(cx.listener(|t, _, _, cx| t.submit(cx))),
                    ),
            )
    }
}

// The tests point HOME at a temp folder; Windows finds the profile folder without HOME.
#[cfg(all(test, unix))]
mod tests;
