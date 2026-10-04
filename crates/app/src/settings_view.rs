//! Settings page: every behavior that can be configured, saved as you change it.

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    Render, SharedString, Window, div, prelude::*, px,
};
use ssh_core::config::Settings;

use crate::input::{InputEvent, TextInput};
use crate::prefs;
use crate::theme::*;
use crate::ui::{button, checkbox, icon_button};

/// Emitted after any change so the rest of the app can refresh.
pub enum SettingsEvent {
    Changed,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

pub struct SettingsView {
    focus: FocusHandle,
    ui_font: Entity<TextInput>,
    terminal_font: Entity<TextInput>,
    extra_mask: Entity<TextInput>,
    patterns: Entity<TextInput>,
    error: Option<SharedString>,
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
            ui_font: make("IBM Plex Sans", cx),
            terminal_font: make("JetBrainsMono Nerd Font", cx),
            extra_mask: make("project-name, client-name", cx),
            patterns: make(".env, *.pem", cx),
            error: None,
        };
        this.load_inputs(cx);
        for input in [
            &this.ui_font,
            &this.terminal_font,
            &this.extra_mask,
            &this.patterns,
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
    }

    fn sync_inputs(&mut self, cx: &mut Context<Self>) {
        let ui = self.ui_font.read(cx).value().trim().to_string();
        let term = self.terminal_font.read(cx).value().trim().to_string();
        let extra = split_list(self.extra_mask.read(cx).value());
        let pats = split_list(self.patterns.read(cx).value());
        self.apply(cx, move |s| {
            s.ui_font = (!ui.is_empty()).then(|| ui.clone());
            s.terminal_font = (!term.is_empty()).then(|| term.clone());
            s.extra_mask = Some(extra.clone());
            s.secret_file_patterns = Some(pats.clone());
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

    fn section(title: &str) -> gpui::Div {
        div()
            .mt_3()
            .mb_0p5()
            .pb(px(3.))
            .border_b_2()
            .border_color(c(BORDER))
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(ACCENT_TEXT))
            .child(title.to_uppercase())
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

    #[allow(clippy::too_many_arguments)]
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
                        .child(if unit.is_empty() {
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
        let s = prefs::get();
        let hide = s.hide_sensitive();

        div()
            .id("settings-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(780.))
                    .px_4()
                    .pb_4()
                    .flex()
                    .flex_col()
                    // ---------- Privacy ----------
                    .child(Self::section("Privacy"))
                    .child(self.toggle(
                        "p-hide",
                        "Hide sensitive information",
                        "Master switch. Everything below only applies while this is on.",
                        hide,
                        |s, v| s.hide_sensitive = Some(v),
                        cx,
                    ))
                    .child(self.toggle("p-hosts", "Mask hostnames", "Saved hosts' addresses, wherever they appear.", s.mask_hosts(), |s, v| s.mask_hosts = Some(v), cx))
                    .child(self.toggle("p-users", "Mask usernames", "Saved users and your macOS user.", s.mask_users(), |s, v| s.mask_users = Some(v), cx))
                    .child(self.toggle("p-v4", "Mask IPv4 addresses", "Any address like 10.0.0.5, even unsaved ones.", s.mask_ipv4(), |s, v| s.mask_ipv4 = Some(v), cx))
                    .child(self.toggle("p-v6", "Mask IPv6 addresses", "Full and compressed forms, ::1, zone ids (fe80::1%en0).", s.mask_ipv6(), |s, v| s.mask_ipv6 = Some(v), cx))
                    .child(self.toggle("p-ports", "Mask ports", "After addresses and saved hosts (host:2222), ssh -p 2222, Port 22, PORT=3000.", s.mask_ports(), |s, v| s.mask_ports = Some(v), cx))
                    .child(self.toggle("p-sec", "Mask secret values", "The value in PASSWORD=..., token: ..., \"api_key\": \"...\" lines.", s.mask_secrets(), |s, v| s.mask_secrets = Some(v), cx))
                    .child(self.toggle("p-files", "Hide secret files", "Leave files matching the patterns below out of file lists.", s.hide_secret_files(), |s, v| s.hide_secret_files = Some(v), cx))
                    .child(self.text("Secret file patterns", "Comma separated. * matches any run, ? one character.", &self.patterns))
                    .child(self.text("Extra words to mask", "Comma separated, 3+ letters, case-insensitive (project or client names).", &self.extra_mask))
                    // ---------- Appearance ----------
                    .child(Self::section("Appearance"))
                    .child(self.toggle("a-anim", "Animations", "Short fades and slides for dialogs, menus and pages.", s.animations(), |s, v| s.animations = Some(v), cx))
                    .child(self.stepper("a-size", "Terminal font size", "Applies immediately.", s.terminal_font_size() as u32, (8, 32, 1), "pt", |s, v| s.terminal_font_size = Some(v as f32), cx))
                    .child(self.text("Interface font", "Family name. Applies after a restart.", &self.ui_font))
                    .child(self.text("Terminal font", "Family name. Applies after a restart.", &self.terminal_font))
                    // ---------- Files ----------
                    .child(Self::section("Files (SFTP)"))
                    .child(self.toggle("f-dot", "Show dotfiles by default", "Initial state of the eye button in each pane.", s.show_dotfiles(), |s, v| s.show_dotfiles = Some(v), cx))
                    .child(self.stepper("f-rfiles", "Suggest rsync from", "Number of files in a transfer.", s.rsync_hint_files(), (50, 100_000, 50), "files", |s, v| s.rsync_hint_files = Some(v), cx))
                    .child(self.stepper("f-rmb", "...or from", "Total size of a transfer.", s.rsync_hint_mb(), (64, 102_400, 64), "MB", |s, v| s.rsync_hint_mb = Some(v), cx))
                    // ---------- Logs & Docker ----------
                    .child(Self::section("Logs and Docker"))
                    .child(self.stepper("l-refresh", "Log auto-scroll refresh", "How often followed logs reload.", s.log_refresh_secs(), (1, 60, 1), "s", |s, v| s.log_refresh_secs = Some(v), cx))
                    .child(self.stepper("l-tail", "Log lines to load", "Newest lines fetched per refresh.", s.log_tail_lines(), (100, 50_000, 100), "lines", |s, v| s.log_tail_lines = Some(v), cx))
                    .child(self.stepper("d-refresh", "Container list refresh", "How often the Docker panel reloads.", s.docker_refresh_secs(), (1, 60, 1), "s", |s, v| s.docker_refresh_secs = Some(v), cx))
                    // ---------- Terminal & network ----------
                    .child(Self::section("Terminal and network"))
                    .child(self.stepper("t-scroll", "Scrollback", "Lines kept per terminal. Applies to new terminals.", s.terminal_scrollback(), (1_000, 200_000, 1_000), "lines", |s, v| s.terminal_scrollback = Some(v), cx))
                    .child(self.stepper("n-keep", "SSH keep-alive", "Seconds between keep-alive probes, 0 turns them off. Applies to new connections.", s.keepalive_secs(), (0, 600, 5), "s", |s, v| s.keepalive_secs = Some(v), cx))
                    // ---------- footer ----------
                    .child(
                        div()
                            .mt_4()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(button("reset", "Reset to defaults").on_click(cx.listener(
                                |t, _, _, cx| {
                                    t.apply(cx, |s| *s = Settings::default());
                                    t.load_inputs(cx);
                                },
                            )))
                            .child(div().text_xs().text_color(c(PLACEHOLDER)).child(
                                ssh_core::config_dir()
                                    .map(|d| format!("Saved automatically to {}/settings.toml", d.display()))
                                    .unwrap_or_default(),
                            )),
                    )
                    .when_some(self.error.clone(), |d, e| d.child(div().mt_2().text_color(c(RED)).child(e)))
                    .child(div().mt_4().text_xs().text_color(c(PLACEHOLDER)).child(format!("Bastion {}", crate::version()))),
            )
    }
}
