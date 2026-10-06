//! Split layout rules, and UI flows driven headless through gpui's test platform.

use std::sync::{MutexGuard, OnceLock};

use gpui::{TestAppContext, VisualTestContext, size};
use ssh_core::config::Settings;

use super::split::{grid_layout, layout};
use super::*;

#[test]
fn split_layout_fits_the_window() {
    // Big window: 2 side by side, then a 2x2 grid that keeps earlier panes in place.
    assert_eq!(layout(2, 1400., 800.), Some(vec![vec![0], vec![1]]));
    assert_eq!(layout(3, 1400., 800.), Some(vec![vec![0, 2], vec![1]]));
    assert_eq!(layout(4, 1400., 800.), Some(vec![vec![0, 2], vec![1, 3]]));
    // Wide and short: no room for rows, so columns.
    assert_eq!(
        layout(4, 1400., 250.),
        Some(vec![vec![0], vec![1], vec![2], vec![3]])
    );
    // Narrow and tall: stacked.
    assert_eq!(layout(2, 500., 800.), Some(vec![vec![0, 1]]));
    // Too small for more.
    assert_eq!(layout(2, 500., 250.), None);
    assert_eq!(layout(4, 900., 250.), None);
    assert_eq!(layout(1, 100., 100.), Some(vec![vec![0]]));
}

// ---------- UI flows, driven headless through gpui's test platform ----------
//
// The secret store, settings and config dir are process-wide, so these run one at a
// time against one temporary config dir. Steps that talk to the SSH test servers
// (docker/compose.yml) need SSHC_DOCKER_TESTS=1.

static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();

fn setup() -> MutexGuard<'static, ()> {
    let guard = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    // SAFETY: tests touching the environment hold GLOBALS, nothing else reads it meanwhile.
    unsafe { std::env::set_var("SSHC_CONFIG_DIR", dir.path()) };
    for f in ["hosts.toml", "secrets.toml", "settings.toml", "known_hosts"] {
        let _ = std::fs::remove_file(dir.path().join(f));
    }
    secrets::lock();
    crate::prefs::init(Settings::default());
    guard
}

fn docker() -> bool {
    std::env::var("SSHC_DOCKER_TESTS").is_ok()
}

fn open_root(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    let (root, cx) = cx.add_window_view(|_, cx| Workspace::new(cx));
    cx.simulate_resize(size(px(1400.), px(900.)));
    cx.update(|w, cx| w.focus(&root.focus_handle(cx)));
    cx.run_until_parked();
    (root, cx)
}

/// Type text into the focused input, one key at a time. No spaces or dashes.
fn type_text(cx: &mut VisualTestContext, text: &str) {
    let keys: Vec<String> = text.chars().map(|c| format!("{c}->{c}")).collect();
    cx.simulate_keystrokes(&keys.join(" "));
}

