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
