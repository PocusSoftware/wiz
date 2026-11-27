//! Saving and loading `settings.toml` through the config folder.

use wiz_core::config::Settings;

/// The settings page writes the accent whenever a text field is saved. A new install
/// must still load the colorful default afterwards.
#[test]
fn saved_settings_keep_the_colorful_default() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: set before anything reads the environment; this binary has a single test.
    unsafe { std::env::set_var("WIZ_CONFIG_DIR", dir.path()) };
    Settings {
        accent_color: Some("#3a63ec".into()),
        ..Default::default()
    }
    .save()
    .unwrap();
    assert!(Settings::load().colorful());
}