/// Pump the UI until `done` holds, answering host key prompts with "trust".
fn wait(
    cx: &mut VisualTestContext,
    root: &Entity<Workspace>,
    what: &str,
    done: impl Fn(&Workspace, &App) -> bool,
) {
    for _ in 0..400 {
        cx.run_until_parked();
        root.update(cx, |r, cx| {
            if let Some(t) = r.trust.pop_front() {
                let _ = t.reply.send(true);
                cx.notify();
            }
        });
        if root.read_with(cx, |r, cx| done(r, cx)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let status = root.read_with(cx, |r, _| r.status.clone());
    panic!("timed out waiting for {what} (status: {status:?})");
}

fn status(root: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    root.read_with(cx, |r, _| {
        r.status
            .as_ref()
            .map(|s| s.0.to_string())
            .unwrap_or_default()
    })
}

fn term(r: &Workspace) -> Option<&TermTab> {
    match r.tabs.get(r.active).map(|t| &t.kind) {
        Some(TabKind::Terminal(t)) => Some(t),
        _ => None,
    }
}

fn test_host(user: &str) -> Host {
    Host {
        name: "test".into(),
        host: "127.0.0.1".into(),
        port: 2222,
        user: user.into(),
        group: None,
        pinned: false,
        auth: AuthMethod::Password,
    }
}

#[gpui::test]
fn quick_connect_dialog_validates_input(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    cx.dispatch_action(QuickConnect);
    assert!(root.read_with(cx, |r, _| r.quick_form.is_some()));
    // Enter with nothing typed: the dialog stays and nothing connects.
    cx.simulate_keystrokes("enter");
    assert!(root.read_with(cx, |r, _| r.quick_form.is_some() && r.pending.is_empty()));
    // Target but no password.
    type_text(cx, "tester@127.0.0.1:2222");
    cx.simulate_keystrokes("enter");
    assert!(root.read_with(cx, |r, _| r.quick_form.is_some() && r.pending.is_empty()));
    // Escape closes it.
    cx.simulate_keystrokes("escape");
    assert!(root.read_with(cx, |r, _| r.quick_form.is_none()));
}

#[gpui::test]
fn quick_connect_opens_shell_then_split_grid(cx: &mut TestAppContext) {
    let _g = setup();
    if !docker() {
        eprintln!("skipped (set SSHC_DOCKER_TESTS=1)");
        return;
    }
    let (root, cx) = open_root(cx);
    cx.dispatch_action(QuickConnect);
    type_text(cx, "tester@127.0.0.1:2222");
    cx.simulate_keystrokes("tab");
    type_text(cx, "hunter2");
    cx.simulate_keystrokes("enter");
    assert!(root.read_with(cx, |r, _| r.quick_form.is_none()));
    wait(cx, &root, "the quick connect shell", |r, _| {
        term(r).is_some()
    });
    root.read_with(cx, |r, _| {
        // Kept in memory for reconnects, never saved.
        assert!(r.quick.contains_key("tester@127.0.0.1:2222"));
        assert!(r.cfg.hosts.is_empty());
    });
    assert!(!ssh_core::config_dir().unwrap().join("hosts.toml").exists());

    // Split three times: side by side, then a 2x2 grid.
    for n in 2..=4 {
        cx.dispatch_action(SplitPane);
        wait(cx, &root, "a split pane", |r, _| {
            term(r).is_some_and(|t| t.panes.len() == n)
        });
    }
    cx.run_until_parked();
    root.read_with(cx, |r, _| {
        let t = term(r).unwrap();
        assert_eq!(t.layout, vec![vec![0, 2], vec![1, 3]]);
        assert_eq!(t.cols.len(), 2);
    });
    // A fifth is refused.
    cx.dispatch_action(SplitPane);
    cx.run_until_parked();
    assert!(status(&root, cx).contains("at most 4"));

    // Drag the column divider right, and the right column's row divider up.
    root.update_in(cx, |r, w, cx| {
        r.pane_drag = Some(PaneDrag {
            divider: Divider::Col(0),
            last: px(700.),
        });
        r.drag_divider(gpui::point(px(840.), px(0.)), w, cx);
        r.pane_drag = Some(PaneDrag {
            divider: Divider::Row { col: 1, n: 0 },
            last: px(400.),
        });
        r.drag_divider(gpui::point(px(0.), px(300.)), w, cx);
        r.pane_drag = None;
        let t = term(r).unwrap();
        assert!((t.cols[0] - 0.6).abs() < 0.01, "{:?}", t.cols);
        assert!((t.cols.iter().sum::<f32>() - 1.).abs() < 1e-4);
        assert!(t.rows[1][0] < 0.45, "{:?}", t.rows);
        assert!((t.rows[1].iter().sum::<f32>() - 1.).abs() < 1e-4);
        assert_eq!(t.rows[0], vec![0.5, 0.5]);
    });

    // A short wide window puts the 4 side by side; a tiny one keeps all 4 in the grid.
    cx.simulate_resize(size(px(1400.), px(250.)));
    cx.run_until_parked();
    root.read_with(cx, |r, _| {
        assert_eq!(term(r).unwrap().layout.len(), 4);
    });
    cx.simulate_resize(size(px(600.), px(250.)));
    cx.run_until_parked();
    root.read_with(cx, |r, _| {
        let t = term(r).unwrap();
        assert_eq!(t.layout, grid_layout(4));
        assert_eq!(t.panes.len(), 4);
    });

    // Back to one pane in a small window: splitting is refused for size.
    for _ in 0..3 {
        root.update_in(cx, |r, w, cx| r.do_close(r.active, Some(1), w, cx));
    }
    cx.simulate_resize(size(px(500.), px(250.)));
    cx.run_until_parked();
    cx.dispatch_action(SplitPane);
    cx.run_until_parked();
    assert!(status(&root, cx).contains("too small"));
    root.read_with(cx, |r, _| assert_eq!(term(r).unwrap().panes.len(), 1));
}

#[gpui::test]
fn secrets_file_unlock_lock_change_and_switch(cx: &mut TestAppContext) {
    let _g = setup();
    // A saved host, with passwords kept in a (not yet created) secrets file.
    let host = test_host("tester");
    Config {
        hosts: vec![host.clone()],
    }
    .save(&Config::default_path().unwrap())
    .unwrap();
    crate::prefs::update(|s| s.secret_store = Some("file".into())).unwrap();
    assert_eq!(secrets::store(), Store::File);
    let (root, cx) = open_root(cx);

    // Opening the host first asks for a new master password.
    root.update_in(cx, |r, w, cx| r.open_host(0, Open::Shell, w, cx));
    assert!(root.read_with(cx, |r, _| r.unlock.is_some() && r.pending.is_empty()));
    // Too short, or the two fields differ: nothing is created and the dialog stays.
    for (first, second) in [("short", "short"), ("longenough1", "longenough2")] {
        type_text(cx, first);
        cx.simulate_keystrokes("tab");
        type_text(cx, second);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(root.read_with(cx, |r, _| r.unlock.is_some()));
        assert!(!secrets::file_exists());
        // Escape cancels the open too; open again for a fresh dialog.
        cx.simulate_keystrokes("escape");
        assert!(root.read_with(cx, |r, _| r.unlock.is_none() && r.pending.is_empty()));
        root.update_in(cx, |r, w, cx| r.open_host(0, Open::Shell, w, cx));
    }
    type_text(cx, "masterpass1");
    cx.simulate_keystrokes("tab");
    type_text(cx, "masterpass1");
    cx.simulate_keystrokes("enter");
    wait(cx, &root, "the unlock", |r, _| r.unlock.is_none());
    assert!(secrets::file_exists() && secrets::is_unlocked());
    // The open went ahead after unlocking: no password saved yet, so it fails cleanly.
    wait(cx, &root, "the missing password error", |r, _| {
        r.pending.is_empty() && r.status.is_some()
    });
    assert!(
        status(&root, cx).contains("no password"),
        "{}",
        status(&root, cx)
    );

    let acct = host.account();
    secrets::set(Kind::Password, &acct, "hunter2").unwrap();
    if docker() {
        root.update_in(cx, |r, w, cx| r.open_host(0, Open::Shell, w, cx));
        wait(cx, &root, "a shell from the secrets file", |r, _| {
            term(r).is_some()
        });
    }

    // Settings: lock, then unlock with a wrong and the right password.
    root.update_in(cx, Workspace::open_settings);
    let settings = root.read_with(cx, |r, _| r.settings.clone().unwrap());
    settings.update(cx, |_, cx| cx.emit(SettingsEvent::Lock));
    cx.run_until_parked();
    assert!(!secrets::is_unlocked());
    settings.update(cx, |_, cx| cx.emit(SettingsEvent::Unlock));
    cx.run_until_parked();
    type_text(cx, "wrongpass");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(root.read_with(cx, |r, _| r.unlock.is_some()));
    assert!(!secrets::is_unlocked());
    type_text(cx, "masterpass1");
    cx.simulate_keystrokes("enter");
    wait(cx, &root, "the unlock", |r, _| r.unlock.is_none());
    assert!(secrets::is_unlocked());

    // Locked, adding a host asks for the password before the form opens.
    secrets::lock();
    root.update_in(cx, |r, w, cx| r.open_form(None, w, cx));
    assert!(root.read_with(cx, |r, _| r.unlock.is_some() && r.form.is_none()));
    type_text(cx, "masterpass1");
    cx.simulate_keystrokes("enter");
    wait(cx, &root, "the form", |r, _| r.form.is_some());
    root.update_in(cx, |r, w, _| {
        r.form = None;
        w.focus(&r.focus);
    });

    // Change the master password.
    settings.update(cx, |_, cx| cx.emit(SettingsEvent::ChangeMaster));
    cx.run_until_parked();
    type_text(cx, "masterpass2");
    cx.simulate_keystrokes("tab");
    type_text(cx, "masterpass2");
    cx.simulate_keystrokes("enter");
    wait(cx, &root, "the change", |r, _| r.unlock.is_none());
    assert!(status(&root, cx).contains("changed"));
    secrets::lock();
    assert!(matches!(
        secrets::unlock("masterpass1"),
        Err(ssh_core::Error::WrongMasterPassword)
    ));
    secrets::unlock("masterpass2").unwrap();
    assert_eq!(
        secrets::get(Kind::Password, &acct).unwrap().as_deref(),
        Some("hunter2")
    );

    // Deleting the host removes its secret from the file.
    root.update_in(cx, |r, w, cx| {
        r.confirm_delete = Some(0);
        r.answer_delete_host(true, w, cx);
    });
    assert_eq!(secrets::get(Kind::Password, &acct).unwrap(), None);
    root.read_with(cx, |r, _| assert!(r.cfg.hosts.is_empty()));

    // Store switching touches the real Keychain (macOS only), like the core Keychain test.
    #[cfg(target_os = "macos")]
    if docker() {
        // Switch to the Keychain and back with a throwaway host: the secret moves both ways.
        let temp = test_host(&format!("switch{}", std::process::id()));
        let tacct = temp.account();
        root.update(cx, |r, _| r.cfg.hosts.push(temp));
        secrets::set(Kind::Password, &tacct, "moveme").unwrap();
        settings.update(cx, |_, cx| {
            cx.emit(SettingsEvent::SwitchStore(Store::Keychain))
        });
        cx.run_until_parked();
        assert_eq!(secrets::store(), Store::Keychain);
        assert_eq!(crate::prefs::get().secret_store(), "keychain");
        assert!(
            status(&root, cx).contains("Moved 1 secret."),
            "{}",
            status(&root, cx)
        );
        assert_eq!(
            secrets::keychain::get(Kind::Password, &tacct)
                .unwrap()
                .as_deref(),
            Some("moveme")
        );
        // Back to the file, locked first: it asks for the master password, then moves.
        secrets::lock();
        settings.update(cx, |_, cx| cx.emit(SettingsEvent::SwitchStore(Store::File)));
        cx.run_until_parked();
        assert!(root.read_with(cx, |r, _| r.unlock.is_some()));
        assert_eq!(secrets::store(), Store::Keychain);
        type_text(cx, "masterpass2");
        cx.simulate_keystrokes("enter");
        wait(cx, &root, "the switch", |r, _| r.unlock.is_none());
        assert_eq!(secrets::store(), Store::File);
        assert_eq!(
            secrets::keychain::get(Kind::Password, &tacct).unwrap(),
            None
        );
        assert_eq!(
            secrets::get(Kind::Password, &tacct).unwrap().as_deref(),
            Some("moveme")
        );
    }
}

#[gpui::test]
fn quick_connect_save_as_host_stores_secret(cx: &mut TestAppContext) {
    let _g = setup();
    crate::prefs::update(|s| s.secret_store = Some("file".into())).unwrap();
    secrets::unlock("masterpass1").unwrap();
    let (root, cx) = open_root(cx);
    let host = test_host("tester");
    let creds = Credentials::Password("hunter2".into());
    root.update_in(cx, |r, w, cx| {
        r.quick_connect(host.clone(), creds, true, w, cx)
    });
    cx.run_until_parked();
    root.read_with(cx, |r, _| {
        assert_eq!(r.cfg.hosts, vec![host.clone()]);
        assert!(r.quick.is_empty());
    });
    assert_eq!(
        secrets::get(Kind::Password, &host.account())
            .unwrap()
            .as_deref(),
        Some("hunter2")
    );
    let saved = Config::load(&Config::default_path().unwrap()).unwrap();
    assert_eq!(saved.hosts, vec![host.clone()]);
    if docker() {
        wait(cx, &root, "the saved host's shell", |r, _| {
            term(r).is_some()
        });
    }
    // Same target again with "save": not duplicated.
    let creds = Credentials::Password("hunter2".into());
    root.update_in(cx, |r, w, cx| {
        r.quick_connect(host.clone(), creds, true, w, cx)
    });
    cx.run_until_parked();
    root.read_with(cx, |r, _| assert_eq!(r.cfg.hosts.len(), 1));
}

#[gpui::test]
fn status_messages_hide_after_a_while(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    let shown = |root: &Entity<Workspace>, cx: &mut VisualTestContext| {
        cx.run_until_parked();
        root.read_with(cx, |r, _| r.status.as_ref().map(|s| s.0.to_string()))
    };

    root.update(cx, |r, cx| {
        r.set_status("info", false);
        cx.notify();
    });
    assert_eq!(shown(&root, cx).as_deref(), Some("info"));
    cx.executor()
        .advance_clock(Duration::from_secs(STATUS_SECS - 1));
    assert_eq!(shown(&root, cx).as_deref(), Some("info"));
    cx.executor().advance_clock(Duration::from_secs(1));
    assert_eq!(shown(&root, cx), None);

    // Errors stay longer, and a newer message is not cut short by an older timer.
    root.update(cx, |r, cx| {
        r.set_status("first", true);
        cx.notify();
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(Duration::from_secs(STATUS_ERROR_SECS - 2));
    root.update(cx, |r, cx| {
        r.set_status("second", true);
        cx.notify();
    });
    cx.executor().advance_clock(Duration::from_secs(5));
    assert_eq!(shown(&root, cx).as_deref(), Some("second"));
    cx.executor()
        .advance_clock(Duration::from_secs(STATUS_ERROR_SECS - 5));
    assert_eq!(shown(&root, cx), None);
}
