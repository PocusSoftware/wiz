//! Saved hosts (`hosts.toml`): who to connect to and how, never the secrets themselves.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// How a host authenticates. Secrets never live here, only in the secret store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthMethod {
    /// Password stored in the secret store under this host's account.
    Password,
    /// Private key file. Passphrase (if any) stored in the secret store.
    Key { path: PathBuf },
    /// Keys held by an ssh-agent (1Password, Secretive, a YubiKey, `ssh-add`).
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    /// Folder the host is listed under on the hosts page. `None` is ungrouped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Listed at the top of the hosts page.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// Account (`user@host:port`) of the saved host to connect through, like `ssh -J`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<String>,
    /// Logo of the operating system found on the last connect (see `os::parse`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// Logo picked in the host form, shown instead of the detected one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    /// Free-form labels, searchable on the hosts page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Typed into the login shell once it opens, like a snippet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<String>,
    // Last: it serializes as a table, and TOML wants plain values before tables.
    pub auth: AuthMethod,
}

fn default_port() -> u16 {
    22
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl Host {
    /// Keychain account string, unique per user@host:port.
    pub fn account(&self) -> String {
        format!("{}@{}:{}", self.user, self.host, self.port)
    }

    /// Check the fields a user typed. Messages never repeat the values, since the form
    /// may be showing them masked.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.name.chars().count() > 64 {
            return Err("Name must be 64 characters or fewer.".into());
        }
        if self.name.chars().any(char::is_control) {
            return Err("Name can't contain control characters.".into());
        }
        if self.host.is_empty() {
            return Err("Host is required.".into());
        }
        if !valid_host(&self.host) {
            return Err(
                "Host must be a hostname or IP address, without user, port or scheme.".into(),
            );
        }
        if self.user.is_empty() {
            return Err("User is required.".into());
        }
        if self.user.len() > 64
            || self
                .user
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "@:/".contains(c))
        {
            return Err("User can't contain spaces, @, : or /.".into());
        }
        // ssh and rsync would read `-oProxyCommand=...@host` as an option.
        if self.user.starts_with('-') {
            return Err("User can't start with -.".into());
        }
        if self.port == 0 {
            return Err("Port must be 1-65535.".into());
        }
        if let Some(g) = &self.group
            && (g.chars().count() > 40 || g.chars().any(char::is_control))
        {
            return Err("Folder must be 40 characters or fewer.".into());
        }
        if let Some(cmd) = &self.startup
            && (cmd.chars().count() > 1000 || cmd.chars().any(char::is_control))
        {
            return Err("Startup command must be one line of 1000 characters or fewer.".into());
        }
        Ok(())
    }
}

/// An IPv4/IPv6 address or a DNS name (letters, digits, `-` and `_`, dot separated).
fn valid_host(h: &str) -> bool {
    if h.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    if h.len() > 253 {
        return false;
    }
    let h = h.strip_suffix('.').unwrap_or(h);
    // An all-digit last label is a mistyped IP (1.2.3, 999.1.1.1), not a name.
    let numeric_tld = h
        .rsplit('.')
        .next()
        .is_some_and(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()));
    !h.is_empty()
        && !numeric_tld
        && h.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, rename = "host")]
    pub hosts: Vec<Host>,
}

/// Most jump hosts in a row before the target.
pub const MAX_JUMPS: usize = 5;

impl Config {
    /// Point the hosts that go through `old` (an account) at `new`, after its user, host or
    /// port changed.
    pub fn retarget_jumps(&mut self, old: &str, new: &str) {
        for h in &mut self.hosts {
            if h.jump.as_deref() == Some(old) {
                h.jump = Some(new.into());
            }
        }
    }

    /// The saved hosts `host` connects through, the first one to dial first. Fails when a
    /// jump host is gone, the chain loops back, or it is longer than `MAX_JUMPS`.
    pub fn jump_chain(&self, host: &Host) -> std::result::Result<Vec<Host>, String> {
        let mut chain: Vec<Host> = Vec::new();
        let mut next = host.jump.clone();
        while let Some(account) = next {
            let Some(hop) = self.hosts.iter().find(|h| h.account() == account) else {
                return Err("The jump host is no longer saved.".into());
            };
            if hop.account() == host.account() || chain.iter().any(|h| h.account() == account) {
                return Err("The jump hosts go round in a circle.".into());
            }
            if chain.len() == MAX_JUMPS {
                return Err(format!("At most {MAX_JUMPS} jump hosts in a row."));
            }
            next = hop.jump.clone();
            chain.push(hop.clone());
        }
        chain.reverse();
        Ok(chain)
    }

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
        let s = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        crate::write_private(path, s.as_bytes())
    }
}
