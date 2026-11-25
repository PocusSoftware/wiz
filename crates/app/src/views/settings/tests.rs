//! Tests for the settings page.

use gpui::TestAppContext;

use super::*;

#[gpui::test]
fn opens_on_appearance_and_keeps_the_picked_page(cx: &mut TestAppContext) {
    let view = cx.new(SettingsView::new);
    view.read_with(cx, |v, _| assert_eq!(v.category, Category::Appearance));
    view.update(cx, |v, cx| v.show(Category::Privacy, cx));
    view.read_with(cx, |v, _| assert_eq!(v.category, Category::Privacy));
}

#[test]
fn every_page_has_its_own_name() {
    let mut names: Vec<_> = Category::ALL.iter().map(|c| c.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Category::ALL.len());
}

#[test]
fn picker_colors_round_trip() {
    use super::accent::{hsv_to_rgb, rgb_to_hsv};
    for rgb in [0xf05a29, 0x3e63dd, 0xffffff, 0x000000, 0x808080, 0x12a594] {
        assert_eq!(hsv_to_rgb(rgb_to_hsv(rgb)), rgb, "{rgb:06x}");
    }
    assert_eq!(hsv_to_rgb((0., 1., 1.)), 0xff0000);
    assert_eq!(hsv_to_rgb((120., 1., 1.)), 0x00ff00);
    assert_eq!(hsv_to_rgb((240., 1., 1.)), 0x0000ff);
}

#[gpui::test]
fn reset_restores_defaults_and_closes_the_prompt(cx: &mut TestAppContext) {
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: GLOBALS is held, no other test reads the environment meanwhile.
    unsafe { std::env::set_var("SSHC_CONFIG_DIR", dir.path()) };
    prefs::update(|s| s.max_splits = Some(8)).unwrap();
    let view = cx.new(SettingsView::new);
    view.update(cx, |v, cx| {
        v.confirm_reset = true;
        v.reset(cx);
        assert!(!v.confirm_reset);
    });
    assert_eq!(prefs::get().max_splits(), 4);
}

#[test]
fn every_shortcut_is_in_one_section_and_keys_split_into_caps() {
    use super::shortcuts::{GROUPS, keycaps};
    let mut grouped: Vec<_> = GROUPS.iter().flat_map(|g| g.1.iter().copied()).collect();
    let mut all: Vec<_> = crate::shortcuts::COMMANDS.iter().map(|c| c.0).collect();
    grouped.sort_unstable();
    all.sort_unstable();
    assert_eq!(grouped, all);

    assert_eq!(keycaps("⇧⌘D"), ["⇧", "⌘", "D"]);
    assert_eq!(keycaps("Ctrl+Shift+D"), ["Ctrl", "Shift", "D"]);
    assert_eq!(keycaps("Ctrl++"), ["Ctrl", "+"]);
}

#[test]
fn search_matches_are_found_case_insensitively() {
    let at = |name: &str, q: &str| shortcuts::match_range(name, q);
    assert_eq!(at("Next pane", "pane"), Some(5..9));
    assert_eq!(at("Next pane", "PANE".to_lowercase().as_str()), Some(5..9));
    assert_eq!(at("Next pane", ""), None);
    assert_eq!(at("Next pane", "tab"), None);
}

#[gpui::test]
fn offers_the_1password_agent_once_its_socket_exists(cx: &mut TestAppContext) {
    let Some(socket) = crate::platform::ONEPASSWORD_AGENT else {
        return;
    };
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let old_home = std::env::var_os("HOME");
    // SAFETY: GLOBALS is held, no other test reads the environment meanwhile.
    unsafe {
        std::env::set_var("SSHC_CONFIG_DIR", dir.path());
        std::env::set_var("HOME", dir.path());
    }
    prefs::update(|s| s.agent_socket = None).unwrap();

    let view = cx.new(SettingsView::new);
    view.read_with(cx, |v, _| assert_eq!(v.onepassword, None));

    let path = ssh_core::config::expand_home(socket);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "").unwrap();
    let view = cx.new(SettingsView::new);
    view.update(cx, |v, cx| {
        assert_eq!(v.onepassword, Some(socket));
        v.use_agent(socket, cx);
    });
    assert_eq!(prefs::get().agent_socket(), socket);

    // SAFETY: as above.
    unsafe {
        match old_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}
