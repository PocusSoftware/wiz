//! Master password dialog for the encrypted secrets file: unlock it, create it, or change
//! its password. The key derivation runs off the UI thread.

use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    SharedString, Window, div, prelude::*, px,
};
use ssh_core::secrets;

use crate::theme::*;
use crate::widgets::input::TextInput;
use crate::widgets::{button, icon_button, label, primary_button};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnlockMode {
    Unlock,
    Create,
    Change,
}

impl UnlockMode {
    /// Unlock when a secrets file exists, otherwise create one.
    pub fn for_file() -> Self {
        if secrets::file_exists() {
            Self::Unlock
        } else {
            Self::Create
        }
    }
}

pub enum UnlockEvent {
    Done,
    Cancelled,
}

impl EventEmitter<UnlockEvent> for UnlockDialog {}

/// Shortest master password accepted when creating or changing it.
const MIN_LEN: usize = 8;

pub struct UnlockDialog {
    mode: UnlockMode,
    password: Entity<TextInput>,
    confirm: Entity<TextInput>,
    error: Option<SharedString>,
    busy: bool,
    focus: FocusHandle,
}

impl UnlockDialog {
    pub fn new(mode: UnlockMode, cx: &mut Context<Self>) -> Self {
        Self {
            mode,
            password: cx.new(|cx| TextInput::new("", true, cx)),
            confirm: cx.new(|cx| TextInput::new("", true, cx)),
            error: None,
            busy: false,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_first(&self, window: &mut Window, cx: &gpui::App) {
        window.focus(&self.password.focus_handle(cx));
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let pw = self.password.read(cx).value().to_string();
        if self.mode != UnlockMode::Unlock {
            if pw.chars().count() < MIN_LEN {
                self.error = Some(format!("Use at least {MIN_LEN} characters.").into());
                cx.notify();
                return;
            }
            if pw != self.confirm.read(cx).value() {
                self.error = Some("The passwords don't match.".into());
                cx.notify();
                return;
            }
        } else if pw.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let mode = self.mode;
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move {
                    match mode {
                        UnlockMode::Change => secrets::change_master(&pw),
                        _ => secrets::unlock(&pw),
                    }
                })
                .await;
            this.update(cx, |t, cx| {
                t.busy = false;
                match r {
                    Ok(()) => cx.emit(UnlockEvent::Done),
                    Err(e) => {
                        t.error = Some(e.to_string().into());
                        t.password.update(cx, |i, _| i.set_value(""));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Focusable for UnlockDialog {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for UnlockDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, hint, action) = match self.mode {
            UnlockMode::Unlock => (
                "Unlock secrets file",
                "Passwords are kept in an encrypted file. Enter the master password to use them. It stays unlocked until Bastion quits or you lock it in Settings.",
                "Unlock",
            ),
            UnlockMode::Create => (
                "Create a master password",
                "Passwords will be kept in an encrypted file (Argon2id and AES-256). The master password can't be recovered: if you forget it, the saved passwords are gone.",
                "Create",
            ),
            UnlockMode::Change => (
                "Change master password",
                "Every saved secret is encrypted again under the new password.",
                "Change",
            ),
        };
        let two = self.mode != UnlockMode::Unlock;
        div()
            .id("unlock")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "enter" => t.submit(cx),
                    "escape" => cx.emit(UnlockEvent::Cancelled),
                    "tab" if t.mode != UnlockMode::Unlock => {
                        let next = if t.password.focus_handle(cx).is_focused(window) {
                            &t.confirm
                        } else {
                            &t.password
                        };
                        window.focus(&next.focus_handle(cx));
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .w(px(360.))
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
                    .child(div().text_sm().child(title))
                    .child(
                        icon_button("unlock-close", "x", "Close (Esc)", MUTED)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(UnlockEvent::Cancelled))),
                    ),
            )
            .child(div().text_xs().text_color(c(MUTED)).child(hint))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(if self.mode == UnlockMode::Change {
                        "New master password"
                    } else {
                        "Master password"
                    }))
                    .child(self.password.clone()),
            )
            .when(two, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(label("Repeat it"))
                        .child(self.confirm.clone()),
                )
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
                        button("unlock-cancel", "Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(UnlockEvent::Cancelled))),
                    )
                    .child(
                        primary_button("unlock-ok", if self.busy { "Working..." } else { action })
                            .on_click(cx.listener(|t, _, _, cx| t.submit(cx))),
                    ),
            )
    }
}
