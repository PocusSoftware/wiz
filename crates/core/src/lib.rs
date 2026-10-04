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
