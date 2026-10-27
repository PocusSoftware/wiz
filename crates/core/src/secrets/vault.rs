//! Encrypted secrets file (`secrets.toml`), the alternative to the Keychain.
//!
//! The master password goes through Argon2id into a 256-bit key that lives only in memory.
//! Each secret is sealed with AES-256-GCM under its own random nonce, with its kind and
//! account as associated data, so entries can't be swapped between hosts. 256-bit symmetric
//! keys stay strong against quantum attacks (Grover's search leaves 128 bits), and there is
//! no public-key step that a quantum computer could break.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::{Kind, NONCE_LEN, Secret, random};
use crate::{Error, Result};

/// Format version: Argon2id key, AES-256-GCM entries.
const VERSION: u32 = 1;
const SALT_LEN: usize = 32;
/// Encrypted at creation and checked on unlock, so a wrong master password is caught even
/// when the file holds no secrets yet.
/// Part of the file format: it keeps the app's old name so existing files still open.
const CHECK: &[u8] = b"bastion-vault-v1";
const CHECK_AAD: &[u8] = b"check";

/// Argon2id cost: 64 MiB, 3 passes. Takes a fraction of a second, once per unlock.
const M_COST: u32 = 64 * 1024;
const T_COST: u32 = 3;
const P_COST: u32 = 1;
/// Highest costs a file may ask for: 1 GiB, 10 passes, 16 lanes. The costs are read from the
/// file, and a doctored one could otherwise make unlock run out of memory or never finish.
const MAX_COST: (u32, u32, u32) = (1 << 20, 10, 16);

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    salt: String,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    check: String,
    #[serde(default, rename = "secret")]
    secrets: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    kind: String,
    account: String,
    /// base64(nonce || ciphertext || tag)
    value: String,
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Password => "password",
        Kind::KeyPassphrase => "passphrase",
    }
}

