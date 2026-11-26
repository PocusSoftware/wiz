//! Backend for the ssh client: config, secrets (Keychain or encrypted file), SSH sessions, Docker, terminal state.
//! No UI dependency, builds with Command Line Tools only.

pub mod config;
pub mod docker;
pub mod error;
pub mod hostkey;
pub mod os;
pub mod privacy;
pub mod secrets;
pub mod session;
pub mod sftp;
pub mod terminal;

pub use error::{Error, Result};

/// Keychain service name and config directory name.
pub const APP_ID: &str = "software.pocus.wiz";

/// Lock `m`, carrying on with the data if another thread panicked while holding it: every
/// mutex here guards plain values that stay valid, and a poisoned lock must not take the
/// app down with it.
pub(crate) fn locked<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Where hosts, known_hosts and settings live. `SSHC_CONFIG_DIR` overrides it (used by
/// tests so they never touch real data).
pub fn config_dir() -> Result<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("SSHC_CONFIG_DIR") {
        return Ok(d.into());
    }
    platform_config_dir()
        .map(|d| d.join(APP_ID))
        .ok_or_else(|| Error::Config("no config dir".into()))
}

#[cfg(target_os = "macos")]
fn platform_config_dir() -> Option<std::path::PathBuf> {
    Some(std::env::home_dir()?.join("Library/Application Support"))
}

#[cfg(windows)]
fn platform_config_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_config_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(std::env::home_dir()?.join(".config")))
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
