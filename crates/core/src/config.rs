use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// How a host authenticates. Secrets never live here, only in the keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthMethod {
    /// Password stored in the keychain under this host's account.
    Password,
    /// Private key file. Passphrase (if any) stored in the keychain.
    Key { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    pub auth: AuthMethod,
}

fn default_port() -> u16 {
    22
}

impl Host {
    /// Keychain account string, unique per user@host:port.
    pub fn account(&self) -> String {
        format!("{}@{}:{}", self.user, self.host, self.port)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, rename = "host")]
    pub hosts: Vec<Host>,
}

impl Config {
    pub fn default_path() -> Result<PathBuf> {
        Ok(crate::config_dir()?.join("hosts.toml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| Error::Config(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        // Write then rename so a crash never leaves a truncated file.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_and_roundtrip() {
        let d = Settings::default();
        assert!(!d.hide_sensitive() && d.mask_ipv6() && d.hide_secret_files() && d.animations());
        assert_eq!(
            (
                d.log_refresh_secs(),
                d.keepalive_secs(),
                d.rsync_hint_files()
            ),
            (2, 30, 500)
        );
        assert!(d.secret_file_patterns().contains(&".env".to_string()));

        let s = Settings {
            hide_sensitive: Some(true),
            mask_ipv6: Some(false),
            extra_mask: Some(vec!["falcon".into()]),
            terminal_font_size: Some(15.0),
            ..Default::default()
        };
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert!(back.hide_sensitive() && !back.mask_ipv6());
        assert_eq!(back.extra_mask(), vec!["falcon".to_string()]);
        assert_eq!(back.terminal_font_size(), 15.0);
        // Unset fields keep their defaults.
        assert!(back.mask_ipv4() && back.mask_ports() && back.log_tail_lines() == 2000);
    }

    #[test]
    fn roundtrip() {
        let cfg = Config {
            hosts: vec![
                Host {
                    name: "a".into(),
                    host: "h".into(),
                    port: 2222,
                    user: "u".into(),
                    auth: AuthMethod::Password,
                },
                Host {
                    name: "b".into(),
                    host: "h2".into(),
                    port: 22,
                    user: "u".into(),
                    auth: AuthMethod::Key { path: "/k".into() },
                },
            ],
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.hosts, cfg.hosts);
    }
}

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

/// Optional `settings.toml` next to `hosts.toml`. Every field is optional; the getters
/// below give the defaults.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    // Appearance
    pub ui_font: Option<String>,
    pub terminal_font: Option<String>,
    pub terminal_font_size: Option<f32>,
    pub animations: Option<bool>,
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
    // Logs, Docker, terminal, network
    pub log_refresh_secs: Option<u32>,
    pub log_tail_lines: Option<u32>,
    pub docker_refresh_secs: Option<u32>,
    pub terminal_scrollback: Option<u32>,
    pub keepalive_secs: Option<u32>,
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
    hide_sensitive: bool = false;
    mask_hosts: bool = true;
    mask_users: bool = true;
    mask_ipv4: bool = true;
    mask_ipv6: bool = true;
    mask_ports: bool = true;
    mask_secrets: bool = true;
    hide_secret_files: bool = true;
    extra_mask: Vec<String> = Vec::new();
    secret_file_patterns: Vec<String> = DEFAULT_SECRET_FILES.iter().map(|s| s.to_string()).collect();
    show_dotfiles: bool = false;
    rsync_hint_files: u32 = 500;
    rsync_hint_mb: u32 = 1024;
    log_refresh_secs: u32 = 2;
    log_tail_lines: u32 = 2000;
    docker_refresh_secs: u32 = 5;
    terminal_scrollback: u32 = 10_000;
    keepalive_secs: u32 = 30;
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
