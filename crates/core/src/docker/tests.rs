//! Tests for `docker`.

use super::*;

#[test]
fn parses_cli_json() {
    let l = r#"{"ID":"abc","Names":"web","Image":"nginx","State":"running","Status":"Up 1 hour","Ports":"0.0.0.0:80->80/tcp"}"#;
    let v = parse_cli_list(l).unwrap();
    assert_eq!(v[0].name, "web");
    assert!(v[0].is_running());
}

#[test]
fn rejects_shell_metachars() {
    assert!(check_ref("a; rm -rf /").is_err());
    assert!(check_ref("web-1_a.b").is_ok());
}
