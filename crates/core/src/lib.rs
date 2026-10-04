//! Backend for the ssh client: config, keychain, SSH sessions, Docker, terminal state.
//! No UI dependency, builds with Command Line Tools only.

pub mod config;
pub mod docker;
pub mod error;
pub mod hostkey;
pub mod keychain;
pub mod session;
pub mod sftp;
pub mod terminal;

pub use error::{Error, Result};

/// Keychain service name and config directory name.
pub const APP_ID: &str = "dev.enty.ssh-client";

/// Where hosts, known_hosts and settings live. `SSHC_CONFIG_DIR` overrides it (used by
/// tests so they never touch real data).
pub fn config_dir() -> Result<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("SSHC_CONFIG_DIR") {
        return Ok(d.into());
    }
    dirs::config_dir()
        .map(|d| d.join(APP_ID))
        .ok_or_else(|| Error::Config("no config dir".into()))
}
