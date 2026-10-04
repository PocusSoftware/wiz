//! Needs `docker compose up -d` in /docker and `docker/gen-keys.sh`.
//! Run with: SSHC_DOCKER_TESTS=1 cargo test -p ssh-core --test integration

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ssh_core::config::{AuthMethod, Host};
use ssh_core::docker;
use ssh_core::hostkey::{HostKeyVerifier, RejectUnknown, TrustPrompt};
use ssh_core::keychain::{self, Kind};
use ssh_core::session::{Connection, Credentials, Shell, ShellEvent};
use ssh_core::{Error, terminal::Terminal};

macro_rules! gate {
    () => {
        if std::env::var("SSHC_DOCKER_TESTS").is_err() {
            eprintln!("skipped (set SSHC_DOCKER_TESTS=1)");
            return;
        }
    };
}

struct TrustAll;
impl TrustPrompt for TrustAll {
    fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
        true
    }
}

fn keys() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docker/keys")
}

fn host(port: u16, auth: AuthMethod) -> Host {
    Host {
        name: "t".into(),
        host: "127.0.0.1".into(),
        port,
        user: "tester".into(),
        auth,
    }
}

fn verifier(dir: &tempfile::TempDir, prompt: Arc<dyn TrustPrompt>) -> HostKeyVerifier {
    HostKeyVerifier {
        known_hosts: dir.path().join("known_hosts"),
        prompt,
    }
}

async fn read_until(shell: &mut Shell, term: &mut Terminal, needle: &str) -> String {
    let fut = async {
        let mut raw = String::new();
        while let Some(ev) = shell.events.recv().await {
            match ev {
                ShellEvent::Data(d) => {
                    raw.push_str(&String::from_utf8_lossy(&d));
                    let reply = term.advance(&d);
                    if !reply.is_empty() {
                        shell.send(reply).await;
                    }
                    if raw.contains(needle) {
                        break;
                    }
                }
                ShellEvent::Exit(_) => break,
            }
        }
        raw
    };
    tokio::time::timeout(Duration::from_secs(15), fut)
        .await
        .unwrap_or_else(|_| panic!("timeout waiting for {needle:?}"))
}

#[tokio::test]
async fn password_auth_ok_and_wrong() {
    gate!();
    let d = tempfile::tempdir().unwrap();
    let h = host(2222, AuthMethod::Password);
    let c = Connection::connect(
        &h,
        &Credentials::Password("hunter2".into()),
        verifier(&d, Arc::new(TrustAll)),
    )
    .await
    .unwrap();
    let out = c.exec("whoami").await.unwrap();
    assert_eq!(out.stdout_str().trim(), "tester");

    let bad = Connection::connect(
        &h,
        &Credentials::Password("nope".into()),
        verifier(&d, Arc::new(TrustAll)),
    )
    .await;
    assert!(matches!(bad, Err(Error::AuthFailed)), "{:?}", bad.err());
}

#[tokio::test]
async fn key_auth_ed25519_and_encrypted_rsa() {
    gate!();
    let d = tempfile::tempdir().unwrap();
    let h = host(2223, AuthMethod::Password);
    let v = verifier(&d, Arc::new(TrustAll));

    let ed = Credentials::Key {
        path: keys().join("ed25519"),
        passphrase: None,
    };
    let c = Connection::connect(&h, &ed, v.clone()).await.unwrap();
    assert_eq!(
        c.exec("whoami").await.unwrap().stdout_str().trim(),
        "tester"
    );

    let rsa = Credentials::Key {
        path: keys().join("rsa_enc"),
        passphrase: Some("testpass".into()),
    };
    let c = Connection::connect(&h, &rsa, v.clone()).await.unwrap();
    assert_eq!(c.exec("echo ok").await.unwrap().stdout_str().trim(), "ok");

    let no_pass = Credentials::Key {
        path: keys().join("rsa_enc"),
        passphrase: None,
    };
    let r = Connection::connect(&h, &no_pass, v.clone()).await;
    assert!(matches!(r, Err(Error::PassphraseRequired)), "{:?}", r.err());

    // Password must not work on the key-only server.
    let r = Connection::connect(&h, &Credentials::Password("hunter2".into()), v).await;
    assert!(matches!(r, Err(Error::AuthFailed)));
}

