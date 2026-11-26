//! Add/edit host dialog. Secrets go to the secret store (Keychain or the encrypted file),
//! never to the config file.

mod folder;
mod logo;
#[cfg(test)]
mod tests;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    PathPromptOptions, Render, SharedString, Window, div, prelude::*, px,
};
use ssh_core::config::{AuthMethod, Config, Host};
use ssh_core::secrets::{self, Kind};

use crate::theme::*;
use crate::widgets::input::{Hide, InputEvent, TextInput};
use crate::widgets::{button, close_button, dialog, dialog_header, label, primary_button};

/// How the host logs in, as picked in the form.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Password,
    Key,
    Agent,
}

pub enum Start {
    New,
    Edit(usize, Host),
    Copy(Host),
}

pub enum FormEvent {
    Saved {
        host: Box<Host>,
        index: Option<usize>,
    },
    Cancelled,
}

impl EventEmitter<FormEvent> for HostForm {}

pub struct HostForm {
    index: Option<usize>,
    old: Option<Host>,
    copy_of: Option<String>,
    /// The other saved hosts, to refuse duplicates and to pick a jump host from.
    others: Vec<Host>,
    /// Existing folder names, so "prod" joins an existing "Prod" instead of making a twin.
    groups: Vec<String>,
    folders_open: bool,
    name: Entity<TextInput>,
    group: Entity<TextInput>,
    tags: Entity<TextInput>,
    host: Entity<TextInput>,
    port: Entity<TextInput>,
    user: Entity<TextInput>,
    /// Name of the saved host to connect through.
    jump: Entity<TextInput>,
    password: Entity<TextInput>,
    key_path: Entity<TextInput>,
    passphrase: Entity<TextInput>,
    mode: Mode,
    /// Logo picked by hand, `None` for the one found when connecting.
    logo: Option<&'static str>,
    logo_open: bool,
    /// Narrows the open logo list.
    logo_search: Entity<TextInput>,
    error: Option<SharedString>,
    focus: FocusHandle,
}

