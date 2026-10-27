//! Tests for the crate root.

use super::*;

#[test]
fn old_config_folder_moves_to_the_new_id() {
    let base = tempfile::tempdir().unwrap();
    let old = base.path().join(OLD_APP_ID);
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("hosts.toml"), "x").unwrap();
    let dir = app_dir(base.path());
    assert_eq!(dir, base.path().join(APP_ID));
    assert_eq!(
        std::fs::read_to_string(dir.join("hosts.toml")).unwrap(),
        "x"
    );
    assert!(!old.exists());

    // Once the new folder exists, a stray old one is left alone.
    std::fs::create_dir(&old).unwrap();
    assert_eq!(app_dir(base.path()), base.path().join(APP_ID));
    assert!(old.exists());
}

#[test]
fn fresh_install_uses_the_new_id() {
    let base = tempfile::tempdir().unwrap();
    assert_eq!(app_dir(base.path()), base.path().join(APP_ID));
}
