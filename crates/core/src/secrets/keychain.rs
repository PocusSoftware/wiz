//! macOS Keychain storage via Security.framework (generic passwords).

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use zeroize::Zeroizing;

use super::{Kind, Secret};
use crate::Result;

/// errSecItemNotFound
const NOT_FOUND: i32 = -25300;

fn service(kind: Kind) -> String {
    match kind {
        Kind::Password => format!("{}.password", crate::APP_ID),
        Kind::KeyPassphrase => format!("{}.passphrase", crate::APP_ID),
    }
}

pub fn set(kind: Kind, account: &str, secret: &str) -> Result<()> {
    set_generic_password(&service(kind), account, secret.as_bytes())?;
    Ok(())
}

pub fn get(kind: Kind, account: &str) -> Result<Option<Secret>> {
    match get_generic_password(&service(kind), account) {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            std::str::from_utf8(&bytes)
                .ok()
                .map(Secret::new)
                .transpose()
        }
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete(kind: Kind, account: &str) -> Result<()> {
    match delete_generic_password(&service(kind), account) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == NOT_FOUND => Ok(()),
        Err(e) => Err(e.into()),
    }
}
