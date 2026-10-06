//! Quick connect dialog: building the host and credentials from what was typed.

use gpui::TestAppContext;

use super::*;

fn fill(d: &mut QuickConnect, target: &str, key_path: &str, cx: &mut Context<QuickConnect>) {
    d.key_mode = true;
    d.target.update(cx, |t, _| t.set_value(target));
    d.key_path.update(cx, |t, _| t.set_value(key_path));
}

#[gpui::test]
fn key_auth_finds_default_key_or_reports_missing(cx: &mut TestAppContext) {
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().unwrap();
    let old_home = std::env::var_os("HOME");
    // SAFETY: GLOBALS is held, no other test reads the environment meanwhile.
    unsafe { std::env::set_var("HOME", home.path()) };
    let dialog = cx.new(QuickConnect::new);

    // No keys in ~/.ssh.
    let err = dialog.update(cx, |d, cx| {
        fill(d, "u@h:2200", "", cx);
        d.build(cx).err()
    });
    assert!(err.unwrap().contains("No key"));

    // id_ecdsa is the first default that exists.
    let ssh = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("id_ecdsa"), "k").unwrap();
    let (host, creds) = dialog
        .update(cx, |d, cx| {
            d.passphrase.update(cx, |t, _| t.set_value("pp"));
            d.build(cx)
        })
        .unwrap();
    assert_eq!((host.user.as_str(), host.port), ("u", 2200));
    assert_eq!(
        host.auth,
        AuthMethod::Key {
            path: ssh.join("id_ecdsa")
        }
    );
    assert!(matches!(creds, Credentials::Key { passphrase: Some(p), .. } if p == "pp"));

    // A typed path, with ~ expanded, must exist.
    let err = dialog.update(cx, |d, cx| {
        fill(d, "u@h", "~/.ssh/nope", cx);
        d.build(cx).err()
    });
    assert_eq!(err.as_deref(), Some("Private key file not found."));
    let ok = dialog.update(cx, |d, cx| {
        fill(d, "u@h", "~/.ssh/id_ecdsa", cx);
        d.build(cx).is_ok()
    });
    assert!(ok);

    // Password mode needs a password; invalid targets are refused.
    let err = dialog.update(cx, |d, cx| {
        d.key_mode = false;
        d.build(cx).err()
    });
    assert!(err.unwrap().contains("Enter a password"));
    let err = dialog.update(cx, |d, cx| {
        d.password.update(cx, |t, _| t.set_value("pw"));
        d.target.update(cx, |t, _| t.set_value("u@bad host"));
        d.build(cx).err()
    });
    assert!(err.is_some());

    // SAFETY: as above.
    unsafe {
        match old_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}
