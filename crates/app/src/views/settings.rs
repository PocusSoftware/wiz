//! Settings page: every behavior that can be configured, saved as you change it.

mod accent;
mod nav;
mod pages;
mod shortcuts;
#[cfg(test)]
mod tests;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    KeyDownEvent, Render, SharedString, Window, div, prelude::*, px,
};
use ssh_core::config::{Settings, WindowControls};
use ssh_core::secrets::{self, Store};

use crate::prefs;
use crate::theme::*;
use crate::widgets::input::{InputEvent, TextInput};
use crate::widgets::{button, checkbox, icon_button, keyboard, primary_button, tip};

/// Emitted after any change so the rest of the app can refresh. The secret store actions
/// need the master password dialog and the host list, so the root view handles them.
pub enum SettingsEvent {
    Changed,
    SwitchStore(Store),
    Unlock,
    Lock,
    ChangeMaster,
    /// Escape: back to the page Settings was opened from.
    Close,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

/// One page of settings, picked in the list on the left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Category {
    Appearance,
    Privacy,
    Passwords,
    Shortcuts,
    Files,
    LogsDocker,
    TerminalNetwork,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Appearance,
        Category::Privacy,
        Category::Passwords,
        Category::Shortcuts,
        Category::Files,
        Category::LogsDocker,
        Category::TerminalNetwork,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::Privacy => "Privacy",
            Category::Passwords => "Passwords",
            Category::Shortcuts => "Shortcuts",
            Category::Files => "Files (SFTP)",
            Category::LogsDocker => "Logs and Docker",
            Category::TerminalNetwork => "Terminal and network",
        }
    }
}

pub struct SettingsView {
    focus: FocusHandle,
    /// The page shown. It stays while Settings is closed and opened again.
    category: Category,
    ui_font: Entity<TextInput>,
    terminal_font: Entity<TextInput>,
    extra_mask: Entity<TextInput>,
    patterns: Entity<TextInput>,
    editor: Entity<TextInput>,
    agent_socket: Entity<TextInput>,
    /// The accent color as #RRGGBB.
    accent: Entity<TextInput>,
    /// Known editors that are installed (`platform::installed_editors`).
    editors: Vec<&'static str>,
    error: Option<SharedString>,
    /// The command whose shortcut is being typed, and the hold that keeps its keystrokes
    /// from running anything meanwhile.
    recording: Option<(&'static str, gpui::Subscription)>,
    /// Why the last typed shortcut was refused.
    shortcut_note: Option<SharedString>,
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let make = |placeholder: &str, cx: &mut Context<Self>| {
            cx.new(|cx| TextInput::new(placeholder, false, cx))
        };
        let this = Self {
            focus: cx.focus_handle(),
            category: Category::Appearance,
            ui_font: make("IBM Plex Sans", cx),
            terminal_font: make("JetBrainsMono Nerd Font", cx),
            extra_mask: make("project-name, client-name", cx),
            patterns: make(".env, *.pem", cx),
            editor: make("System default", cx),
            agent_socket: make(crate::platform::AGENT_DEFAULT, cx),
            accent: make("#f05a29", cx),
            editors: crate::platform::installed_editors(),
            error: None,
            recording: None,
            shortcut_note: None,
        };
        this.load_inputs(cx);
        for input in [
            &this.ui_font,
            &this.terminal_font,
            &this.extra_mask,
            &this.patterns,
            &this.editor,
            &this.agent_socket,
            &this.accent,
        ] {
            cx.subscribe(input, |t, _, _: &InputEvent, cx| t.sync_inputs(cx))
                .detach();
        }
        this
    }

    /// Fill the text fields from the current settings.
    fn load_inputs(&self, cx: &mut Context<Self>) {
        let s = prefs::get();
        let ui = s.ui_font.clone().unwrap_or_default();
        let term = s.terminal_font.clone().unwrap_or_default();
        let extra = s.extra_mask().join(", ");
        let pats = s.secret_file_patterns().join(", ");
        self.ui_font.update(cx, |i, _| i.set_value(&ui));
        self.terminal_font.update(cx, |i, _| i.set_value(&term));
        self.extra_mask.update(cx, |i, _| i.set_value(&extra));
        self.patterns.update(cx, |i, _| i.set_value(&pats));
        let editor = s.editor_app();
        self.editor.update(cx, |i, _| i.set_value(&editor));
        let socket = s.agent_socket();
        self.agent_socket.update(cx, |i, _| i.set_value(&socket));
        let accent = s.accent_color();
        self.accent.update(cx, |i, _| i.set_value(&accent));
    }

