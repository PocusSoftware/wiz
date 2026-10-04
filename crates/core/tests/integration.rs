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

#[tokio::test]
async fn sftp_roundtrip_rename_remove_cancel() {
    use ssh_core::sftp::{self, Progress};
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
    let sftp = c.open_sftp().await.unwrap();

    let home = sftp.home().await.unwrap();
    assert_eq!(home, "/home/tester");
    let base = sftp::join(&home, "sftp-test");
    if sftp.exists(&base).await.unwrap() {
        sftp.remove(&base).await.unwrap();
    }
    sftp.mkdir(&base).await.unwrap();

    // Local tree: a.txt, sub/b.bin (300 KB, crosses chunk boundaries), sub/deep/c.txt
    let src = tempfile::tempdir().unwrap();
    let tree = src.path().join("tree");
    std::fs::create_dir_all(tree.join("sub/deep")).unwrap();
    std::fs::write(tree.join("a.txt"), "hello").unwrap();
    let blob: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(tree.join("sub/b.bin"), &blob).unwrap();
    std::fs::write(tree.join("sub/deep/c.txt"), "deep").unwrap();

    let up = Progress::new();
    let remote_tree = sftp::join(&base, "tree");
    sftp.upload(&tree, &remote_tree, &up).await.unwrap();
    assert_eq!(
        up.done.load(std::sync::atomic::Ordering::Relaxed),
        5 + 300_000 + 4
    );
    assert_eq!(up.fraction(), 1.0);

    let listing = sftp.list(&remote_tree).await.unwrap();
    assert_eq!(
        listing.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["sub", "a.txt"]
    );
    assert!(listing[0].is_dir && !listing[1].is_dir);
    assert_eq!(listing[1].size, 5);

    let back = src.path().join("back");
    sftp.download(&remote_tree, &back, &Progress::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(back.join("sub/b.bin")).unwrap(), blob);
    assert_eq!(
        std::fs::read_to_string(back.join("sub/deep/c.txt")).unwrap(),
        "deep"
    );

    // rename
    let renamed = sftp::join(&base, "tree2");
    sftp.rename(&remote_tree, &renamed).await.unwrap();
    assert!(!sftp.exists(&remote_tree).await.unwrap());
    assert!(sftp.is_dir(&renamed).await.unwrap());

    // cancel mid-transfer leaves no partial file behind
    let big = src.path().join("big.bin");
    std::fs::write(&big, vec![7u8; 8_000_000]).unwrap();
    let p = Progress::new();
    p.cancel();
    let r = sftp.upload(&big, &sftp::join(&base, "big.bin"), &p).await;
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert!(!sftp.exists(&sftp::join(&base, "big.bin")).await.unwrap());

    // recursive remove
    sftp.remove(&base).await.unwrap();
    assert!(!sftp.exists(&base).await.unwrap());
}

