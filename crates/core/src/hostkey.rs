//! Host key verification against the app's own known_hosts file.

use std::path::PathBuf;
use std::sync::Arc;

use russh::keys::{HashAlg, PublicKey, known_hosts};

use crate::{Error, Result};

/// Decides whether to trust a host that is not in known_hosts yet.
pub trait TrustPrompt: Send + Sync {
    /// `fingerprint` is SHA256, e.g. `SHA256:abc...`. Return true to trust and remember.
    fn trust_new_host(&self, host: &str, port: u16, fingerprint: &str) -> bool;

    /// The host presents a different key than the one remembered. Return true to replace it.
    fn replace_changed_host(&self, _host: &str, _port: u16, _fingerprint: &str) -> bool {
        false
    }
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
    /// A changed key is an error unless the user agrees to replace the remembered one.
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
            Err(russh::keys::Error::KeyChanged { line }) => {
                let fp = key.fingerprint(HashAlg::Sha256).to_string();
                if !self.prompt.replace_changed_host(host, port, &fp) {
                    return Err(Error::HostKeyChanged {
                        host: host.into(),
                        port,
                        line,
                    });
                }
                self.remove_line(line)?;
                known_hosts::learn_known_hosts_path(host, port, key, &self.known_hosts)?;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Drop the 1-based `line` of known_hosts.
    fn remove_line(&self, line: usize) -> Result<()> {
        let text = std::fs::read_to_string(&self.known_hosts)?;
        let kept: String = text
            .split_inclusive('\n')
            .enumerate()
            .filter(|(i, _)| i + 1 != line)
            .map(|(_, l)| l)
            .collect();
        // Write then rename so a crash never leaves a truncated file.
        let tmp = self.known_hosts.with_extension("tmp");
        std::fs::write(&tmp, kept)?;
        std::fs::rename(tmp, &self.known_hosts)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
