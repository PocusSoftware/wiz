//! The one error type of the backend, and its `Result` alias.

use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("ssh: {0}")]
    Ssh(#[from] russh::Error),
    #[error("key: {0}")]
    Key(#[from] russh::keys::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[cfg(target_os = "macos")]
    #[error("keychain: {0}")]
    Keychain(#[from] security_framework::base::Error),
    #[error("docker: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("config: {0}")]
    Config(String),
    #[error("authentication failed")]
    AuthFailed,
    #[error("no usable credentials: {0}")]
    NoCredentials(String),
    #[error("key is encrypted, passphrase required")]
    PassphraseRequired,
    #[error("host key for {host}:{port} changed (known_hosts line {line}), refusing to connect")]
    HostKeyChanged {
        host: String,
        port: u16,
        line: usize,
    },
    #[error("host key for {host}:{port} rejected ({fingerprint})")]
    HostKeyRejected {
        host: String,
        port: u16,
        fingerprint: String,
    },
    #[error("sftp: {0}")]
    Sftp(String),
    #[error("transfer cancelled")]
    Cancelled,
    #[error("connection timed out after {0} s")]
    Timeout(u32),
    #[error("the secrets file is locked, enter the master password")]
    Locked,
    #[error("wrong master password")]
    WrongMasterPassword,
    #[error("secrets file: {0}")]
    Vault(String),
    #[error("remote command failed: {0}")]
    Remote(String),
}
