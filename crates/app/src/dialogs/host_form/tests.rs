//! Tests for the host form: what the fields turn into when left empty.

use gpui::TestAppContext;

use super::*;

#[gpui::test]
fn only_the_host_is_required(cx: &mut TestAppContext) {
    let form = cx.new(|cx| HostForm::new(None, Vec::new(), Vec::new(), cx));
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
}

#[gpui::test]
fn os_logo_kept_only_for_the_same_address(cx: &mut TestAppContext) {
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
        auth: AuthMethod::Agent,
    };
    let form = cx.new(|cx| HostForm::new(Some((0, old)), Vec::new(), Vec::new(), cx));
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
        auth: AuthMethod::Agent,
    };
    let form = cx.new(|cx| HostForm::new(Some((0, old)), Vec::new(), Vec::new(), cx));
    assert_eq!(form.update(cx, HostForm::try_save).unwrap().logo, None);
    form.update(cx, |f, _| f.logo = Some("debian"));
    let saved = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(saved.logo.as_deref(), Some("debian"));
}

#[gpui::test]
fn enter_in_logo_search_picks_the_first_match(cx: &mut TestAppContext) {
    let form = cx.new(|cx| HostForm::new(None, Vec::new(), Vec::new(), cx));
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
    let form = cx.new(|cx| HostForm::new(None, Vec::new(), Vec::new(), cx));
    form.update(cx, |f, cx| {
        f.mode = Mode::Agent;
        f.host.update(cx, |t, _| t.set_value("example.com"));
        f.tags.update(cx, |t, _| t.set_value(" db, Web ,,web, eu "));
    });
    let saved = form.update(cx, HostForm::try_save).unwrap();
    assert_eq!(saved.tags, ["db", "Web", "eu"]);
}