    fn sync_inputs(&mut self, cx: &mut Context<Self>) {
        let ui = self.ui_font.read(cx).value().trim().to_string();
        let term = self.terminal_font.read(cx).value().trim().to_string();
        let extra = split_list(self.extra_mask.read(cx).value());
        let pats = split_list(self.patterns.read(cx).value());
        let editor = self.editor.read(cx).value().trim().to_string();
        let socket = self.agent_socket.read(cx).value().trim().to_string();
        // Half-typed colors wait until they are complete.
        let accent = parse_hex(self.accent.read(cx).value());
        self.apply(cx, move |s| {
            s.ui_font = (!ui.is_empty()).then(|| ui.clone());
            s.terminal_font = (!term.is_empty()).then(|| term.clone());
            s.extra_mask = Some(extra.clone());
            s.secret_file_patterns = Some(pats.clone());
            s.editor_app = Some(editor);
            s.agent_socket = Some(socket);
            if let Some(rgb) = accent {
                s.accent_color = Some(format!("#{rgb:06x}"));
            }
        });
    }

    fn apply(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        self.error = prefs::update(f)
            .err()
            .map(|e| format!("Could not save settings: {e}").into());
        cx.emit(SettingsEvent::Changed);
        cx.notify();
    }

    fn row(label: &str, desc: &str, control: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap_4()
            .py(px(4.))
            .border_b_1()
            .border_color(c(BORDER_VARIANT))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().child(label.to_string()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(c(PLACEHOLDER))
                            .child(desc.to_string()),
                    ),
            )
            .child(div().flex_none().child(control))
    }

    fn title(title: &str) -> gpui::Div {
        div()
            .mt_4()
            .mb_1()
            .pb(px(6.))
            .border_b_2()
            .border_color(c(BORDER))
            .text_lg()
            .font_weight(FontWeight::SEMIBOLD)
            .child(title.to_string())
    }

    fn toggle(
        &self,
        id: &'static str,
        label: &str,
        desc: &str,
        on: bool,
        set: fn(&mut Settings, bool),
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        Self::row(
            label,
            desc,
            checkbox(id, on, "").on_click(cx.listener(move |t, _: &ClickEvent, _, cx| {
                t.apply(cx, move |s| set(s, !on));
            })),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one setting row: label, text, value, range and setter"
    )]
    fn stepper(
        &self,
        id: &'static str,
        label: &str,
        desc: &str,
        value: u32,
        (min, max, step): (u32, u32, u32),
        unit: &str,
        set: fn(&mut Settings, u32),
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let dec = value.saturating_sub(step).max(min);
        let inc = (value + step).min(max);
        Self::row(
            label,
            desc,
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    icon_button((id, 0usize), "minus", "Decrease", MUTED).on_click(
                        cx.listener(move |t, _, _, cx| t.apply(cx, move |s| set(s, dec))),
                    ),
                )
                .child(
                    div()
                        .w(px(96.))
                        .flex()
                        .justify_center()
                        // Zero means "off" for steppers that allow it (keep-alive, timeout).
                        .child(if value == 0 {
                            "Off".to_string()
                        } else if unit.is_empty() {
                            value.to_string()
                        } else {
                            format!("{value} {unit}")
                        }),
                )
                .child(
                    icon_button((id, 1usize), "plus", "Increase", MUTED).on_click(
                        cx.listener(move |t, _, _, cx| t.apply(cx, move |s| set(s, inc))),
                    ),
                ),
        )
    }

    /// Like `stepper`, but steps through fixed `choices` (for ranges where linear steps are
    /// useless, like speeds from 128 KB/s to 100 MB/s).
    #[expect(
        clippy::too_many_arguments,
        reason = "one setting row: label, text, value, range and setter"
    )]
    fn choice(
        &self,
        id: &'static str,
        label: &str,
        desc: &str,
        value: u32,
        choices: &[u32],
        show: fn(u32) -> String,
        set: fn(&mut Settings, u32),
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let dec = choices
            .iter()
            .rev()
            .find(|c| **c < value)
            .copied()
            .unwrap_or(choices[0]);
        let inc = choices
            .iter()
            .find(|c| **c > value)
            .copied()
            .unwrap_or(*choices.last().unwrap());
        Self::row(
            label,
            desc,
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    icon_button((id, 0usize), "minus", "Lower", MUTED).on_click(
                        cx.listener(move |t, _, _, cx| t.apply(cx, move |s| set(s, dec))),
                    ),
                )
                .child(div().w(px(96.)).flex().justify_center().child(show(value)))
                .child(
                    icon_button((id, 1usize), "plus", "Higher", MUTED).on_click(
                        cx.listener(move |t, _, _, cx| t.apply(cx, move |s| set(s, inc))),
                    ),
                ),
        )
    }

    /// Quick picks for the editor field: the system default plus installed known editors.
    fn editor_picks(&self, current: &str, cx: &mut Context<Self>) -> gpui::Div {
        let pick = |t: &mut Self, name: String, cx: &mut Context<Self>| {
            t.editor.update(cx, |i, _| i.set_value(&name));
            t.sync_inputs(cx);
        };
        let mut chips = div().flex().flex_wrap().gap_1().child(
            if current.is_empty() {
                primary_button("ed-default", "System default")
            } else {
                button("ed-default", "System default")
            }
            .on_click(cx.listener(move |t, _, _, cx| pick(t, String::new(), cx))),
        );
        for (i, name) in self.editors.iter().enumerate() {
            let b = if *name == current {
                primary_button(("ed", i), name)
            } else {
                button(("ed", i), name)
            };
            let name = name.to_string();
            chips =
                chips.child(b.on_click(cx.listener(move |t, _, _, cx| pick(t, name.clone(), cx))));
        }
        div()
            .py(px(4.))
            .border_b_1()
            .border_color(c(BORDER_VARIANT))
            .child(chips)
    }

    /// Keychain or encrypted file, plus lock state and master password for the file.
    fn credentials(&self, cx: &mut Context<Self>) -> gpui::Div {
        let store = secrets::store();
        let mut picks = div().flex().gap_1();
        for (i, &s) in secrets::STORES.iter().enumerate() {
            picks = picks.child(if store == s {
                primary_button(("s-store", i), s.name())
            } else {
                button(("s-store", i), s.name()).on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(SettingsEvent::SwitchStore(s))),
                )
            });
        }
        let mut d = div().flex().flex_col().child(if secrets::STORES.len() > 1 {
            Self::row(
                "Store passwords in",
                "Keychain uses macOS. Encrypted file keeps them in secrets.toml, sealed with AES-256 under a key from your master password (Argon2id). Switching moves every saved secret across.",
                picks,
            )
        } else {
            Self::row(
                "Passwords",
                "Kept in secrets.toml, sealed with AES-256 under a key from your master password (Argon2id).",
                div(),
            )
        });
        if store == Store::File {
            let unlocked = secrets::is_unlocked();
            let mut controls = div().flex().gap_1();
            if unlocked {
                controls =
                    controls
                        .child(button("s-change", "Change password").on_click(
                            cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::ChangeMaster)),
                        ))
                        .child(
                            button("s-lock", "Lock now")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Lock))),
                        );
            } else {
                controls = controls.child(
                    primary_button("s-unlock", "Unlock")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Unlock))),
                );
            }
            d = d.child(Self::row(
                if unlocked {
                    "Secrets file is unlocked"
                } else {
                    "Secrets file is locked"
                },
                "Unlocked until Wiz quits or you lock it. Connecting while locked asks for the master password.",
                controls,
            ));
        }
        d.child(self.text("SSH agent", crate::platform::AGENT_HINT, &self.agent_socket))
    }

    fn text(&self, label: &str, desc: &str, input: &Entity<TextInput>) -> gpui::Div {
        Self::row(label, desc, div().w(px(320.)).child(input.clone()))
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("settings-page")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    cx.emit(SettingsEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .child(self.nav(cx))
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .max_w(px(780.))
                            .px_4()
                            .pb_4()
                            .flex()
                            .flex_col()
                            .child(Self::title(self.category.name()))
                            .child(self.page(cx))
                            .when_some(self.error.clone(), |d, e| {
                                d.child(div().mt_2().text_color(c(RED)).child(e))
                            }),
                    ),
            )
    }
}
