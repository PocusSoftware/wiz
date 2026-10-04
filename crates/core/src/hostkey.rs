//! Host key verification against the app's own known_hosts file.

use std::path::PathBuf;
use std::sync::Arc;

use russh::keys::{HashAlg, PublicKey, known_hosts};

use crate::{Error, Result};

/// Decides whether to trust a host that is not in known_hosts yet.
pub trait TrustPrompt: Send + Sync {
    /// `fingerprint` is SHA256, e.g. `SHA256:abc...`. Return true to trust and remember.
    fn trust_new_host(&self, host: &str, port: u16, fingerprint: &str) -> bool;
}

/// Trusts nothing unknown. Safe default for headless use.
pub struct RejectUnknown;
impl TrustPrompt for RejectUnknown {
    fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct HostKeyVerifier {
    pub known_hosts: PathBuf,
    pub prompt: Arc<dyn TrustPrompt>,
}

impl HostKeyVerifier {
    pub fn default_path() -> Result<PathBuf> {
        Ok(crate::config_dir()?.join("known_hosts"))
    }

    /// Ok if the key is known and matches, or unknown and the user trusts it.
    /// A changed key is always an error.
    pub fn verify(&self, host: &str, port: u16, key: &PublicKey) -> Result<()> {
        match known_hosts::check_known_hosts_path(host, port, key, &self.known_hosts) {
            Ok(true) => Ok(()),
            Ok(false) => {
                let fp = key.fingerprint(HashAlg::Sha256).to_string();
                if self.prompt.trust_new_host(host, port, &fp) {
                    known_hosts::learn_known_hosts_path(host, port, key, &self.known_hosts)?;
                    Ok(())
                } else {
                    Err(Error::HostKeyRejected {
                        host: host.into(),
                        port,
                        fingerprint: fp,
                    })
                }
            }
            Err(russh::keys::Error::KeyChanged { line }) => Err(Error::HostKeyChanged {
                host: host.into(),
                port,
                line,
            }),
            Err(e) => Err(e.into()),
        }
    }
}
