//! Quick connect: open a shell on `user@host:port` without adding a host first. The
//! credentials stay in memory for the session unless "Save as host" is ticked.

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    Render, SharedString, Window, div, prelude::*,
};
use ssh_core::config::{AuthMethod, Host, parse_target};
use ssh_core::secrets::Secret;
use ssh_core::session::Credentials;

use crate::theme::*;
use crate::widgets::input::{Hide, TextInput};
use crate::widgets::{
    button, checkbox, close_button, dialog, dialog_header, label, primary_button,
};

pub enum QuickEvent {
    Connect {
        host: Box<Host>,
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

    pub fn focus_first(&self, window: &mut Window, cx: &mut gpui::App) {
        window.focus(&self.target.focus_handle(cx), cx);
    }

    /// Start with `target` typed in, as searched for on the hosts page.
    pub fn set_target(&mut self, target: &str, cx: &mut Context<Self>) {
        self.target.update(cx, |t, cx| {
            t.set_value(target);
            cx.notify();
        });
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        match self.build(cx) {
            Ok((host, creds)) => cx.emit(QuickEvent::Connect {
                host: Box::new(host),
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
                let ssh = std::env::home_dir()
                    .map(|h| h.join(".ssh"))
                    .unwrap_or_default();
                DEFAULT_KEYS
                    .iter()
                    .map(|k| ssh.join(k))
                    .find(|p| p.is_file())
                    .ok_or("No key in ~/.ssh (id_ed25519, id_ecdsa, id_rsa). Pick one.")?
            } else {
                ssh_core::config::expand_home(&typed)
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
            jump: None,
            os: None,
            logo: None,
            tags: Vec::new(),
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
                .tab_index(1)
                .when(active, |b| b.bg(c(accent())).text_color(c(on_accent())))
                .on_click(cx.listener(move |t, _: &ClickEvent, _, cx| {
                    t.key_mode = mode;
                    cx.notify();
                }))
        };
        let store = ssh_core::secrets::store().label();
        div()
            .id("quick-connect")
            .track_focus(&self.focus)
            // Tab visits the fields first, then the buttons (tab index 1).
            .tab_group()
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, _, cx| {
                match ev.keystroke.key.as_str() {
                    "enter" => t.submit(cx),
                    "escape" => cx.emit(QuickEvent::Cancelled),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .map(|d| dialog(d, 380.))
            .child(dialog_header(
                "Quick connect",
                close_button("qc-close", "Close (Esc)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(QuickEvent::Cancelled))),
            ))
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
                .tab_index(1)
                .on_click(cx.listener(|t, _, _, cx| {
                    t.save = !t.save;
                    cx.notify();
                })),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(div().text_xs().text_color(c(palette().red)).child(e))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("qc-cancel", "Cancel")
                            .tab_index(1)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(QuickEvent::Cancelled))),
                    )
                    .child(
                        primary_button("qc-go", "Connect")
                            .tab_index(1)
                            .on_click(cx.listener(|t, _, _, cx| t.submit(cx))),
                    ),
            )
    }
}

// The tests point HOME at a temp folder; Windows finds the profile folder without HOME.

#[cfg(all(test, unix))]
mod tests {
    //! Quick connect dialog: building the host and credentials from what was typed.

    use gpui::TestAppContext;

    use super::*;

    fn fill(d: &mut QuickConnect, target: &str, key_path: &str, cx: &mut Context<QuickConnect>) {
        d.key_mode = true;
        d.target.update(cx, |t, _| t.set_value(target));
        d.key_path.update(cx, |t, _| t.set_value(key_path));
    }

    #[gpui::test]
    fn key_auth_finds_default_key_or_reports_missing(cx: &mut TestAppContext) {
        let _g = crate::testing::GLOBALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        // SAFETY: GLOBALS is held, no other test reads the environment meanwhile.
        unsafe { std::env::set_var("HOME", home.path()) };
        let dialog = cx.new(QuickConnect::new);

        // No keys in ~/.ssh.
        let err = dialog.update(cx, |d, cx| {
            fill(d, "u@h:2200", "", cx);
            d.build(cx).err()
        });
        assert!(err.unwrap().contains("No key"));

        // id_ecdsa is the first default that exists.
        let ssh = home.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(ssh.join("id_ecdsa"), "k").unwrap();
        let (host, creds) = dialog
            .update(cx, |d, cx| {
                d.passphrase.update(cx, |t, _| t.set_value("pp"));
                d.build(cx)
            })
            .unwrap();
        assert_eq!((host.user.as_str(), host.port), ("u", 2200));
        assert_eq!(
            host.auth,
            AuthMethod::Key {
                path: ssh.join("id_ecdsa")
            }
        );
        assert!(
            matches!(creds, Credentials::Key { passphrase: Some(p), .. } if *p.reveal() == "pp")
        );

        // A typed path, with ~ expanded, must exist.
        let err = dialog.update(cx, |d, cx| {
            fill(d, "u@h", "~/.ssh/nope", cx);
            d.build(cx).err()
        });
        assert_eq!(err.as_deref(), Some("Private key file not found."));
        let ok = dialog.update(cx, |d, cx| {
            fill(d, "u@h", "~/.ssh/id_ecdsa", cx);
            d.build(cx).is_ok()
        });
        assert!(ok);

        // Password mode needs a password; invalid targets are refused.
        let err = dialog.update(cx, |d, cx| {
            d.key_mode = false;
            d.build(cx).err()
        });
        assert!(err.unwrap().contains("Enter a password"));
        let err = dialog.update(cx, |d, cx| {
            d.password.update(cx, |t, _| t.set_value("pw"));
            d.target.update(cx, |t, _| t.set_value("u@bad host"));
            d.build(cx).err()
        });
        assert!(err.is_some());

        // SAFETY: as above.
        unsafe {
            match old_home {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}
