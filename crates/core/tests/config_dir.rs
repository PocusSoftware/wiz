//! The default config folder, without `SSHC_CONFIG_DIR`, matches where earlier versions kept it.

use ssh_core::{APP_ID, config_dir};

#[test]
fn default_config_dir_matches_platform_convention() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // SAFETY: this is the only test in this binary, so nothing else reads the environment.
    unsafe {
        std::env::remove_var("SSHC_CONFIG_DIR");
        std::env::set_var("HOME", home);
    }
    #[cfg(target_os = "macos")]
    assert_eq!(
        config_dir().unwrap(),
        home.join("Library/Application Support").join(APP_ID)
    );
    #[cfg(windows)]
    {
        let appdata = home.join("AppData/Roaming");
        // SAFETY: as above.
        unsafe { std::env::set_var("APPDATA", &appdata) };
        assert_eq!(config_dir().unwrap(), appdata.join(APP_ID));
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        // SAFETY: as above.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "relative") };
        assert_eq!(config_dir().unwrap(), home.join(".config").join(APP_ID));
        let xdg = home.join("xdg");
        // SAFETY: as above.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &xdg) };
        assert_eq!(config_dir().unwrap(), xdg.join(APP_ID));
    }
}
