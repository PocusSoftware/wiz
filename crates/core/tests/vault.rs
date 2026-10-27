//! The encrypted secrets file: encryption, wrong password, tampering, change of password.

use ssh_core::Error;
use ssh_core::secrets::Kind;
use ssh_core::secrets::vault::Vault;

#[test]
fn roundtrip_wrong_password_and_tamper() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.toml");
    let mut v = Vault::create(&path, "correct horse").unwrap();
    v.set(Kind::Password, "u@h:22", "s3cret").unwrap();
    v.set(Kind::KeyPassphrase, "u@h:22", "pp").unwrap();
    assert!(Vault::create(&path, "x").is_err());

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("s3cret") && text.contains("u@h:22"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let v = Vault::unlock(&path, "correct horse").unwrap();
    assert_eq!(
        v.get(Kind::Password, "u@h:22")
            .unwrap()
            .map(|s| s.reveal().to_string())
            .as_deref(),
        Some("s3cret")
    );
    assert_eq!(
        v.get(Kind::KeyPassphrase, "u@h:22")
            .unwrap()
            .map(|s| s.reveal().to_string())
            .as_deref(),
        Some("pp")
    );
    assert!(v.get(Kind::Password, "other@h:22").unwrap().is_none());
    assert!(matches!(
        Vault::unlock(&path, "wrong"),
        Err(Error::WrongMasterPassword)
    ));

    // Moving a sealed value to another account fails: the account is authenticated.
    let mut file: toml::Table = toml::from_str(&text).unwrap();
    file["secret"].as_array_mut().unwrap()[0]["account"] = "evil@h:22".into();
    std::fs::write(&path, toml::to_string(&file).unwrap()).unwrap();
    let v = Vault::unlock(&path, "correct horse").unwrap();
    assert!(v.get(Kind::Password, "evil@h:22").is_err());
}

#[test]
fn delete_and_change_master() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.toml");
    let mut v = Vault::create(&path, "one").unwrap();
    v.set(Kind::Password, "a@h:22", "pa").unwrap();
    v.set(Kind::Password, "b@h:22", "pb").unwrap();
    v.set(Kind::Password, "a@h:22", "pa2").unwrap();
    v.delete(Kind::Password, "b@h:22").unwrap();
    v.change_master("two").unwrap();
    assert!(matches!(
        Vault::unlock(&path, "one"),
        Err(Error::WrongMasterPassword)
    ));
    let v = Vault::unlock(&path, "two").unwrap();
    assert_eq!(
        v.get(Kind::Password, "a@h:22")
            .unwrap()
            .map(|s| s.reveal().to_string())
            .as_deref(),
        Some("pa2")
    );
    assert!(v.get(Kind::Password, "b@h:22").unwrap().is_none());
}

#[test]
fn excessive_key_costs_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.toml");
    Vault::create(&path, "pw").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    // 4 TiB of memory: refused before any key derivation starts.
    let line = text.lines().find(|l| l.starts_with("m_cost")).unwrap();
    std::fs::write(&path, text.replace(line, "m_cost = 4294967295")).unwrap();
    assert!(matches!(Vault::unlock(&path, "pw"), Err(Error::Vault(_))));
}
