//! What each settings page shows.

use super::*;

impl SettingsView {
    /// The rows of the current page.
    pub(super) fn page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let s = prefs::get();
        let d = div().flex().flex_col();
        match self.category {
            Category::Appearance => self.appearance_page(&s, cx),
            Category::Privacy => self.privacy_page(&s, cx),
            Category::Passwords => d.child(self.credentials(cx)),
            Category::Shortcuts => d.child(self.shortcut_rows(cx)),
            Category::Snippets => d.child(self.snippet_rows(cx)),
            Category::Files => self.files_page(&s, cx),
            Category::LogsDocker => self.logs_docker_page(&s, cx),
            Category::TerminalNetwork => self.terminal_network_page(&s, cx),
            Category::Experimental => self.experimental_page(&s, cx),
        }
    }

    fn appearance_page(&self, s: &Settings, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .child(Self::row(
                "Theme",
                "Light or dark colors. System follows the system setting.",
                div().flex().gap_1().children(
                    [
                        ("a-theme-system", "System", Appearance::System),
                        ("a-theme-light", "Light", Appearance::Light),
                        ("a-theme-dark", "Dark", Appearance::Dark),
                    ]
                    .map(|(id, name, look)| {
                        if s.appearance() == look {
                            primary_button(id, name)
                        } else {
                            button(id, name)
                        }
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.apply(cx, move |s| s.appearance = Some(look))
                        }))
                    }),
                ),
            ))
            .child(Self::row(
                "Contrast",
                "Stronger text and borders. System follows the system \
                 accessibility setting.",
                div().flex().gap_1().children(
                    [
                        ("a-contrast-system", "System", Contrast::System),
                        ("a-contrast-standard", "Standard", Contrast::Standard),
                        ("a-contrast-high", "High", Contrast::High),
                    ]
                    .map(|(id, name, contrast)| {
                        if s.contrast() == contrast {
                            primary_button(id, name)
                        } else {
                            button(id, name)
                        }
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.apply(cx, move |s| s.contrast = Some(contrast))
                        }))
                    }),
                ),
            ))
            .child(self.toggle(
                "a-colorful",
                "Colorful",
                "The accent color, and red, green and yellow icons and status \
                 dots. Off is the gray theme.",
                s.colorful(),
                |s, v| s.colorful = Some(v),
                cx,
            ))
            .child(self.accent_row(cx))
            .child(self.toggle(
                "a-oled",
                "OLED black",
                "Pure black behind the window and terminals of the dark \
                 theme, so OLED screens switch those pixels off.",
                s.oled_black(),
                |s, v| s.oled_black = Some(v),
                cx,
            ))
            .child(self.toggle(
                "a-blur",
                "Blurred background",
                "A see-through window with the desktop blurred behind it.",
                s.blur(),
                |s, v| s.blur = Some(v),
                cx,
            ))
            .child(Self::row(
                "Window buttons",
                "Which end of the tab bar has minimize, maximize and close.",
                div().flex().gap_1().children(
                    [
                        ("Left", WindowControls::Left),
                        ("Right", WindowControls::Right),
                    ]
                    .map(|(name, side)| {
                        if s.window_controls() == side {
                            primary_button(name, name)
                        } else {
                            button(name, name)
                        }
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.apply(cx, move |s| s.window_controls = Some(side))
                        }))
                    }),
                ),
            ))
            .child(self.toggle(
                "a-os-logos",
                "Server logos",
                "The operating system logo found when connecting, like Ubuntu \
                 or NixOS. A logo picked in the host form always shows.",
                s.show_os_logos(),
                |s, v| s.show_os_logos = Some(v),
                cx,
            ))
            .child(self.toggle(
                "a-anim",
                "Animations",
                "Short fades and slides for dialogs, menus and pages.",
                s.animations(),
                |s, v| s.animations = Some(v),
                cx,
            ))
            .child(self.text(
                "Interface font",
                "Family name. Applies after a restart.",
                &self.ui_font,
            ))
            .child(self.text(
                "Terminal font",
                "Family name. Applies after a restart.",
                &self.terminal_font,
            ))
            .child(Self::group("Tab bar"))
            .child(self.toggle(
                "a-latency",
                "Show latency",
                "The round trip to the server of the terminal in front, like 42 ms.",
                s.show_latency(),
                |s, v| s.show_latency = Some(v),
                cx,
            ))
            .child(self.toggle(
                "a-split-btn",
                "Split button",
                "Splits the terminal in front. The shortcut keeps working without it.",
                s.show_split_button(),
                |s, v| s.show_split_button = Some(v),
                cx,
            ))
            .child(self.toggle(
                "a-quick-btn",
                "Quick connect button",
                "The shortcut keeps working without it.",
                s.show_quick_connect_button(),
                |s, v| s.show_quick_connect_button = Some(v),
                cx,
            ))
            .child(self.toggle(
                "a-privacy-btn",
                "Privacy button",
                "The eye that hides hosts, users and ports. The setting stays \
                 under Privacy.",
                s.show_privacy_button(),
                |s, v| s.show_privacy_button = Some(v),
                cx,
            ))
    }

    fn privacy_page(&self, s: &Settings, cx: &mut Context<Self>) -> gpui::Div {
        let d = div().flex().flex_col();

        let on = s.hide_sensitive();
        // Dimmed, not disabled: options can still be set up while the switch is off.
        let options = div()
            .flex()
            .flex_col()
            .when(!on, |d| d.opacity(0.45))
            .child(Self::group("What to mask"))
            .child(self.toggle(
                "p-hosts",
                "Hostnames",
                "Saved hosts' addresses, wherever they appear.",
                s.mask_hosts(),
                |s, v| s.mask_hosts = Some(v),
                cx,
            ))
            .child(self.toggle(
                "p-users",
                "Usernames",
                "Saved users and your macOS user.",
                s.mask_users(),
                |s, v| s.mask_users = Some(v),
                cx,
            ))
            .child(self.toggle(
                "p-names",
                "Host names",
                "The names you gave saved hosts, like in the list and tab titles.",
                s.mask_names(),
                |s, v| s.mask_names = Some(v),
                cx,
            ))
            .child(self.toggle(
                "p-v4",
                "IPv4 addresses",
                "Any address like 10.0.0.5, even unsaved ones.",
                s.mask_ipv4(),
                |s, v| s.mask_ipv4 = Some(v),
                cx,
            ))
            .child(self.toggle(
                "p-v6",
                "IPv6 addresses",
                "Full and compressed forms, ::1, zone ids (fe80::1%en0).",
                s.mask_ipv6(),
                |s, v| s.mask_ipv6 = Some(v),
                cx,
            ))
            .child(self.toggle(
                "p-ports",
                "Ports",
                "After addresses and saved hosts (host:2222), ssh -p 2222, \
                 Port 22, PORT=3000.",
                s.mask_ports(),
                |s, v| s.mask_ports = Some(v),
                cx,
            ))
            .child(self.toggle(
                "p-sec",
                "Secret values",
                "The value in PASSWORD=..., token: ..., \"api_key\": \"...\" lines.",
                s.mask_secrets(),
                |s, v| s.mask_secrets = Some(v),
                cx,
            ))
            .child(self.toggle(
                "p-containers",
                "Containers",
                "Names and IDs of the containers listed in Docker tabs, wherever \
                 they appear.",
                s.mask_containers(),
                |s, v| s.mask_containers = Some(v),
                cx,
            ))
            .child(self.text(
                "Extra words",
                "Comma separated, 3+ letters, case-insensitive (project or \
                 client names).",
                &self.extra_mask,
            ))
            .child(Self::group("Files"))
            .child(self.toggle(
                "p-files",
                "Hide secret files",
                "Leave files matching the patterns below out of file lists.",
                s.hide_secret_files(),
                |s, v| s.hide_secret_files = Some(v),
                cx,
            ))
            .child(self.text(
                "Secret file patterns",
                "Comma separated. * matches any run, ? one character.",
                &self.patterns,
            ));
        d.child(self.toggle(
            "p-hide",
            "Hide sensitive information",
            if on {
                "On. Everything below is masked."
            } else {
                "Off. Turn on to apply the options below."
            },
            on,
            |s, v| s.hide_sensitive = Some(v),
            cx,
        ))
        .child(options)
    }

    fn files_page(&self, s: &Settings, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .child(self.toggle(
                "f-dot",
                "Show dotfiles by default",
                "Initial state of the eye button in each pane.",
                s.show_dotfiles(),
                |s, v| s.show_dotfiles = Some(v),
                cx,
            ))
            .child(self.choice(
                "f-limit",
                "Transfer speed limit",
                "For SFTP and rsync transfers. Changes apply to running transfers too.",
                s.transfer_limit_kbps(),
                &[
                    0, 128, 256, 512, 1024, 2048, 5120, 10240, 25600, 51200, 102400,
                ],
                |v| match v {
                    0 => "Off".to_string(),
                    v if v < 1024 => format!("{v} KB/s"),
                    v => format!("{} MB/s", v / 1024),
                },
                |s, v| s.transfer_limit_kbps = Some(v),
                cx,
            ))
            .child(self.editor_field(cx))
            .child(self.editor_picks(&s.editor_app(), cx))
            .child(self.toggle(
                "f-autoup",
                "Upload edited files on save",
                "Files opened from a server are uploaded again each time they \
                 are saved.",
                s.auto_upload_edits(),
                |s, v| s.auto_upload_edits = Some(v),
                cx,
            ))
            .child(self.stepper(
                "f-par",
                "Parallel transfers",
                "More than this wait in the queue. Pause and resume are on \
                 each transfer.",
                s.max_transfers(),
                (1, 8, 1),
                "",
                |s, v| s.max_transfers = Some(v),
                cx,
            ))
            // rsync and its hint exist only where the rsync bridge does.
            .when(crate::platform::RSYNC, |d| {
                d.child(self.toggle(
                    "f-rsync",
                    "Use rsync for large transfers",
                    "Between this Mac and a server, when both have rsync \
                 installed. It runs over Wiz's own connection, so password \
                 hosts work too. Otherwise files go over SFTP.",
                    s.use_rsync(),
                    |s, v| s.use_rsync = Some(v),
                    cx,
                ))
                .child(self.stepper(
                    "f-rfiles",
                    "Large transfer from",
                    "Number of files in a transfer.",
                    s.rsync_hint_files(),
                    (50, 100_000, 50),
                    "files",
                    |s, v| s.rsync_hint_files = Some(v),
                    cx,
                ))
                .child(self.stepper(
                    "f-rmb",
                    "...or from",
                    "Total size of a transfer.",
                    s.rsync_hint_mb(),
                    (64, 102_400, 64),
                    "MB",
                    |s, v| s.rsync_hint_mb = Some(v),
                    cx,
                ))
            })
    }

    fn logs_docker_page(&self, s: &Settings, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .child(self.stepper(
                "l-refresh",
                "Log auto-scroll refresh",
                "How often followed logs reload.",
                s.log_refresh_secs(),
                (1, 60, 1),
                "s",
                |s, v| s.log_refresh_secs = Some(v),
                cx,
            ))
            .child(self.stepper(
                "l-tail",
                "Log lines to load",
                "Newest lines fetched per refresh.",
                s.log_tail_lines(),
                (100, 50_000, 100),
                "lines",
                |s, v| s.log_tail_lines = Some(v),
                cx,
            ))
            .child(self.stepper(
                "d-refresh",
                "Container list refresh",
                "How often the Docker panel reloads.",
                s.docker_refresh_secs(),
                (1, 60, 1),
                "s",
                |s, v| s.docker_refresh_secs = Some(v),
                cx,
            ))
    }

    fn terminal_network_page(&self, s: &Settings, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .child(self.zoom_row(s, cx))
            .child(self.stepper(
                "t-scroll",
                "Scrollback",
                "Lines kept per terminal. Applies to new terminals.",
                s.terminal_scrollback(),
                (1_000, 200_000, 1_000),
                "lines",
                |s, v| s.terminal_scrollback = Some(v),
                cx,
            ))
            .child(self.stepper(
                "n-keep",
                "SSH keep-alive",
                "Seconds between keep-alive probes, 0 turns them off. Applies \
                 to new connections.",
                s.keepalive_secs(),
                (0, 600, 5),
                "s",
                |s, v| s.keepalive_secs = Some(v),
                cx,
            ))
            .child(self.toggle(
                "n-confirm",
                "Ask before closing a session",
                "Confirm before a terminal, files or Docker tab (or a split \
                 pane) on a server is closed.",
                s.confirm_close(),
                |s, v| s.confirm_close = Some(v),
                cx,
            ))
            .child(self.toggle(
                "n-updates",
                "Check for updates",
                "Looks for a new Wiz release on GitHub at start and every 6 hours, and offers \
                 it in the tab bar.",
                s.check_updates(),
                |s, v| s.check_updates = Some(v),
                cx,
            ))
            .child(self.toggle(
                "n-reconnect",
                "Reconnect lost terminals",
                "When a connection drops or the server reboots, keep retrying \
                 and reopen the terminal in the same tab.",
                s.auto_reconnect(),
                |s, v| s.auto_reconnect = Some(v),
                cx,
            ))
            .child(self.toggle(
                "n-exit",
                "Close terminal when the shell exits",
                "Typing exit or pressing Ctrl+D closes the tab, or the pane of a split.",
                s.close_on_exit(),
                |s, v| s.close_on_exit = Some(v),
                cx,
            ))
            .child(self.toggle(
                "n-quit",
                "Block quitting with open sessions",
                &format!(
                    "Quit ({} or the window close button) is refused while a \
                 terminal, files or Docker tab on a server is open.",
                    crate::platform::QUIT_KEYS
                ),
                s.block_quit(),
                |s, v| s.block_quit = Some(v),
                cx,
            ))
            .child(self.stepper(
                "n-timeout",
                "Connect timeout",
                "Give up on a connection that takes longer than this. 0 turns \
                 it off and waits until you cancel. Time spent on the host \
                 key prompt doesn't count.",
                s.connect_timeout_secs(),
                (0, 300, 5),
                "s",
                |s, v| s.connect_timeout_secs = Some(v),
                cx,
            ))
    }

    fn experimental_page(&self, s: &Settings, cx: &mut Context<Self>) -> gpui::Div {
        div().flex().flex_col().child(self.stepper(
            "x-splits",
            "Terminals per tab",
            "Most split terminals in one tab. Panes go in a grid and get \
                 small quickly, and each one is its own connection. Tabs \
                 already open keep their panes.",
            s.max_splits(),
            (2, 16, 1),
            "",
            |s, v| s.max_splits = Some(v),
            cx,
        ))
    }
}
