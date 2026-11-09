//! Where passwords and key passphrases live: the macOS Keychain (default there) or the
//! encrypted secrets file (`vault`), chosen by the `secret_store` setting. Other systems
//! have only the secrets file. Everything that reads or writes a secret goes through here.

#[cfg(target_os = "macos")]
pub mod keychain;
mod sealed;
pub mod vault;

use std::sync::{Mutex, PoisonError, RwLock};

use crate::{Error, Result, locked};
pub use sealed::Secret;
use vault::Vault;

/// AES-GCM nonce length, for the secrets file and for secrets sealed in memory.
const NONCE_LEN: usize = 12;

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| Error::Vault(format!("random: {e}")))?;
    Ok(b)
}

/// Which secret of a host.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Password,
    KeyPassphrase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    #[cfg(target_os = "macos")]
    Keychain,
    File,
}

impl Store {
    /// The store a `secret_store` value names. Without a Keychain it is always the file.
    pub fn from_setting(s: &str) -> Self {
        #[cfg(target_os = "macos")]
        if s != "file" {
            return Store::Keychain;
        }
        let _ = s;
        Store::File
    }

    /// Name on its own, for a choice between stores.
    pub fn name(self) -> &'static str {
        match self {
            #[cfg(target_os = "macos")]
            Store::Keychain => "Keychain",
            Store::File => "Encrypted file",
        }
    }

    /// How the UI names it, after "the" ("stored in the Keychain").
    pub fn label(self) -> &'static str {
        match self {
            #[cfg(target_os = "macos")]
            Store::Keychain => "Keychain",
            Store::File => "secrets file",
        }
    }

    pub fn setting(self) -> &'static str {
        match self {
            #[cfg(target_os = "macos")]
            Store::Keychain => "keychain",
            Store::File => "file",
        }
    }
}

/// The stores this system has, the default first.
#[cfg(target_os = "macos")]
pub const STORES: &[Store] = &[Store::Keychain, Store::File];
#[cfg(not(target_os = "macos"))]
pub const STORES: &[Store] = &[Store::File];

#[cfg(target_os = "macos")]
static STORE: RwLock<Store> = RwLock::new(Store::Keychain);
#[cfg(not(target_os = "macos"))]
static STORE: RwLock<Store> = RwLock::new(Store::File);
/// The unlocked secrets file, if any. Locking drops it (and the key with it).
static VAULT: Mutex<Option<Vault>> = Mutex::new(None);

pub fn store() -> Store {
    *STORE.read().unwrap_or_else(PoisonError::into_inner)
}

pub fn set_store(s: Store) {
    *STORE.write().unwrap_or_else(PoisonError::into_inner) = s;
}

/// True when secrets can be read without asking for the master password.
pub fn ready() -> bool {
    match store() {
        #[cfg(target_os = "macos")]
        Store::Keychain => true,
        Store::File => is_unlocked(),
    }
}

pub fn is_unlocked() -> bool {
    locked(&VAULT).is_some()
}

/// Whether a secrets file has been created yet.
pub fn file_exists() -> bool {
    Vault::default_path().is_ok_and(|p| Vault::exists(&p))
}

/// Open the secrets file with `master`, creating it when there is none yet.
pub fn unlock(master: &str) -> Result<()> {
    let path = Vault::default_path()?;
    let v = if Vault::exists(&path) {
        Vault::unlock(&path, master)?
    } else {
        Vault::create(&path, master)?
    };
    *locked(&VAULT) = Some(v);
    Ok(())
}

pub fn lock() {
    *locked(&VAULT) = None;
}

pub fn change_master(new_master: &str) -> Result<()> {
    with_vault(|v| v.change_master(new_master))
}

fn with_vault<T>(f: impl FnOnce(&mut Vault) -> Result<T>) -> Result<T> {
    match locked(&VAULT).as_mut() {
        Some(v) => f(v),
        None => Err(Error::Locked),
    }
}

fn get_in(store: Store, kind: Kind, account: &str) -> Result<Option<Secret>> {
    match store {
        #[cfg(target_os = "macos")]
        Store::Keychain => keychain::get(kind, account),
        Store::File => with_vault(|v| v.get(kind, account)),
    }
}

fn set_in(store: Store, kind: Kind, account: &str, secret: &str) -> Result<()> {
    match store {
        #[cfg(target_os = "macos")]
        Store::Keychain => keychain::set(kind, account, secret),
        Store::File => with_vault(|v| v.set(kind, account, secret)),
    }
}

fn delete_in(store: Store, kind: Kind, account: &str) -> Result<()> {
    match store {
        #[cfg(target_os = "macos")]
        Store::Keychain => keychain::delete(kind, account),
        Store::File => with_vault(|v| v.delete(kind, account)),
    }
}

pub fn get(kind: Kind, account: &str) -> Result<Option<Secret>> {
    get_in(store(), kind, account)
}

pub fn set(kind: Kind, account: &str, secret: &str) -> Result<()> {
    set_in(store(), kind, account, secret)
}

pub fn delete(kind: Kind, account: &str) -> Result<()> {
    delete_in(store(), kind, account)
}

/// Move the secrets of `accounts` from one store to the other. Each secret is written to
/// the new store before it is removed from the old one, so a failure part way loses nothing.
/// Returns how many secrets moved. The secrets file must be unlocked.
pub fn migrate(accounts: &[String], from: Store, to: Store) -> Result<usize> {
    let mut moved = 0;
    for account in accounts {
        for kind in [Kind::Password, Kind::KeyPassphrase] {
            if let Some(s) = get_in(from, kind, account)? {
                set_in(to, kind, account, &s.reveal())?;
                delete_in(from, kind, account)?;
                moved += 1;
            }
        }
    }
    Ok(moved)
}
