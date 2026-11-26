//! A secret kept encrypted in memory while the app holds on to it (open connections,
//! terminals, quick connect hosts), so a memory dump or a strings view of the process
//! doesn't show passwords in plain text. The key is random per process and never written
//! anywhere. This hides secrets from scans; it can't stop code running inside the process.

use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use zeroize::Zeroizing;

use super::{NONCE_LEN, random};
use crate::Result;

static KEY: OnceLock<Zeroizing<[u8; 32]>> = OnceLock::new();

fn cipher() -> Result<Aes256Gcm> {
    if KEY.get().is_none() {
        // A racing thread may set it first, then both use that one.
        let _ = KEY.set(Zeroizing::new(random()?));
    }
    let key = KEY.get().expect("set just above");
    Ok(Aes256Gcm::new_from_slice(key.as_ref()).expect("32-byte key"))
}

/// A password or passphrase, sealed with AES-256-GCM under the process key.
#[derive(Clone)]
pub struct Secret {
    /// nonce || ciphertext || tag
    sealed: Vec<u8>,
}

impl Secret {
    pub fn new(plain: &str) -> Result<Self> {
        let nonce_bytes = random::<NONCE_LEN>()?;
        let nonce = Nonce::try_from(&nonce_bytes[..]).expect("12-byte nonce");
        let ct = cipher()?
            .encrypt(&nonce, plain.as_bytes())
            .map_err(|_| crate::Error::Vault("encryption failed".into()))?;
        let mut sealed = nonce_bytes.to_vec();
        sealed.extend_from_slice(&ct);
        Ok(Self { sealed })
    }

    /// The plain text, wiped from memory when the returned value is dropped. Keep it only
    /// as long as it is needed.
    pub fn reveal(&self) -> Zeroizing<String> {
        let (n, ct) = self.sealed.split_at(NONCE_LEN);
        let nonce = Nonce::try_from(n).expect("12-byte nonce");
        let plain = cipher()
            .expect("the key exists once a secret was sealed")
            .decrypt(&nonce, ct)
            .expect("sealed in this process with the same key");
        Zeroizing::new(String::from_utf8(plain).expect("sealed from a str"))
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_bytes_hide_the_plain_text() {
        let s = Secret::new("hunter2-very-secret").unwrap();
        let needle = b"hunter2-very-secret";
        assert!(!s.sealed.windows(needle.len()).any(|w| w == needle));
        assert!(!format!("{s:?}").contains("hunter2"));
        assert_eq!(*s.reveal(), "hunter2-very-secret");
    }

    #[test]
    fn same_secret_seals_differently() {
        let (a, b) = (Secret::new("pw").unwrap(), Secret::new("pw").unwrap());
        assert_ne!(a.sealed, b.sealed);
        assert_eq!(*Secret::new("").unwrap().reveal(), "");
    }
}
