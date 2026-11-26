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

#[test]
fn remote_command_quotes_only_the_template() {
    assert_eq!(
        remote_command(&["ps", "-a", "--format", "{{json .}}"], false),
        "docker ps -a --format '{{json .}}'"
    );
    assert_eq!(
        remote_command(&["logs", "--tail", "5", "web"], true),
        "docker logs --tail 5 web 2>&1"
    );
}

#[test]
fn finds_docker_in_install_dirs() {
    let empty = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    std::fs::write(bin.path().join(DOCKER), "").unwrap();
    let dirs = [empty.path().to_path_buf(), bin.path().to_path_buf()];
    assert_eq!(find_bin(dirs, DOCKER), Some(bin.path().join(DOCKER)));
    assert_eq!(find_bin([empty.path().to_path_buf()], DOCKER), None);
}
