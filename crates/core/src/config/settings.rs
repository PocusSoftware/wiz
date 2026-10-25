//! App settings (`settings.toml`), every field optional with a default.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Default secret-file patterns (matched case-insensitively against a file name).
pub const DEFAULT_SECRET_FILES: &[&str] = &[
    ".env",
    ".env.*",
    "*.env",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "id_dsa",
    ".netrc",
    ".npmrc",
    ".pgpass",
];

/// Which end of the tab bar has the minimize, maximize and close buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowControls {
    Left,
    Right,
}

/// Optional `settings.toml` next to `hosts.toml`. Every field is optional; the getters
/// below give the defaults.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    // Appearance
    pub ui_font: Option<String>,
    pub terminal_font: Option<String>,
    pub terminal_font_size: Option<f32>,
    pub animations: Option<bool>,
    /// Color of buttons, the active tab, the cursor and focus, as "#RRGGBB".
    pub accent_color: Option<String>,
    pub window_controls: Option<WindowControls>,
    // Privacy
    pub hide_sensitive: Option<bool>,
    pub mask_hosts: Option<bool>,
    pub mask_users: Option<bool>,
    pub mask_ipv4: Option<bool>,
    pub mask_ipv6: Option<bool>,
    /// Mask port numbers (`host:2222`, `ssh -p 2222`, `Port 22`).
    pub mask_ports: Option<bool>,
    /// Mask values of `PASSWORD=...` / `token: ...` style assignments in output.
    pub mask_secrets: Option<bool>,
    /// Hide secret files (see `secret_file_patterns`) from file lists.
    pub hide_secret_files: Option<bool>,
    /// Extra words to mask everywhere.
    pub extra_mask: Option<Vec<String>>,
    pub secret_file_patterns: Option<Vec<String>>,
    // Files
    pub show_dotfiles: Option<bool>,
    pub rsync_hint_files: Option<u32>,
    pub rsync_hint_mb: Option<u32>,
    /// Run large local/server transfers with rsync when both sides have it.
    pub use_rsync: Option<bool>,
    /// Speed limit for file transfers (SFTP and rsync) in KB/s, 0 for none.
    pub transfer_limit_kbps: Option<u32>,
    /// Transfers that run at the same time, the rest wait in the queue.
    pub max_transfers: Option<u32>,
    /// Program that opens files edited from a server (`open -a`), empty for the system default.
    pub editor_app: Option<String>,
    /// Upload a file opened from a server again whenever it is saved.
    pub auto_upload_edits: Option<bool>,
    // Logs, Docker, terminal, network
    pub log_refresh_secs: Option<u32>,
    pub log_tail_lines: Option<u32>,
    pub docker_refresh_secs: Option<u32>,
    pub terminal_scrollback: Option<u32>,
    pub keepalive_secs: Option<u32>,
    /// Ask before closing a tab or pane that holds an SSH session.
    pub confirm_close: Option<bool>,
    /// Reconnect a terminal on its own after the connection drops (network loss, reboot).
    pub auto_reconnect: Option<bool>,
    /// Close a terminal tab or pane when its shell exits (`exit`, Ctrl+D).
    pub close_on_exit: Option<bool>,
    /// Refuse to quit while sessions are open.
    pub block_quit: Option<bool>,
    /// Give up connecting after this many seconds (0 waits forever).
    pub connect_timeout_secs: Option<u32>,
    // Keyboard
    /// Shortcuts the user changed: command id to keystrokes like "cmd-shift-d".
    pub shortcuts: Option<BTreeMap<String, String>>,
    // Credentials
    /// Where passwords and passphrases are kept: `keychain` or `file` (encrypted
    /// `secrets.toml`, unlocked with a master password).
    pub secret_store: Option<String>,
}

macro_rules! getters {
    ($($name:ident: $ty:ty = $default:expr;)*) => {
        impl Settings {
            $(pub fn $name(&self) -> $ty { self.$name.clone().unwrap_or_else(|| $default) })*
        }
    };
}

getters! {
    terminal_font_size: f32 = 13.0;
    animations: bool = true;
    accent_color: String = "#f05a29".into();
    window_controls: WindowControls = WindowControls::Right;
    hide_sensitive: bool = false;
    mask_hosts: bool = true;
    mask_users: bool = true;
    mask_ipv4: bool = true;
    mask_ipv6: bool = true;
    mask_ports: bool = true;
    mask_secrets: bool = true;
    hide_secret_files: bool = true;
    extra_mask: Vec<String> = Vec::new();
    secret_file_patterns: Vec<String> = DEFAULT_SECRET_FILES.iter().map(ToString::to_string).collect();
    show_dotfiles: bool = false;
    rsync_hint_files: u32 = 500;
    rsync_hint_mb: u32 = 1024;
    use_rsync: bool = true;
    transfer_limit_kbps: u32 = 0;
    max_transfers: u32 = 2;
    editor_app: String = String::new();
    auto_upload_edits: bool = true;
    log_refresh_secs: u32 = 2;
    log_tail_lines: u32 = 2000;
    docker_refresh_secs: u32 = 5;
    terminal_scrollback: u32 = 10_000;
    keepalive_secs: u32 = 30;
    confirm_close: bool = true;
    block_quit: bool = true;
    auto_reconnect: bool = true;
    close_on_exit: bool = true;
    connect_timeout_secs: u32 = 15;
    shortcuts: BTreeMap<String, String> = BTreeMap::new();
    secret_store: String = "keychain".into();
}

impl Settings {
    /// Write to `settings.toml` (atomic rename, creates the folder).
    pub fn save(&self) -> Result<()> {
        let dir = crate::config_dir()?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("settings.toml");
        let s = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn load() -> Self {
        let Ok(dir) = crate::config_dir() else {
            return Self::default();
        };
        let path = dir.join("settings.toml");
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }
}
