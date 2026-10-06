//! Quick connect target parsing.

use ssh_core::config::parse_target;

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