#[tokio::test]
async fn sftp_server_to_server_and_local_copy() {
    use ssh_core::sftp::{self, Progress};
    gate!();
    let d = tempfile::tempdir().unwrap();
    let v = verifier(&d, Arc::new(TrustAll));
    let a = Connection::connect(
        &host(2223, AuthMethod::Password),
        &Credentials::Key {
            path: keys().join("ed25519"),
            passphrase: None,
        },
        v.clone(),
    )
    .await
    .unwrap();
    let b = Connection::connect(
        &host(2222, AuthMethod::Password),
        &Credentials::Password("hunter2".into()),
        v,
    )
    .await
    .unwrap();
    let (sa, sb) = (a.open_sftp().await.unwrap(), b.open_sftp().await.unwrap());

    let root_a = "/home/tester/s2s-src";
    let root_b = "/home/tester/s2s-dst";
    for (s, p) in [(&sa, root_a), (&sb, root_b)] {
        if s.exists(p).await.unwrap() {
            s.remove(p).await.unwrap();
        }
    }
    sa.mkdir(root_a).await.unwrap();
    sb.mkdir(root_b).await.unwrap();

    // Seed server A with a tree via upload, then copy A -> B.
    let src = tempfile::tempdir().unwrap();
    let tree = src.path().join("t");
    std::fs::create_dir_all(tree.join("x/y")).unwrap();
    let blob: Vec<u8> = (0..400_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(tree.join("x/big.bin"), &blob).unwrap();
    std::fs::write(tree.join("x/y/small.txt"), "tiny").unwrap();
    sa.upload(&tree, &sftp::join(root_a, "t"), &Progress::new())
        .await
        .unwrap();

    let p = Progress::new();
    sa.copy_to(&sftp::join(root_a, "t"), &sb, &sftp::join(root_b, "t"), &p)
        .await
        .unwrap();
    assert_eq!(p.files.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(p.done.load(std::sync::atomic::Ordering::Relaxed), 400_004);

    // Verify on B by downloading.
    let back = src.path().join("back");
    sb.download(&sftp::join(root_b, "t"), &back, &Progress::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(back.join("x/big.bin")).unwrap(), blob);
    assert_eq!(
        std::fs::read_to_string(back.join("x/y/small.txt")).unwrap(),
        "tiny"
    );

    // Local -> local copy.
    let copy = src.path().join("copy");
    sftp::local::copy(&back, &copy, &Progress::new()).unwrap();
    assert_eq!(std::fs::read(copy.join("x/big.bin")).unwrap(), blob);

    for (s, p) in [(&sa, root_a), (&sb, root_b)] {
        s.remove(p).await.unwrap();
    }
}

#[tokio::test]
async fn sftp_symlink_loops_terminate_and_progress_is_exact() {
    use ssh_core::sftp::{self, Progress};
    use std::sync::atomic::Ordering::Relaxed;
    gate!();
    let d = tempfile::tempdir().unwrap();
    let c = Connection::connect(
        &host(2223, AuthMethod::Password),
        &Credentials::Key {
            path: keys().join("ed25519"),
            passphrase: None,
        },
        verifier(&d, Arc::new(TrustAll)),
    )
    .await
    .unwrap();
    let sftp = c.open_sftp().await.unwrap();
    let base = "/home/tester/loop-test";
    c.exec(&format!("rm -rf {base}; mkdir -p {base}/t/sub; echo data > {base}/t/sub/f.txt; ln -s .. {base}/t/sub/up; ln -s sub/f.txt {base}/t/link-to-file"))
        .await
        .unwrap();

    // Remote -> local: the loop link is skipped, the file link is followed, totals match.
    let local = tempfile::tempdir().unwrap();
    let p = Progress::new();
    sftp.download(&sftp::join(base, "t"), &local.path().join("t"), &p)
        .await
        .unwrap();
    assert_eq!(
        p.done.load(Relaxed),
        p.total.load(Relaxed),
        "progress must end at exactly 100%"
    );
    assert_eq!(p.files.load(Relaxed), 2);
    assert!(local.path().join("t/sub/f.txt").exists());
    assert!(local.path().join("t/link-to-file").exists());
    assert!(!local.path().join("t/sub/up").exists());

    // Remote -> remote (same server, second session) with the remote loop.
    let sftp2 = c.open_sftp().await.unwrap();
    let p = Progress::new();
    sftp.copy_to(
        &sftp::join(base, "t"),
        &sftp2,
        &sftp::join(base, "t-copy"),
        &p,
    )
    .await
    .unwrap();
    assert_eq!(p.done.load(Relaxed), p.total.load(Relaxed));
    assert!(
        !sftp2
            .exists(&sftp::join(base, "t-copy/sub/up"))
            .await
            .unwrap()
    );

    // Local -> remote with a local loop.
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("a")).unwrap();
    std::fs::write(src.path().join("a/f"), "x").unwrap();
    std::os::unix::fs::symlink(src.path(), src.path().join("a/loop")).unwrap();
    let p = Progress::new();
    sftp.upload(&src.path().join("a"), &sftp::join(base, "up-a"), &p)
        .await
        .unwrap();
    assert_eq!(p.done.load(Relaxed), p.total.load(Relaxed));
    assert!(!sftp.exists(&sftp::join(base, "up-a/loop")).await.unwrap());

    // Local -> local with the same loop.
    let p = Progress::new();
    sftp::local::copy(&src.path().join("a"), &local.path().join("copy-a"), &p).unwrap();
    assert_eq!(p.done.load(Relaxed), p.total.load(Relaxed));

    sftp.remove(base).await.unwrap();
}
