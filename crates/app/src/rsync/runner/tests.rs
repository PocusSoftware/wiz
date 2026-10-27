//! Tests for the rsync runner and bridge.

use super::*;

#[tokio::test]
async fn rsync_over_connection() {
    if std::env::var("SSHC_DOCKER_TESTS").is_err() {
        eprintln!("skipped (set SSHC_DOCKER_TESTS=1)");
        return;
    }
    use ssh_core::config::{AuthMethod, Host};
    use ssh_core::hostkey::{HostKeyVerifier, TrustPrompt};
    use ssh_core::secrets::Secret;
    use ssh_core::session::Credentials;
    struct TrustAll;
    impl TrustPrompt for TrustAll {
        fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
            true
        }
    }
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    // SAFETY: set once, before anything reads it.
    unsafe { std::env::set_var("WIZ_RSH_EXE", root.join("target/debug/wiz")) };
    let tmp = std::env::temp_dir().join(format!("wiz-rsync-test-{}", std::process::id()));
    let (up, down) = (tmp.join("up"), tmp.join("down"));
    std::fs::create_dir_all(up.join("dir with space")).unwrap();
    std::fs::create_dir_all(&down).unwrap();
    std::fs::write(up.join("dir with space/it's.txt"), "hello").unwrap();
    std::fs::write(up.join("big.bin"), vec![7u8; 300_000]).unwrap();

    let host = Host {
        name: "t".into(),
        host: "127.0.0.1".into(),
        port: 2222,
        user: "tester".into(),
        group: None,
        pinned: false,
        jump: None,
        auth: AuthMethod::Password,
    };
    let verifier = HostKeyVerifier {
        known_hosts: tmp.join("known_hosts"),
        prompt: Arc::new(TrustAll),
    };
    let conn = Arc::new(
        Connection::connect(
            &host,
            &Credentials::Password(Secret::new("hunter2").unwrap()),
            verifier,
        )
        .await
        .unwrap(),
    );
    let local = local().await.expect("local rsync");
    assert!(conn.has_command("rsync").await);
    let remote_dir = format!("/tmp/wiz rsync {}", std::process::id());
    conn.exec(&format!("mkdir -p {}", q(&remote_dir)))
        .await
        .unwrap();
    let names = vec!["dir with space".to_string(), "big.bin".to_string()];
    let up_s = up.to_string_lossy().to_string();
    let down_s = down.to_string_lossy().to_string();

    let p = Progress::new();
    p.set_prescan(300_005, 2);
    run(
        conn.clone(),
        local,
        true,
        &up_s,
        &names,
        &remote_dir,
        p.clone(),
    )
    .await
    .unwrap();
    assert_eq!(p.done.load(Ordering::Relaxed), 300_005);
    let p = Progress::new();
    run(conn.clone(), local, false, &remote_dir, &names, &down_s, p)
        .await
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(down.join("dir with space/it's.txt")).unwrap(),
        "hello"
    );
    assert_eq!(std::fs::read(down.join("big.bin")).unwrap().len(), 300_000);
    // The speed limit holds: 300 KB at 150 KB/s takes about two seconds.
    conn.exec(&format!("rm -rf {}/big.bin", q(&remote_dir)))
        .await
        .unwrap();
    let p = Progress::new();
    p.limit.store(150_000, Ordering::Relaxed);
    let started = std::time::Instant::now();
    run(
        conn.clone(),
        local,
        true,
        &up_s,
        &["big.bin".to_string()],
        &remote_dir,
        p,
    )
    .await
    .unwrap();
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );

    conn.exec(&format!("rm -rf {}", q(&remote_dir)))
        .await
        .unwrap();
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn bridge_only_runs_rsync_servers() {
    assert!(allowed("rsync --server -logDtpre.iLsfxCIvu . /tmp/x/"));
    assert!(allowed("'rsync' '--server' '-g' '.' '/tmp/a b/'"));
    assert!(!allowed("rm -rf /"));
    assert!(!allowed("rsync --server ; rm -rf /"));
    assert!(allowed("rsync --server . /tmp/a\\$b/"));
    assert!(allowed("'rsync' '--server' '.' '/tmp/a;$b/'"));
    assert!(!allowed("rsync --daemon"));
}

#[test]
fn quoting_follows_rsync_version() {
    assert!(needs_quoting(
        "openrsync: protocol version 29\nrsync version 2.6.9 compatible"
    ));
    assert!(needs_quoting("rsync  version 3.2.3  protocol version 31"));
    assert!(!needs_quoting("rsync  version 3.2.4  protocol version 31"));
    assert!(!needs_quoting("rsync  version v3.4.1  protocol version 32"));
}
