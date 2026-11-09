//! Tests for `privacy`.

use super::*;

/// The mask that stands in for any span. The argument is the hidden text's length,
/// kept for readability only: the mask never depends on it.
fn b(_n: usize) -> String {
    mask()
}

fn setup(f: impl FnOnce(&mut Settings)) {
    let mut s = Settings {
        hide_sensitive: Some(true),
        ..Default::default()
    };
    f(&mut s);
    configure(&s);
    set_known(
        vec!["my-server.example.com".into(), "ab".into()],
        vec!["tester".into()],
        vec!["rekade".into()],
    );
}

// The statics are shared, so everything runs in one test.
#[test]
fn privacy_engine() {
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Off: nothing changes.
    configure(&Settings::default());
    assert_eq!(redact("tester@10.0.0.5"), "tester@10.0.0.5");
    assert!(!file_hidden(".env"));

    setup(|_| {});
    // Names and IPv4, keeping length.
    assert_eq!(
        redact("tester@10.0.0.5:22"),
        format!("{}@{}:{}", b(6), b(8), b(2))
    );
    assert_eq!(
        redact("TESTER testers retest"),
        format!("{} testers retest", b(6))
    );
    assert_eq!(redact("/home/tester/file"), format!("/home/{}/file", b(6)));
    assert_eq!(redact("my-server.example.com ok"), format!("{} ok", b(21)));
    assert_eq!(redact("999.1.1.1 1.2.3.4.5"), "999.1.1.1 1.2.3.4.5");
    assert_eq!(redact("ab ab"), "ab ab");

    // IPv6: full, compressed, loopback, bracketed with port, mapped IPv4, zone id.
    assert_eq!(redact("2001:db8::1"), b(11));
    assert_eq!(redact("fe80::1%en0 up"), format!("{} up", b(11)));
    assert_eq!(redact("[::1]:22"), format!("[{}]:{}", b(3), b(2)));
    assert_eq!(
        redact("2001:0db8:0000:0000:0000:ff00:0042:8329."),
        format!("{}.", b(39))
    );
    assert_eq!(redact("::ffff:192.168.1.1"), b(18));
    // Not addresses: times, MACs, C++ scopes, plain words.
    assert_eq!(
        redact("at 10:45:30 mac aa:bb:cc:dd:ee:ff"),
        "at 10:45:30 mac aa:bb:cc:dd:ee:ff"
    );
    assert_eq!(redact("std::string a::b"), "std::string a::b");

    // Secret-looking assignments.
    assert_eq!(
        redact("PASSWORD=hunter2 ok"),
        format!("PASSWORD={} ok", b(7))
    );
    assert_eq!(
        redact("api_key: \"abc def\" x"),
        format!("api_key: \"{}\" x", b(7))
    );
    assert_eq!(
        redact("{\"token\": \"xyz\"}"),
        format!("{{\"token\": \"{}\"}}", b(3))
    );
    assert_eq!(
        redact("PATH=/usr/bin user=alice"),
        "PATH=/usr/bin user=alice"
    );

    // Ports: after addresses and saved hosts, by name, and the ssh -p flag.
    assert_eq!(
        redact("my-server.example.com:2222"),
        format!("{}:{}", b(21), b(4))
    );
    assert_eq!(redact("ssh -p 2222 x"), format!("ssh -p {} x", b(4)));
    assert_eq!(redact("Port 22"), format!("Port {}", b(2)));
    assert_eq!(redact("PORT=3000"), format!("PORT={}", b(4)));
    assert_eq!(redact("db_port: 5432"), format!("db_port: {}", b(4)));
    assert_eq!(redact("--port=8080"), format!("--port={}", b(4)));
    assert_eq!(
        redact("report: 5 ports 80 ps -p 42 x:99"),
        "report: 5 ports 80 ps -p 42 x:99"
    );
    assert_eq!(redact("port 99999"), "port 99999");
    assert_eq!(port(2222), b(4));
    assert_eq!(port("80-90"), format!("{}-{}", b(2), b(2)));

    // Files.
    assert!(file_hidden(".env"));
    assert!(file_hidden(".ENV.production"));
    assert!(file_hidden("prod.env"));
    assert!(file_hidden("server.pem"));
    assert!(file_hidden("id_ed25519"));
    assert!(!file_hidden("id_ed25519.pub"));
    assert!(!file_hidden("environment.txt"));
    assert!(!file_hidden("readme.md"));

    // Every category can be switched off on its own.
    setup(|s| s.mask_ipv6 = Some(false));
    assert_eq!(redact("2001:db8::1"), "2001:db8::1");
    setup(|s| s.mask_ipv4 = Some(false));
    assert_eq!(redact("10.0.0.5"), "10.0.0.5");
    setup(|s| s.mask_secrets = Some(false));
    assert_eq!(redact("PASSWORD=hunter2"), "PASSWORD=hunter2");
    setup(|s| s.mask_users = Some(false));
    assert_eq!(redact("tester"), "tester");
    // Host names are off by default.
    assert_eq!(redact("rekade"), "rekade");
    setup(|s| s.mask_names = Some(true));
    assert_eq!(redact("rekade"), b(6));
    setup(|s| s.mask_hosts = Some(false));
    assert_eq!(redact("my-server.example.com"), "my-server.example.com");
    assert_eq!(
        redact("my-server.example.com:2222"),
        format!("my-server.example.com:{}", b(4))
    );
    setup(|s| s.mask_ipv4 = Some(false));
    assert_eq!(redact("10.0.0.5:22"), format!("10.0.0.5:{}", b(2)));
    setup(|s| s.mask_ports = Some(false));
    assert_eq!(
        redact("tester@10.0.0.5:22"),
        format!("{}@{}:22", b(6), b(8))
    );
    assert_eq!(redact("ssh -p 2222 PORT=1"), "ssh -p 2222 PORT=1");
    assert_eq!(port(22), "22");
    setup(|s| s.hide_secret_files = Some(false));
    assert!(!file_hidden(".env"));
    setup(|s| s.secret_file_patterns = Some(vec!["*.sql".into()]));
    assert!(file_hidden("dump.SQL"));
    assert!(!file_hidden(".env"));
    setup(|s| s.extra_mask = Some(vec!["Project-Falcon".into()]));
    assert_eq!(
        redact("deploy project-falcon now"),
        format!("deploy {} now", b(14))
    );

    // The mask never reveals the hidden length.
    setup(|_| {});
    assert_eq!(
        redact("PASSWORD=a"),
        redact("PASSWORD=a-much-longer-secret")
    );
    assert_eq!(redact("1.2.3.4"), redact("192.168.100.200"));
    assert_eq!(port(22), port(65535));

    configure(&Settings::default());
}
