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
        let dir = dirs::config_dir().ok_or_else(|| Error::Config("no config dir".into()))?;
        Ok(dir.join(crate::APP_ID).join("hosts.toml"))
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