fn derive(master: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<Zeroizing<[u8; 32]>> {
    let params = argon2::Params::new(m, t, p, Some(32)).map_err(|e| Error::Vault(e.to_string()))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(master.as_bytes(), salt, key.as_mut())
        .map_err(|e| Error::Vault(e.to_string()))?;
    Ok(key)
}

/// An unlocked secrets file. Changes are written straight away.
pub struct Vault {
    path: PathBuf,
    file: File,
    key: Zeroizing<[u8; 32]>,
}

impl Vault {
    pub fn default_path() -> Result<PathBuf> {
        Ok(crate::config_dir()?.join("secrets.toml"))
    }

    pub fn exists(path: &Path) -> bool {
        path.is_file()
    }

    /// Start a new, empty file protected by `master`. Refuses to replace an existing one.
    pub fn create(path: &Path, master: &str) -> Result<Self> {
        if Self::exists(path) {
            return Err(Error::Vault("a secrets file already exists".into()));
        }
        if master.is_empty() {
            return Err(Error::Vault("the master password can't be empty".into()));
        }
        let salt = random::<SALT_LEN>()?;
        let key = derive(master, &salt, M_COST, T_COST, P_COST)?;
        let mut v = Self {
            path: path.to_path_buf(),
            file: File {
                version: VERSION,
                salt: B64.encode(salt),
                m_cost: M_COST,
                t_cost: T_COST,
                p_cost: P_COST,
                check: String::new(),
                secrets: Vec::new(),
            },
            key,
        };
        v.file.check = v.seal(CHECK, CHECK_AAD)?;
        v.save()?;
        Ok(v)
    }

    /// Open an existing file. A wrong password gives `Error::WrongMasterPassword`.
    pub fn unlock(path: &Path, master: &str) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let file: File = toml::from_str(&text).map_err(|e| Error::Vault(e.to_string()))?;
        if file.version != VERSION {
            return Err(Error::Vault("unsupported secrets file format".into()));
        }
        let (m, t, p) = MAX_COST;
        if file.m_cost > m || file.t_cost > t || file.p_cost > p {
            return Err(Error::Vault(
                "the secrets file asks for too much work to open".into(),
            ));
        }
        let salt = B64
            .decode(&file.salt)
            .map_err(|e| Error::Vault(e.to_string()))?;
        let key = derive(master, &salt, file.m_cost, file.t_cost, file.p_cost)?;
        let v = Self {
            path: path.to_path_buf(),
            file,
            key,
        };
        match v.open(&v.file.check, CHECK_AAD) {
            Ok(p) if p.as_slice() == CHECK => Ok(v),
            _ => Err(Error::WrongMasterPassword),
        }
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(self.key.as_ref()).expect("32-byte key")
    }

    fn seal(&self, plain: &[u8], aad: &[u8]) -> Result<String> {
        let nonce_bytes = random::<NONCE_LEN>()?;
        let nonce = Nonce::try_from(&nonce_bytes[..]).expect("12-byte nonce");
        let ct = self
            .cipher()
            .encrypt(&nonce, Payload { msg: plain, aad })
            .map_err(|_| Error::Vault("encryption failed".into()))?;
        let mut out = nonce_bytes.to_vec();
        out.extend_from_slice(&ct);
        Ok(B64.encode(out))
    }

    fn open(&self, sealed: &str, aad: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        let raw = B64
            .decode(sealed)
            .map_err(|e| Error::Vault(e.to_string()))?;
        if raw.len() < NONCE_LEN {
            return Err(Error::Vault("damaged entry".into()));
        }
        let (n, ct) = raw.split_at(NONCE_LEN);
        let nonce = Nonce::try_from(n).expect("12-byte nonce");
        self.cipher()
            .decrypt(&nonce, Payload { msg: ct, aad })
            .map(Zeroizing::new)
            .map_err(|_| Error::Vault("a secret could not be decrypted (file damaged?)".into()))
    }

    fn aad(kind: Kind, account: &str) -> Vec<u8> {
        format!("{}\n{account}", kind_name(kind)).into_bytes()
    }

    pub fn get(&self, kind: Kind, account: &str) -> Result<Option<Secret>> {
        let Some(e) = self
            .file
            .secrets
            .iter()
            .find(|e| e.kind == kind_name(kind) && e.account == account)
        else {
            return Ok(None);
        };
        let plain = self.open(&e.value, &Self::aad(kind, account))?;
        let text =
            std::str::from_utf8(&plain).map_err(|_| Error::Vault("secret is not text".into()))?;
        Secret::new(text).map(Some)
    }

    pub fn set(&mut self, kind: Kind, account: &str, secret: &str) -> Result<()> {
        let value = self.seal(secret.as_bytes(), &Self::aad(kind, account))?;
        let entry = Entry {
            kind: kind_name(kind).into(),
            account: account.into(),
            value,
        };
        match self
            .file
            .secrets
            .iter_mut()
            .find(|e| e.kind == entry.kind && e.account == account)
        {
            Some(e) => *e = entry,
            None => self.file.secrets.push(entry),
        }
        self.save()
    }

    pub fn delete(&mut self, kind: Kind, account: &str) -> Result<()> {
        let before = self.file.secrets.len();
        self.file
            .secrets
            .retain(|e| !(e.kind == kind_name(kind) && e.account == account));
        if self.file.secrets.len() != before {
            self.save()?;
        }
        Ok(())
    }

    /// Re-encrypt everything under a new master password (new salt, new key).
    pub fn change_master(&mut self, new_master: &str) -> Result<()> {
        if new_master.is_empty() {
            return Err(Error::Vault("the master password can't be empty".into()));
        }
        let mut plain = Vec::new();
        for e in &self.file.secrets {
            let kind = if e.kind == "password" {
                Kind::Password
            } else {
                Kind::KeyPassphrase
            };
            plain.push((
                kind,
                e.account.clone(),
                self.open(&e.value, &Self::aad(kind, &e.account))?,
            ));
        }
        let salt = random::<SALT_LEN>()?;
        self.key = derive(new_master, &salt, M_COST, T_COST, P_COST)?;
        self.file.salt = B64.encode(salt);
        (self.file.m_cost, self.file.t_cost, self.file.p_cost) = (M_COST, T_COST, P_COST);
        self.file.check = self.seal(CHECK, CHECK_AAD)?;
        let mut secrets = Vec::new();
        for (kind, account, p) in plain {
            secrets.push(Entry {
                kind: kind_name(kind).into(),
                value: self.seal(&p, &Self::aad(kind, &account))?,
                account,
            });
        }
        self.file.secrets = secrets;
        self.save()
    }

    fn save(&self) -> Result<()> {
        let s = toml::to_string_pretty(&self.file).map_err(|e| Error::Vault(e.to_string()))?;
        crate::write_private(&self.path, s.as_bytes())
    }
}
