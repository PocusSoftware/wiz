//! macOS Keychain storage via Security.framework (generic passwords).

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use zeroize::Zeroizing;

use super::{Kind, Secret};
use crate::{APP_ID, OLD_APP_ID, Result};

/// errSecItemNotFound
const NOT_FOUND: i32 = -25300;

fn service(app_id: &str, kind: Kind) -> String {
    match kind {
        Kind::Password => format!("{app_id}.password"),
        Kind::KeyPassphrase => format!("{app_id}.passphrase"),
    }
}

pub fn set(kind: Kind, account: &str, secret: &str) -> Result<()> {
    set_generic_password(&service(APP_ID, kind), account, secret.as_bytes())?;
    Ok(())
}

/// The secret, moved over from the old app ID's entry the first time it is read.
pub fn get(kind: Kind, account: &str) -> Result<Option<Secret>> {
    let bytes = match read(&service(APP_ID, kind), account)? {
        Some(b) => b,
        None => {
            let old = service(OLD_APP_ID, kind);
            let Some(b) = read(&old, account)? else {
                return Ok(None);
            };
            set_generic_password(&service(APP_ID, kind), account, &b)?;
            remove(&old, account)?;
            b
        }
    };
    std::str::from_utf8(&bytes)
        .ok()
        .map(Secret::new)
        .transpose()
}

/// Removes the entry under the old app ID too, so no copy is left behind.
pub fn delete(kind: Kind, account: &str) -> Result<()> {
    remove(&service(APP_ID, kind), account)?;
    remove(&service(OLD_APP_ID, kind), account)
}

fn read(service: &str, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
    match get_generic_password(service, account) {
        Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn remove(service: &str, account: &str) -> Result<()> {
    match delete_generic_password(service, account) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == NOT_FOUND => Ok(()),
        Err(e) => Err(e.into()),
    }
}
