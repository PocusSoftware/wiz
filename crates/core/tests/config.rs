//! Saved hosts, settings and quick connect targets.

use ssh_core::config::{AuthMethod, Config, Host, Settings, WindowControls, parse_target};

#[test]
fn settings_defaults_and_roundtrip() {
    let d = Settings::default();
    assert!(!d.hide_sensitive() && d.mask_ipv6() && d.hide_secret_files() && d.animations());
    assert_eq!(
        (
            d.log_refresh_secs(),
            d.keepalive_secs(),
            d.rsync_hint_files()
        ),
        (2, 30, 500)
    );
    assert!(d.secret_file_patterns().contains(&".env".to_string()));
    assert_eq!(d.window_controls(), WindowControls::Right);
    assert_eq!(d.accent_color(), "#f05a29");

    let s = Settings {
        hide_sensitive: Some(true),
        mask_ipv6: Some(false),
        extra_mask: Some(vec!["falcon".into()]),
        terminal_font_size: Some(15.0),
        ..Default::default()
    };
    let text = toml::to_string_pretty(&s).unwrap();
    let back: Settings = toml::from_str(&text).unwrap();
    assert!(back.hide_sensitive() && !back.mask_ipv6());
    assert_eq!(back.extra_mask(), vec!["falcon".to_string()]);
    assert_eq!(back.terminal_font_size(), 15.0);
    // Window buttons are written as a plain word.
    let left: Settings = toml::from_str("window_controls = \"left\"").unwrap();
    assert_eq!(left.window_controls(), WindowControls::Left);
    // Unset fields keep their defaults.
    assert!(back.mask_ipv4() && back.mask_ports() && back.log_tail_lines() == 2000);
}

#[test]
fn roundtrip() {
    let cfg = Config {
        hosts: vec![
            Host {
                name: "a".into(),
                host: "h".into(),
                port: 2222,
                user: "u".into(),
                group: Some("prod".into()),
                pinned: true,
                auth: AuthMethod::Password,
            },
            Host {
                name: "b".into(),
                host: "h2".into(),
                port: 22,
                user: "u".into(),
                group: None,
                pinned: false,
                auth: AuthMethod::Key { path: "/k".into() },
            },
        ],
    };
    let s = toml::to_string_pretty(&cfg).unwrap();
    let back: Config = toml::from_str(&s).unwrap();
    assert_eq!(back.hosts, cfg.hosts);
    assert!(!s.contains("pinned = false"));
}

#[test]
fn old_files_without_group_or_pin_still_load() {
    let old =
        "[[host]]\nname = \"a\"\nhost = \"h\"\nuser = \"u\"\n\n[host.auth]\ntype = \"password\"\n";
    let cfg: Config = toml::from_str(old).unwrap();
    assert_eq!(cfg.hosts[0].group, None);
    assert!(!cfg.hosts[0].pinned);
    assert_eq!(cfg.hosts[0].port, 22);
}

#[test]
fn validate() {
    let h = |host: &str, user: &str, port: u16| Host {
        name: "n".into(),
        host: host.into(),
        port,
        user: user.into(),
        group: None,
        pinned: false,
        auth: AuthMethod::Password,
    };
    for ok in [
        "example.com",
        "my-server.example.com.",
        "localhost",
        "10.0.0.5",
        "::1",
        "2001:db8::1",
        "host_1",
    ] {
        assert!(h(ok, "root", 22).validate().is_ok(), "{ok}");
    }
    for bad in [
        "",
        "root@example.com",
        "example.com:22",
        "ssh://example.com",
        "exa mple.com",
        "-bad.com",
        "a..b",
        "1.2.3",
        "999.1.1.1",
        "example.com/x",
    ] {
        assert!(h(bad, "root", 22).validate().is_err(), "{bad}");
    }
    assert!(h("example.com", "", 22).validate().is_err());
    assert!(h("example.com", "ro ot", 22).validate().is_err());
    assert!(h("example.com", "a@b", 22).validate().is_err());
    assert!(h("example.com", "root", 0).validate().is_err());
    assert!(h("example.com", "deploy-user.1", 65535).validate().is_ok());
}

#[test]
fn parse_targets() {
    let p = |s: &str| parse_target(s, "me");
    assert_eq!(
        p("example.com"),
        Ok(("me".into(), "example.com".into(), 22))
    );
    assert_eq!(p("root@h:2222"), Ok(("root".into(), "h".into(), 2222)));
    assert_eq!(p("ssh -p 2200 a@h"), Ok(("a".into(), "h".into(), 2200)));
    assert_eq!(p("ssh a@h -p2200"), Ok(("a".into(), "h".into(), 2200)));
    assert_eq!(p("u@[::1]:22"), Ok(("u".into(), "::1".into(), 22)));
    assert_eq!(
        p("u@2001:db8::1"),
        Ok(("u".into(), "2001:db8::1".into(), 22))
    );
    assert_eq!(p("ssh://u@h:23"), Ok(("u".into(), "h".into(), 23)));
    assert!(p("").is_err());
    assert!(p("h:99999").is_err());
    assert!(p("a@h b@h").is_err());
    assert!(p("-i key h").is_err());
    assert!(p("h:22 -p 23").is_err());
}