#[tokio::test]
async fn host_key_policy() {
    gate!();
    let d = tempfile::tempdir().unwrap();
    let h = host(2222, AuthMethod::Password);
    let creds = Credentials::Password("hunter2".into());

    // Unknown + reject.
    let r = Connection::connect(&h, &creds, verifier(&d, Arc::new(RejectUnknown))).await;
    assert!(
        matches!(r, Err(Error::HostKeyRejected { .. })),
        "{:?}",
        r.err()
    );

    // Unknown + trust, then remembered (RejectUnknown now succeeds).
    Connection::connect(&h, &creds, verifier(&d, Arc::new(TrustAll)))
        .await
        .unwrap();
    Connection::connect(&h, &creds, verifier(&d, Arc::new(RejectUnknown)))
        .await
        .unwrap();

    // Changed key: plant a different ed25519 key for this host:port.
    let d2 = tempfile::tempdir().unwrap();
    let pubkey = std::fs::read_to_string(keys().join("ed25519.pub")).unwrap();
    let mut parts = pubkey.split_whitespace();
    let line = format!(
        "[127.0.0.1]:2222 {} {}\n",
        parts.next().unwrap(),
        parts.next().unwrap()
    );
    std::fs::write(d2.path().join("known_hosts"), line).unwrap();
    let r = Connection::connect(&h, &creds, verifier(&d2, Arc::new(TrustAll))).await;
    assert!(
        matches!(r, Err(Error::HostKeyChanged { .. })),
        "{:?}",
        r.err()
    );
}

#[tokio::test]
async fn sudo_password_prompt_in_pty() {
    gate!();
    let d = tempfile::tempdir().unwrap();
    let h = host(2222, AuthMethod::Password);
    let c = Connection::connect(
        &h,
        &Credentials::Password("hunter2".into()),
        verifier(&d, Arc::new(TrustAll)),
    )
    .await
    .unwrap();
    let mut sh = c.open_shell(100, 30, None).await.unwrap();
    let mut term = Terminal::new(100, 30);
    read_until(&mut sh, &mut term, "$").await;

    sh.send("sudo -k; sudo whoami; echo DONE_$?\n").await;
    read_until(&mut sh, &mut term, "password for tester").await;
    // Password typed raw; remote PTY has echo off.
    sh.send("hunter2\n").await;
    let rest = read_until(&mut sh, &mut term, "DONE_0").await;
    assert!(!rest.contains("hunter2"), "password was echoed: {rest:?}");
    assert!(term.screen_text().contains("root"));

    sh.resize(60, 20).await;
    term.resize(60, 20);
    sh.send("stty size\n").await;
    let out = read_until(&mut sh, &mut term, "20 60").await;
    assert!(out.contains("20 60"));
}

#[tokio::test]
async fn remote_docker_list_and_exec_shell() {
    gate!();
    let d = tempfile::tempdir().unwrap();
    let h = host(2223, AuthMethod::Password);
    let creds = Credentials::Key {
        path: keys().join("ed25519"),
        passphrase: None,
    };
    let c = Connection::connect(&h, &creds, verifier(&d, Arc::new(TrustAll)))
        .await
        .unwrap();
    let list = docker::Remote::new(&c).list().await.unwrap();
    let pw = list
        .iter()
        .find(|x| x.name == "sshc-password")
        .expect("sshc-password listed");
    assert!(pw.is_running());

    let cmd = docker::Remote::exec_shell_command(&pw.id).unwrap();
    let mut sh = c.open_shell(80, 24, Some(&cmd)).await.unwrap();
    let mut term = Terminal::new(80, 24);
    sh.send("hostname; echo EXEC_$?\n").await;
    read_until(&mut sh, &mut term, "EXEC_0").await;
}

#[tokio::test]
async fn local_docker_list() {
    gate!();
    let list = docker::Local::new().unwrap().list().await.unwrap();
    assert!(list.iter().any(|c| c.name == "sshc-key"));
}

#[test]
fn keychain_roundtrip() {
    gate!();
    let acct = format!("test-{}@localhost:1", std::process::id());
    assert_eq!(keychain::get(Kind::Password, &acct).unwrap(), None);
    keychain::set(Kind::Password, &acct, "s3cret").unwrap();
    assert_eq!(
        keychain::get(Kind::Password, &acct).unwrap().as_deref(),
        Some("s3cret")
    );
    keychain::set(Kind::Password, &acct, "changed").unwrap();
    assert_eq!(
        keychain::get(Kind::Password, &acct).unwrap().as_deref(),
        Some("changed")
    );
    keychain::delete(Kind::Password, &acct).unwrap();
    assert_eq!(keychain::get(Kind::Password, &acct).unwrap(), None);
}

#[tokio::test]
async fn dropping_shell_ends_remote_process() {
    gate!();
    let d = tempfile::tempdir().unwrap();
    let h = host(2223, AuthMethod::Password);
    let creds = Credentials::Key {
        path: keys().join("ed25519"),
        passphrase: None,
    };
    let c = Connection::connect(&h, &creds, verifier(&d, Arc::new(TrustAll)))
        .await
        .unwrap();
    let mut sh = c.open_shell(80, 24, None).await.unwrap();
    let mut term = Terminal::new(80, 24);
    read_until(&mut sh, &mut term, "$").await;
    sh.send("sleep 4242\n").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let running = c.exec("pgrep -f 'sleep 4242'").await.unwrap();
    assert_eq!(running.status, Some(0), "sleep should be running");
    drop(sh); // what closing a tab does
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = c.exec("pgrep -f 'sleep 4242'").await.unwrap();
    assert_ne!(after.status, Some(0), "remote process survived tab close");
}
