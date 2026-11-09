//! Secret store (encrypted file and Keychain) and quick connect targets, end to end.
//! The store is process-wide state, so everything runs in one test, in order.
//! Keychain and SSH steps need SSHC_DOCKER_TESTS=1 and the servers from /docker.

use std::sync::Arc;

use ssh_core::Error;
use ssh_core::config::{AuthMethod, Host, parse_target};
use ssh_core::hostkey::{HostKeyVerifier, TrustPrompt};
use ssh_core::secrets::{self, Kind, Secret, Store};
use ssh_core::session::{Connection, Credentials};

struct TrustAll;
impl TrustPrompt for TrustAll {
    fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
        true
    }
}

fn verifier(dir: &std::path::Path) -> HostKeyVerifier {
    HostKeyVerifier {
        known_hosts: dir.join("known_hosts"),
        prompt: Arc::new(TrustAll),
    }
}

/// The password test server as a saved host, from a typed quick connect target.
fn server() -> Host {
    let (user, host, port) = parse_target("ssh -p 2222 tester@127.0.0.1", "nobody").unwrap();
    Host {
        name: format!("{user}@{host}"),
        host,
        port,
        user,
        group: None,
        pinned: false,
        jump: None,
        os: None,
        logo: None,
        tags: Vec::new(),
        auth: AuthMethod::Password,
    }
}

#[tokio::test]
async fn secret_store_flow() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: set before anything reads the environment; this binary has a single test.
    unsafe { std::env::set_var("SSHC_CONFIG_DIR", dir.path()) };
    let docker = std::env::var("SSHC_DOCKER_TESTS").is_ok();
    let host = server();
    let acct = host.account();

    // File store, no file yet: locked.
    secrets::set_store(Store::File);
    assert!(!secrets::file_exists() && !secrets::ready());
    assert!(matches!(
        secrets::get(Kind::Password, &acct),
        Err(Error::Locked)
    ));
    assert!(matches!(Credentials::from_store(&host), Err(Error::Locked)));

    // First unlock creates the file.
    secrets::unlock("first-master").unwrap();
    assert!(secrets::file_exists() && secrets::ready());
    secrets::set(Kind::Password, &acct, "hunter2").unwrap();
    let text = std::fs::read_to_string(dir.path().join("secrets.toml")).unwrap();
    assert!(!text.contains("hunter2"));
    assert!(matches!(
        Credentials::from_store(&host),
        Ok(Credentials::Password(p)) if *p.reveal() == "hunter2"
    ));

    // Lock, wrong password, right password.
    secrets::lock();
    assert!(matches!(
        secrets::get(Kind::Password, &acct),
        Err(Error::Locked)
    ));
    assert!(matches!(
        secrets::unlock("nope"),
        Err(Error::WrongMasterPassword)
    ));
    assert!(!secrets::is_unlocked());
    secrets::unlock("first-master").unwrap();

    // Change the master password.
    secrets::change_master("second-master").unwrap();
    secrets::lock();
    assert!(matches!(
        secrets::unlock("first-master"),
        Err(Error::WrongMasterPassword)
    ));
    secrets::unlock("second-master").unwrap();
    assert_eq!(
        secrets::get(Kind::Password, &acct)
            .unwrap()
            .map(|s| s.reveal().to_string())
            .as_deref(),
        Some("hunter2")
    );

    // Delete.
    secrets::set(Kind::KeyPassphrase, "x@y:22", "pp").unwrap();
    secrets::delete(Kind::KeyPassphrase, "x@y:22").unwrap();
    assert!(
        secrets::get(Kind::KeyPassphrase, "x@y:22")
            .unwrap()
            .is_none()
    );

    if !docker {
        eprintln!("Keychain and SSH steps skipped (set SSHC_DOCKER_TESTS=1)");
        return;
    }

    // Saved host whose password lives in the secrets file connects.
    let creds = Credentials::from_store(&host).unwrap();
    let conn = Connection::connect(&host, &creds, verifier(dir.path()))
        .await
        .unwrap();
    assert_eq!(
        conn.exec("echo ok").await.unwrap().stdout_str().trim(),
        "ok"
    );

    // Quick connect: credentials typed, nothing stored.
    let conn = Connection::connect(
        &host,
        &Credentials::Password(Secret::new("hunter2").unwrap()),
        verifier(dir.path()),
    )
    .await
    .unwrap();
    assert_eq!(
        conn.exec("whoami").await.unwrap().stdout_str().trim(),
        "tester"
    );

    // Move file -> Keychain and back, under a throwaway account.
    #[cfg(target_os = "macos")]
    {
        let temp = format!("migrate-{}@localhost:1", std::process::id());
        secrets::set(Kind::Password, &temp, "moved").unwrap();
        let accounts = vec![temp.clone()];
        assert_eq!(
            secrets::migrate(&accounts, Store::File, Store::Keychain).unwrap(),
            1
        );
        assert!(secrets::get(Kind::Password, &temp).unwrap().is_none());
        secrets::set_store(Store::Keychain);
        assert_eq!(
            secrets::get(Kind::Password, &temp)
                .unwrap()
                .map(|s| s.reveal().to_string())
                .as_deref(),
            Some("moved")
        );
        assert_eq!(
            secrets::migrate(&accounts, Store::Keychain, Store::File).unwrap(),
            1
        );
        assert!(secrets::get(Kind::Password, &temp).unwrap().is_none());
        secrets::set_store(Store::File);
        assert_eq!(
            secrets::get(Kind::Password, &temp)
                .unwrap()
                .map(|s| s.reveal().to_string())
                .as_deref(),
            Some("moved")
        );
    }
}
