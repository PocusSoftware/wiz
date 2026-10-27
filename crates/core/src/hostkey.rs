//! Host key verification against the app's own known_hosts file.

use std::path::PathBuf;
use std::sync::Arc;

use russh::keys::{HashAlg, PublicKey, known_hosts};

use crate::{Error, Result};

/// Decides whether to trust a host that is not in known_hosts yet.
pub trait TrustPrompt: Send + Sync {
    /// `fingerprint` is SHA256, e.g. `SHA256:abc...`. Return true to trust and remember.
    fn trust_new_host(&self, host: &str, port: u16, fingerprint: &str) -> bool;

    /// The host presents a different key (`fingerprint`) than the one remembered
    /// (`previous`). Return true to replace it.
    fn replace_changed_host(
        &self,
        _host: &str,
        _port: u16,
        _previous: &str,
        _fingerprint: &str,
    ) -> bool {
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
                    self.learn(host, port, key)
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
                let previous = known_hosts::known_host_keys_path(host, port, &self.known_hosts)?
                    .into_iter()
                    .find(|(l, _)| *l == line)
                    .map(|(_, k)| k.fingerprint(HashAlg::Sha256).to_string())
                    .unwrap_or_default();
                if !self.prompt.replace_changed_host(host, port, &previous, &fp) {
                    return Err(Error::HostKeyChanged {
                        host: host.into(),
                        port,
                        line,
                    });
                }
                self.remove_line(line)?;
                self.learn(host, port, key)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Remember `key` for the host, in an owner-only file (russh creates it world-readable).
    fn learn(&self, host: &str, port: u16, key: &PublicKey) -> Result<()> {
        known_hosts::learn_known_hosts_path(host, port, key, &self.known_hosts)?;
        #[cfg(unix)]
        std::fs::set_permissions(
            &self.known_hosts,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )?;
        Ok(())
    }

    /// Drop the 1-based `line` of known_hosts, counted the way russh counts it: comment
    /// lines don't count, so with comments the plain line number would hit another host.
    fn remove_line(&self, line: usize) -> Result<()> {
        let text = std::fs::read_to_string(&self.known_hosts)?;
        let mut n = 0;
        let kept: String = text
            .split_inclusive('\n')
            .filter(|l| {
                if l.starts_with('#') {
                    return true;
                }
                n += 1;
                n != line
            })
            .collect();
        crate::write_private(&self.known_hosts, kept.as_bytes())
    }
}

#[cfg(test)]
mod tests;
