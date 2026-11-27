//! Tests for the host form: what the fields turn into when left empty.

use std::sync::{MutexGuard, PoisonError};

use gpui::TestAppContext;

use super::*;

fn unlocked() -> (MutexGuard<'static, ()>, tempfile::TempDir) {
    let guard = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: tests touching the environment hold GLOBALS, nothing else reads it meanwhile.
    unsafe { std::env::set_var("WIZ_CONFIG_DIR", dir.path()) };
    secrets::unlock("masterpass1").unwrap();
    (guard, dir)
}

#[gpui::test]
fn only_the_host_is_required(cx: &mut TestAppContext) {
    let _g = unlocked();
    let form = cx.new(|cx| HostForm::new(Start::New, Vec::new(), Vec::new(), cx));
    let err = form.update(cx, |f, cx| f.try_save(cx).err());
    assert_eq!(err.as_deref(), Some("Host is required."));

    form.update(cx, |f, cx| {
        f.mode = Mode::Agent;
        f.host.update(cx, |t, _| t.set_value("example.com"))
    });
    let host = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(host.user, "root");
    assert_eq!(host.port, 22);
    assert_eq!(host.name, "root@example.com");
    assert_eq!(host.startup, None);
}

#[gpui::test]
fn os_logo_kept_only_for_the_same_address(cx: &mut TestAppContext) {
    let _g = unlocked();
    let old = Host {
        name: "web".into(),
        host: "example.com".into(),
        port: 22,
        user: "root".into(),
        group: None,
        pinned: false,
        jump: None,
        os: Some("ubuntu".into()),
        logo: None,
        tags: Vec::new(),
        startup: None,
        auth: AuthMethod::Agent,
    };
    let form = cx.new(|cx| HostForm::new(Start::Edit(0, old), Vec::new(), Vec::new(), cx));
    let kept = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(kept.os.as_deref(), Some("ubuntu"));

    form.update(cx, |f, cx| {
        f.host.update(cx, |t, _| t.set_value("other.example.com"))
    });
    let moved = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(moved.os, None);
}

#[gpui::test]
fn picked_logo_is_saved_and_unknown_ones_dropped(cx: &mut TestAppContext) {
    let _g = unlocked();
    let old = Host {
        name: "web".into(),
        host: "example.com".into(),
        port: 22,
        user: "root".into(),
        group: None,
        pinned: false,
        jump: None,
        os: None,
        logo: Some("not-a-logo".into()),
        tags: Vec::new(),
        startup: None,
        auth: AuthMethod::Agent,
    };
    let form = cx.new(|cx| HostForm::new(Start::Edit(0, old), Vec::new(), Vec::new(), cx));
    assert_eq!(form.update(cx, HostForm::try_save).unwrap().logo, None);
    form.update(cx, |f, _| f.logo = Some("debian"));
    let saved = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(saved.logo.as_deref(), Some("debian"));
}

#[gpui::test]
fn enter_in_logo_search_picks_the_first_match(cx: &mut TestAppContext) {
    let form = cx.new(|cx| HostForm::new(Start::New, Vec::new(), Vec::new(), cx));
    form.update(cx, |f, cx| {
        assert_eq!(f.first_logo(cx), Some(None));
        f.logo_search.update(cx, |t, _| t.set_value("rhel"));
        assert_eq!(f.first_logo(cx), Some(Some("redhat")));
        f.logo_search.update(cx, |t, _| t.set_value("zzz"));
        assert_eq!(f.first_logo(cx), None);
    });
}

#[gpui::test]
fn tags_are_split_trimmed_and_deduplicated(cx: &mut TestAppContext) {
    let _g = unlocked();
    let form = cx.new(|cx| HostForm::new(Start::New, Vec::new(), Vec::new(), cx));
    form.update(cx, |f, cx| {
        f.mode = Mode::Agent;
        f.host.update(cx, |t, _| t.set_value("example.com"));
        f.tags.update(cx, |t, _| t.set_value(" db, Web ,,web, eu "));
    });
    let saved = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(saved.tags, ["db", "Web", "eu"]);
}

#[test]
fn existing_folders_are_listed_once() {
    let names = vec!["prod".into(), "Lab".into(), "Prod".into(), "lab".into()];
    assert_eq!(folder::folder_names(names), vec!["Lab", "prod"]);
}

#[gpui::test]
fn picked_folder_is_saved(cx: &mut TestAppContext) {
    let _g = unlocked();
    let form = cx.new(|cx| HostForm::new(Start::New, Vec::new(), vec!["Prod".into()], cx));
    form.update(cx, |f, cx| {
        f.mode = Mode::Agent;
        f.host.update(cx, |t, _| t.set_value("example.com"));
        f.folders_open = true;
        f.close_folders(Some("Prod"), cx);
        assert!(!f.folders_open);
    });
    let host = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(host.group.as_deref(), Some("Prod"));
    // A typed name joins the folder that differs only in case.
    form.update(cx, |f, cx| f.group.update(cx, |t, _| t.set_value("prod")));
    let host = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(host.group.as_deref(), Some("Prod"));
}

#[gpui::test]
fn copy_starts_as_a_new_host_with_the_same_settings(cx: &mut TestAppContext) {
    let _g = unlocked();
    let src = Host {
        name: "web".into(),
        host: "example.com".into(),
        port: 2222,
        user: "deploy".into(),
        group: Some("Prod".into()),
        pinned: true,
        jump: None,
        os: Some("ubuntu".into()),
        logo: Some("debian".into()),
        tags: vec!["eu".into()],
        startup: Some("tmux attach".into()),
        auth: AuthMethod::Agent,
    };
    let form = cx.new(|cx| {
        HostForm::new(
            Start::Copy(src.clone()),
            vec![src.clone()],
            vec!["Prod".into()],
            cx,
        )
    });
    let err = form.update(cx, |f, cx| f.try_save(cx).err());
    assert_eq!(
        err.as_deref(),
        Some("Another saved host already uses this user, host and port.")
    );
    form.update(cx, |f, cx| {
        f.host.update(cx, |t, _| t.set_value("other.example.com"))
    });
    let copy = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(form.read_with(cx, |f, _| f.index), None);
    assert_eq!(copy.name, "web copy");
    assert_eq!(copy.port, 2222);
    assert_eq!(copy.user, "deploy");
    assert_eq!(copy.group.as_deref(), Some("Prod"));
    assert_eq!(copy.tags, ["eu"]);
    assert_eq!(copy.startup.as_deref(), Some("tmux attach"));
    assert_eq!(copy.logo.as_deref(), Some("debian"));
    assert_eq!(copy.auth, AuthMethod::Agent);
    assert!(!copy.pinned);
    assert_eq!(copy.os, None);
}
