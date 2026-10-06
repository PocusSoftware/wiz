//! Tests for `sftp`.

use super::transfers::self_copy_error;

fn n(v: &[&str]) -> Vec<String> {
    v.iter().map(ToString::to_string).collect()
}

#[test]
fn refuses_same_folder_and_self_nesting() {
    assert!(self_copy_error(true, "/a/b", "/a/b", &n(&["f"])).is_some());
    assert!(self_copy_error(true, "/a/b/", "/a/b", &n(&["f"])).is_some());
    assert!(self_copy_error(true, "/a", "/a/dir", &n(&["dir"])).is_some());
    assert!(self_copy_error(true, "/a", "/a/dir/deeper", &n(&["dir"])).is_some());
}

#[test]
fn allows_normal_copies() {
    assert!(self_copy_error(true, "/a", "/b", &n(&["x"])).is_none());
    // Prefix of the name but a different folder.
    assert!(self_copy_error(true, "/a", "/a/dirty", &n(&["dir"])).is_none());
    // Different file systems can use identical paths.
    assert!(self_copy_error(false, "/a", "/a", &n(&["x"])).is_none());
}
