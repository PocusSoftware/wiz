//! Backend for the ssh client: config, secrets (Keychain or encrypted file), SSH sessions, Docker, terminal state.
//! No UI dependency, builds with Command Line Tools only.

pub mod config;
pub mod docker;
pub mod error;
pub mod hostkey;
pub mod secrets;
pub mod session;
pub mod sftp;
pub mod terminal;

pub use error::{Error, Result};

/// Keychain service name and config directory name.
pub const APP_ID: &str = "software.pocus.wiz";
/// What `APP_ID` was before. Data found under it is moved over on first use.
pub(crate) const OLD_APP_ID: &str = "dev.enty.ssh-client";

/// Where hosts, known_hosts and settings live. `SSHC_CONFIG_DIR` overrides it (used by
/// tests so they never touch real data).
pub fn config_dir() -> Result<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("SSHC_CONFIG_DIR") {
        return Ok(d.into());
    }
    dirs::config_dir()
        .map(|d| app_dir(&d))
        .ok_or_else(|| Error::Config("no config dir".into()))
}

/// The app's folder in `base`, renamed from the old ID's folder when only that one exists.
/// If the rename fails the old folder stays in use, so nothing ever looks lost.
fn app_dir(base: &std::path::Path) -> std::path::PathBuf {
    let (new, old) = (base.join(APP_ID), base.join(OLD_APP_ID));
    if new.exists() || !old.exists() || std::fs::rename(&old, &new).is_ok() {
        new
    } else {
        old
    }
}

/// Write `bytes` to `path` through a temp file and a rename, so a crash never leaves it half
/// written. Owner-only (mode 0600 on unix; on Windows the per-user config folder already keeps
/// other users out): even files without secrets list hosts and users.
pub(crate) fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
