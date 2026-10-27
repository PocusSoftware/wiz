//! Tests for `sftp`.

use super::*;

#[test]
fn pace_waits_to_hold_the_limit() {
    let p = Progress::new();
    assert_eq!(p.pace(1_000_000), std::time::Duration::ZERO);
    p.limit.store(100_000, Ordering::Relaxed);
    // 50 KB at 100 KB/s is due after about half a second.
    let d = p.pace(50_000);
    assert!(d > std::time::Duration::from_millis(400), "{d:?}");
    assert!(d <= std::time::Duration::from_millis(500), "{d:?}");
}

#[test]
fn path_helpers() {
    assert_eq!(join("/a", "b"), "/a/b");
    assert_eq!(join("/", "b"), "/b");
    assert_eq!(parent("/a/b"), "/a");
    assert_eq!(parent("/a"), "/");
    assert_eq!(parent("/"), "/");
    assert_eq!(parent("/a/b/"), "/a");
}

#[test]
fn unsafe_names_are_rejected() {
    for ok in ["a", "a.txt", ".env", "..a", "a b"] {
        assert!(is_safe_name(ok), "{ok}");
    }
    for bad in ["", ".", "..", "../x", "a/b", "/etc/passwd", "a\\b", "..\\x"] {
        assert!(!is_safe_name(bad), "{bad}");
    }
}

#[test]
fn dirs_sort_first_case_insensitive() {
    let f = |n: &str, d: bool| Entry {
        name: n.into(),
        is_dir: d,
        is_symlink: false,
        size: 0,
        modified: None,
    };
    let mut v = vec![
        f("b.txt", false),
        f("Z", true),
        f("a", true),
        f("A.txt", false),
    ];
    sort_entries(&mut v);
    let names: Vec<_> = v.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["a", "Z", "A.txt", "b.txt"]);
}
