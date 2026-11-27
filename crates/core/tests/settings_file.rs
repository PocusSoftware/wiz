//! Saving and loading `settings.toml` through the config folder.

use wiz_core::config::Settings;

/// The settings page writes the accent whenever a text field is saved. A new install that
/// never turned colorful on must still load gray afterwards.
#[test]
fn saved_settings_keep_the_gray_default() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: set before anything reads the environment; this binary has a single test.
    unsafe { std::env::set_var("WIZ_CONFIG_DIR", dir.path()) };
    Settings {
        accent_color: Some("#f05a29".into()),
        ..Default::default()
    }
    .save()
    .unwrap();
    assert!(!Settings::load().colorful());
}