impl HostForm {
    pub fn new(
        start: Start,
        others: Vec<Host>,
        groups: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| TextInput::new("My server", false, cx));
        let group = cx.new(|cx| TextInput::new("Optional, e.g. Production", false, cx));
        let tags = cx.new(|cx| TextInput::new("Optional, comma separated", false, cx));
        let host = cx.new(|cx| TextInput::new("example.com or 10.0.0.5", false, cx));
        let port = cx.new(|cx| TextInput::new("22", false, cx));
        let user = cx.new(|cx| TextInput::new("root", false, cx));
        let jump = cx.new(|cx| TextInput::new("Optional, name of a saved host", false, cx));
        // Host, user and port are bullets unless the field is being edited.
        name.update(cx, |t, _| t.set_hide_mode(Hide::Redact));
        group.update(cx, |t, _| t.set_hide_mode(Hide::Redact));
        host.update(cx, |t, _| t.set_hide_mode(Hide::Full));
        user.update(cx, |t, _| t.set_hide_mode(Hide::Full));
        port.update(cx, |t, _| t.set_hide_mode(Hide::Port));
        let (secret_hint, passphrase_hint) = match start {
            Start::New => ("", "optional"),
            Start::Edit(..) => ("(unchanged)", "(unchanged)"),
            Start::Copy(_) => ("(copied)", "(copied)"),
        };
        let password = cx.new(|cx| TextInput::new(secret_hint, true, cx));
        let key_path = cx.new(|cx| TextInput::new("~/.ssh/id_ed25519", false, cx));
        let passphrase = cx.new(|cx| TextInput::new(passphrase_hint, true, cx));
        let logo_search = cx.new(|cx| TextInput::new("Search, like ubuntu or rhel", false, cx));
        cx.subscribe(&logo_search, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        let mut mode = Mode::Password;
        let (index, old, copy_of, shown) = match start {
            Start::New => (None, None, None, None),
            Start::Edit(i, h) => (Some(i), Some(h.clone()), None, Some(h)),
            Start::Copy(h) => (None, None, Some(h.account()), Some(h)),
        };
        let logo = shown
            .as_ref()
            .and_then(|h| h.logo.as_deref())
            .and_then(ssh_core::os::logo)
            .map(|l| l.slug);
        if let Some(h) = shown {
            let shown_name = if copy_of.is_some() {
                format!("{} copy", h.name)
            } else {
                h.name.clone()
            };
            name.update(cx, |t, _| t.set_value(&shown_name));
            group.update(cx, |t, _| {
                t.set_value(h.group.as_deref().unwrap_or_default())
            });
            tags.update(cx, |t, _| t.set_value(&h.tags.join(", ")));
            host.update(cx, |t, _| t.set_value(&h.host));
            port.update(cx, |t, _| t.set_value(&h.port.to_string()));
            user.update(cx, |t, _| t.set_value(&h.user));
            if let Some(j) = others.iter().find(|o| Some(o.account()) == h.jump) {
                jump.update(cx, |t, _| t.set_value(&j.name));
            }
            match &h.auth {
                AuthMethod::Password => {}
                AuthMethod::Key { path } => {
                    mode = Mode::Key;
                    key_path.update(cx, |t, _| t.set_value(&path.to_string_lossy()));
                }
                AuthMethod::Agent => mode = Mode::Agent,
            }
        }
        Self {
            index,
            old,
            copy_of,
            others,
            groups: folder::folder_names(groups),
            folders_open: false,
            name,
            group,
            tags,
            host,
            port,
            user,
            jump,
            password,
            key_path,
            passphrase,
            mode,
            logo,
            logo_open: false,
            logo_search,
            error: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut gpui::App) {
        window.focus(&self.name.focus_handle(cx), cx);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        match self.try_save(cx) {
            Ok(host) => cx.emit(FormEvent::Saved {
                host: Box::new(host),
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
        // Like the placeholder says: an empty user is root.
        let user = Some(get(&self.user))
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "root".into());
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
        let mut tags: Vec<String> = Vec::new();
        for t in get(&self.tags)
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            if !tags.iter().any(|e| e.to_lowercase() == t.to_lowercase()) {
                tags.push(t.to_string());
            }
        }
        let name = get(&self.name);
        let jump_name = get(&self.jump);
        let jump = if jump_name.is_empty() {
            None
        } else {
            let mut named = self
                .others
                .iter()
                .filter(|o| o.name.to_lowercase() == jump_name.to_lowercase());
            match (named.next(), named.next()) {
                (Some(j), None) => Some(j.account()),
                (None, _) => return Err("No saved host has that jump host name.".into()),
                (Some(_), Some(_)) => {
                    return Err("Several saved hosts have that name; rename one first.".into());
                }
            }
        };
        let auth = match self.mode {
            Mode::Password => AuthMethod::Password,
            Mode::Key => {
                let p = get(&self.key_path);
                if p.is_empty() {
                    return Err("Choose a private key file.".into());
                }
                let p = ssh_core::config::expand_home(&p);
                if !p.is_file() {
                    return Err("Private key file not found.".into());
                }
                AuthMethod::Key { path: p }
            }
            Mode::Agent => AuthMethod::Agent,
        };
        // A new address may be another server: its logo is found again on connect.
        let os = self
            .old
            .as_ref()
            .filter(|o| o.host == host)
            .and_then(|o| o.os.clone());
        let new = Host {
            name,
            host,
            port,
            user,
            group,
            pinned: self.old.as_ref().is_some_and(|o| o.pinned),
            jump,
            os,
            logo: self.logo.map(Into::into),
            tags,
            auth,
        };
        new.validate()?;
        // Filled in after validating: the length limit is for names the user typed.
        let mut new = new;
        if new.name.is_empty() {
            new.name = format!("{}@{}", new.user, new.host);
        }
        let acct = new.account();
        if self.others.iter().any(|o| o.account() == acct) {
            return Err("Another saved host already uses this user, host and port.".into());
        }
        // A jump host that (through others) goes through this one would never connect.
        let all = Config {
            hosts: self.others.iter().cloned().chain([new.clone()]).collect(),
        };
        all.jump_chain(&new)?;
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
                        secrets::set(kind, &acct, &s.reveal()).map_err(|e| e.to_string())?;
                    }
                    secrets::delete(kind, &oa).map_err(|e| e.to_string())?;
                }
            }
        }
        if let Some(src) = &self.copy_of {
            for kind in [Kind::Password, Kind::KeyPassphrase] {
                if let Some(s) = secrets::get(kind, src).map_err(|e| e.to_string())? {
                    secrets::set(kind, &acct, &s.reveal()).map_err(|e| e.to_string())?;
                }
            }
        }
        match self.mode {
            Mode::Key => {
                secrets::delete(Kind::Password, &acct).map_err(|e| e.to_string())?;
                if !pass.is_empty() {
                    secrets::set(Kind::KeyPassphrase, &acct, &pass).map_err(|e| e.to_string())?;
                }
            }
            // The agent holds the keys: nothing of this host stays in the secret store.
            Mode::Agent => {
                for kind in [Kind::Password, Kind::KeyPassphrase] {
                    secrets::delete(kind, &acct).map_err(|e| e.to_string())?;
                }
            }
            Mode::Password => {
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
                input.update(cx, |t, cx| {
                    t.set_value(&s);
                    cx.notify();
                });
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
        let mode = self.mode;
        let tab = |id: &'static str, text: &str, to: Mode| {
            button(id, text)
                .tab_index(1)
                .when(mode == to, |b| b.bg(c(accent())).text_color(c(on_accent())))
                .on_click(cx.listener(move |t, _: &ClickEvent, _, cx| {
                    t.mode = to;
                    cx.notify();
                }))
        };
        div()
            .id("host-form")
            .track_focus(&self.focus)
            // Tab visits the fields first, then the buttons (tab index 1).
            .tab_group()
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, w, cx| {
                match ev.keystroke.key.as_str() {
                    // In the logo search, Enter picks the first match.
                    "enter" if t.logo_open => {
                        if let Some(pick) = t.first_logo(cx) {
                            t.close_logos(Some(pick), w, cx);
                        }
                    }
                    "enter" => t.submit(cx),
                    "escape" if t.logo_open => t.close_logos(None, w, cx),
                    "escape" if t.folders_open => t.close_folders(None, cx),
                    "escape" => cx.emit(FormEvent::Cancelled),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .map(|d| dialog(d, 340.))
            .child(dialog_header(if self.index.is_some() {
                        "Edit host"
                    } else {
                        "Add host"
                    }, close_button("form-close", "Close (Esc)").on_click(cx.listener(|_, _, _, cx| cx.emit(FormEvent::Cancelled)))))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(field("Name", &self.name).flex_1())
                    .child(self.folder_field(cx).flex_1()),
            )
            .child(field("Tags", &self.tags))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(field("Host", &self.host).flex_1())
                    .child(field("Port", &self.port).w(px(80.))),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(field("User", &self.user).flex_1())
                    .child(field("Jump host", &self.jump).flex_1()),
            )
            .child(self.logo_field(cx))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(tab("auth-pw", "Password", Mode::Password))
                    .child(tab("auth-key", "SSH key", Mode::Key))
                    .child(tab("auth-agent", "SSH agent", Mode::Agent)),
            )
            .when(mode == Mode::Password, |d| {
                d.child(field(
                    &format!("Password (stored in the {})", secrets::store().label()),
                    &self.password,
                ))
            })
            .when(mode == Mode::Agent, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(c(palette().muted))
                        .child("Logs in with the keys in your SSH agent (1Password, Secretive, ssh-add). Set another agent under Settings, Passwords."),
                )
            })
            .when(mode == Mode::Key, |d| {
                d.child(
                    div()
                        .flex()
                        .gap_2()
                        .items_end()
                        .child(field("Private key", &self.key_path).flex_1())
                        .child(
                            button("browse", "Browse")
                                .tab_index(1)
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
                d.child(div().text_xs().text_color(c(palette().red)).child(e))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("cancel", "Cancel")
                            .tab_index(1)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(FormEvent::Cancelled))),
                    )
                    .child(
                        primary_button("save", "Save")
                            .tab_index(1)
                            .on_click(cx.listener(|t, _, _, cx| t.submit(cx))),
                    ),
            )
    }
}
