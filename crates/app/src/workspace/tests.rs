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
    // More than four panes grow the grid.
    assert_eq!(
        layout(6, 1400., 800.),
        Some(vec![vec![0, 3], vec![1, 4], vec![2, 5]])
    );
    assert_eq!(grid_layout(5), vec![vec![0, 3], vec![1, 4], vec![2]]);
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
    let (root, cx) = cx.add_window_view(Workspace::new);
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
        jump: None,
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
fn snippets_need_a_terminal_in_front(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    cx.dispatch_action(Snippets);
    assert!(root.read_with(cx, |r, _| r.snippet_picker.is_none() && r.status.is_some()));
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
    // Dragging a pane's header onto another swaps the two; the dragged one keeps focus.
    let ids = |cx: &mut VisualTestContext| {
        root.read_with(cx, |r, _| {
            term(r)
                .unwrap()
                .panes
                .iter()
                .map(|p| p.view.entity_id())
                .collect::<Vec<_>>()
        })
    };
    let before = ids(cx);
    root.update_in(cx, |r, w, cx| r.swap_panes(r.active, 0, 3, w, cx));
    let after = ids(cx);
    assert_eq!((after[0], after[3]), (before[3], before[0]));
    assert_eq!(root.read_with(cx, |r, _| term(r).unwrap().focused), 3);
    // The pane shortcuts go round in pane order.
    cx.dispatch_action(NextPane);
    assert_eq!(root.read_with(cx, |r, _| term(r).unwrap().focused), 0);
    cx.dispatch_action(PrevPane);
    assert_eq!(root.read_with(cx, |r, _| term(r).unwrap().focused), 3);

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
        secrets::get(Kind::Password, &acct)
            .unwrap()
            .map(|s| s.reveal().to_string())
            .as_deref(),
        Some("hunter2")
    );

    // Deleting the host removes its secret from the file.
    root.update_in(cx, |r, w, cx| {
        r.confirm_delete = Some(0);
        r.answer_delete_host(true, w, cx);
    });
    assert!(secrets::get(Kind::Password, &acct).unwrap().is_none());
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
                .map(|s| s.reveal().to_string())
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
        assert!(
            secrets::keychain::get(Kind::Password, &tacct)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            secrets::get(Kind::Password, &tacct)
                .unwrap()
                .map(|s| s.reveal().to_string())
                .as_deref(),
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
    let creds = Credentials::Password(Secret::new("hunter2").unwrap());
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
            .map(|s| s.reveal().to_string())
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
    let creds = Credentials::Password(Secret::new("hunter2").unwrap());
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

/// Open a tab that needs no server: container logs (fetching them fails quietly here).
fn logs_tab(root: &Entity<Workspace>, cx: &mut VisualTestContext, name: &str) {
    root.update_in(cx, |r, w, cx| {
        let local = Arc::new(docker::Local::new().unwrap());
        let view = cx.new(|cx| LogView::new(Backend::Local(local), name.into(), cx));
        r.push_tab(name.into(), TabKind::Logs(view), w, cx);
    });
}

fn labels(root: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    root.read_with(cx, |r, _| {
        r.tabs.iter().map(|t| t.label.to_string()).collect()
    })
}

#[gpui::test]
fn settings_close_back_to_where_they_opened(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    let page = |cx: &mut VisualTestContext| root.read_with(cx, |r, _| r.page);
    // The settings shortcut and button toggle.
    cx.dispatch_action(OpenSettings);
    assert!(page(cx) == Page::Settings);
    cx.dispatch_action(OpenSettings);
    assert!(page(cx) == Page::Home);
    // Escape closes, back to the tab it was opened from.
    logs_tab(&root, cx, "a");
    logs_tab(&root, cx, "b");
    cx.dispatch_action(SelectTab(0));
    cx.dispatch_action(OpenSettings);
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    assert!(page(cx) == Page::Tab);
    assert_eq!(root.read_with(cx, |r, _| r.active), 0);
}

#[gpui::test]
fn select_tab_by_number_and_pin(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    for name in ["a", "b", "c"] {
        logs_tab(&root, cx, name);
    }
    let active =
        |cx: &mut VisualTestContext| root.read_with(cx, |r, _| r.tabs[r.active].label.to_string());
    cx.dispatch_action(SelectTab(1));
    assert_eq!(active(cx), "b");
    // Past the last tab: nothing changes.
    cx.dispatch_action(SelectTab(8));
    assert_eq!(active(cx), "b");

    // Pinned tabs move to the front in pin order; the selected tab stays selected.
    let id = |cx: &mut VisualTestContext, name: &str| {
        root.read_with(cx, |r, _| {
            r.tabs.iter().find(|t| t.label == name).unwrap().id
        })
    };
    let (a, c) = (id(cx, "a"), id(cx, "c"));
    root.update(cx, |r, cx| r.toggle_pin_tab(c, cx));
    assert_eq!(labels(&root, cx), ["c", "a", "b"]);
    root.update(cx, |r, cx| r.toggle_pin_tab(a, cx));
    assert_eq!(labels(&root, cx), ["c", "a", "b"]);
    assert_eq!(active(cx), "b");
    // Unpinning puts it right after the pinned ones.
    root.update(cx, |r, cx| r.toggle_pin_tab(c, cx));
    assert_eq!(labels(&root, cx), ["a", "c", "b"]);
    assert_eq!(active(cx), "b");

    // Dragging moves a tab, but never between pinned and unpinned ones.
    let b = id(cx, "b");
    root.update(cx, |r, cx| r.move_tab(b, 0, cx));
    assert_eq!(labels(&root, cx), ["a", "b", "c"]);
    root.update(cx, |r, cx| r.move_tab(b, 2, cx));
    assert_eq!(labels(&root, cx), ["a", "c", "b"]);
    root.update(cx, |r, cx| r.move_tab(a, 2, cx));
    assert_eq!(labels(&root, cx), ["a", "c", "b"]);
    assert_eq!(active(cx), "b");

    // Middle click closes a tab, but not a pinned one.
    let middle = |cx: &mut VisualTestContext, x: f32| {
        cx.simulate_event(gpui::MouseDownEvent {
            position: gpui::point(px(x), px(15.)),
            modifiers: gpui::Modifiers::default(),
            button: gpui::MouseButton::Middle,
            click_count: 1,
            first_mouse: false,
        });
    };
    // Tabs start after the hat and are 220 px wide here: "a" is first, then "c".
    middle(cx, 100.);
    assert_eq!(labels(&root, cx), ["a", "c", "b"]);
    middle(cx, 320.);
    assert_eq!(labels(&root, cx), ["a", "b"]);

    // The right-click menu closes with Escape.
    root.update_in(cx, |r, w, cx| {
        r.open_tab_menu(a, gpui::point(px(10.), px(10.)), w, cx)
    });
    cx.simulate_keystrokes("escape");
    assert!(root.read_with(cx, |r, _| r.tab_menu.is_none()));
}

#[gpui::test]
fn dialogs_block_clicks_behind_them(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    logs_tab(&root, cx, "a");
    cx.dispatch_action(QuickConnect);
    cx.run_until_parked();
    // A right click on the tab under the backdrop does not open its menu.
    cx.simulate_event(gpui::MouseDownEvent {
        position: gpui::point(px(100.), px(15.)),
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    assert!(root.read_with(cx, |r, _| r.tab_menu.is_none()));
    // A click inside quick connect keeps it, a click outside closes it.
    cx.simulate_click(gpui::point(px(700.), px(450.)), gpui::Modifiers::default());
    assert!(root.read_with(cx, |r, _| r.quick_form.is_some()));
    cx.simulate_click(gpui::point(px(20.), px(800.)), gpui::Modifiers::default());
    assert!(root.read_with(cx, |r, _| r.quick_form.is_none()));
    // So does the delete question, but not for a press on the window buttons above it
    // (only pressed here: the test window can't really zoom).
    root.update(cx, |r, cx| {
        r.cfg.hosts.push(test_host("deluser"));
        r.confirm_delete = Some(0);
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_event(gpui::MouseDownEvent {
        position: gpui::point(px(1339.), px(15.)),
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    assert!(root.read_with(cx, |r, _| r.confirm_delete.is_some()));
    cx.simulate_click(gpui::point(px(20.), px(800.)), gpui::Modifiers::default());
    assert!(root.read_with(cx, |r, _| r.confirm_delete.is_none()
        && r.cfg.hosts.len() == 1));
    // The master password dialog cancels the same way.
    root.update_in(cx, |r, w, cx| r.show_unlock(UnlockMode::for_file(), w, cx));
    cx.run_until_parked();
    cx.simulate_click(gpui::point(px(20.), px(800.)), gpui::Modifiers::default());
    assert!(root.read_with(cx, |r, _| r.unlock.is_none()));
}

#[gpui::test]
fn rename_tab_for_the_session(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    // Focus changes are only reported for the active window.
    cx.update(|w, _| w.activate_window());
    logs_tab(&root, cx, "a");
    let id = root.read_with(cx, |r, _| r.tabs[0].id);
    let name = |cx: &mut VisualTestContext| {
        root.read_with(cx, |r, _| r.tabs[0].name.as_ref().map(ToString::to_string))
    };
    let rename = |cx: &mut VisualTestContext| {
        root.update_in(cx, |r, w, cx| r.start_rename(id, w, cx));
        cx.run_until_parked();
    };
    // The old name is selected, so typing replaces it; Enter keeps it.
    rename(cx);
    type_text(cx, "web");
    cx.simulate_keystrokes("enter");
    assert_eq!(name(cx).as_deref(), Some("web"));
    assert!(root.read_with(cx, |r, _| r.renaming.is_none()));
    // Escape throws the edit away.
    rename(cx);
    type_text(cx, "db");
    cx.simulate_keystrokes("escape");
    assert_eq!(name(cx).as_deref(), Some("web"));
    // Clicking away (here the hat, for the hosts page) keeps it.
    rename(cx);
    type_text(cx, "db");
    cx.simulate_click(gpui::point(px(19.), px(15.)), gpui::Modifiers::default());
    assert_eq!(name(cx).as_deref(), Some("db"));
    // An empty name goes back to the default title.
    rename(cx);
    cx.simulate_keystrokes("backspace enter");
    assert_eq!(name(cx), None);
}

#[gpui::test]
fn one_click_presses_one_button(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    // The settings button ends 4 px before the window buttons (1400 - 121); this is the
    // edge it would share with the eye button without the gap.
    cx.simulate_click(gpui::point(px(1251.), px(15.)), gpui::Modifiers::default());
    assert!(root.read_with(cx, |r, _| r.page == Page::Settings));
    assert!(!crate::prefs::get().hide_sensitive());
}

/// Press and release a key: gpui clicks a focused control on the release.
fn press(cx: &mut VisualTestContext, key: &str) {
    cx.simulate_keystrokes(key);
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse(key).unwrap(),
    });
}

fn focused(cx: &mut VisualTestContext, handle: &FocusHandle) -> bool {
    cx.update(|w, _| handle.is_focused(w))
}

#[gpui::test]
fn tab_strip_by_keyboard(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    cx.update(|w, _| w.activate_window());
    for name in ["a", "b", "c"] {
        logs_tab(&root, cx, name);
    }
    cx.dispatch_action(SelectTab(0));
    let tab =
        |cx: &mut VisualTestContext, ix: usize| root.read_with(cx, |r, _| r.tabs[ix].focus.clone());
    let home = root.read_with(cx, |r, _| r.home_focus.clone());

    // The shortcut lands on the selected tab; the arrows move focus without selecting.
    cx.dispatch_action(FocusTabs);
    let a = tab(cx, 0);
    assert!(focused(cx, &a));
    cx.simulate_keystrokes("right");
    let b = tab(cx, 1);
    assert!(focused(cx, &b));
    assert_eq!(root.read_with(cx, |r, _| r.active), 0);
    // Enter selects, and focus goes to the tab's content.
    press(cx, "enter");
    assert_eq!(root.read_with(cx, |r, _| r.active), 1);
    assert!(!focused(cx, &b));

    cx.dispatch_action(FocusTabs);
    cx.simulate_keystrokes("home");
    assert!(focused(cx, &home));
    cx.simulate_keystrokes("end");
    let c = tab(cx, 2);
    assert!(focused(cx, &c));
    // Shift and an arrow move the tab, which keeps focus.
    cx.simulate_keystrokes("shift-left shift-left");
    assert_eq!(labels(&root, cx), ["c", "a", "b"]);
    assert!(focused(cx, &c));
    // Delete closes it.
    cx.simulate_keystrokes("delete");
    assert_eq!(labels(&root, cx), ["a", "b"]);

    // Shift+F10 opens the tab menu, Down enters it, Enter picks "Pin tab" (for "b", which
    // moves to the front).
    cx.dispatch_action(FocusTabs);
    cx.simulate_keystrokes("shift-f10");
    assert!(root.read_with(cx, |r, _| r.tab_menu.is_some()));
    cx.simulate_keystrokes("down");
    press(cx, "enter");
    assert!(root.read_with(cx, |r, _| r.tab_menu.is_none() && r.tabs[0].pinned));
    assert_eq!(labels(&root, cx), ["b", "a"]);

    // Keys the rename field inside a tab leaves alone are not for the tab either.
    let a = root.read_with(cx, |r, _| r.tabs[1].id);
    root.update_in(cx, |r, w, cx| r.start_rename(a, w, cx));
    cx.simulate_keystrokes("shift-f10");
    assert!(root.read_with(cx, |r, _| r.tab_menu.is_none() && r.renaming.is_some()));
}

#[gpui::test]
fn tab_moves_between_controls_and_stays_in_dialogs(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    cx.update(|w, _| w.activate_window());
    logs_tab(&root, cx, "a");
    let view = root.read_with(cx, |r, cx| match &r.tabs[0].kind {
        TabKind::Logs(v) => v.read(cx).focus_handle(cx),
        _ => unreachable!(),
    });
    // From the tab strip, Tab reaches the page content.
    cx.dispatch_action(FocusTabs);
    let mut steps = 0;
    while !focused(cx, &view) {
        cx.simulate_keystrokes("tab");
        steps += 1;
        assert!(steps < 10, "Tab never reached the page");
    }
    // A mouse click on a title bar button leaves focus there.
    cx.simulate_click(gpui::point(px(1238.), px(15.)), gpui::Modifiers::default());
    assert!(crate::prefs::get().hide_sensitive());
    assert!(focused(cx, &view));
    root.update(cx, Workspace::toggle_hide);

    // In quick connect, Tab goes field to field, then over the buttons, and never leaves.
    cx.dispatch_action(QuickConnect);
    cx.run_until_parked();
    let dialog = root.read_with(cx, |r, cx| r.quick_form.as_ref().unwrap().focus_handle(cx));
    // It opens on the target field.
    let target = cx.update(|w, cx| w.focused(cx)).unwrap();
    type_text(cx, "tester@127.0.0.1:2222");
    cx.simulate_keystrokes("tab");
    type_text(cx, "hunter2");
    // Then close, the two auth buttons, save as host, cancel, connect, and the target again.
    for _ in 0..7 {
        cx.simulate_keystrokes("tab");
        assert!(cx.update(|w, cx| dialog.contains_focused(w, cx)));
    }
    assert!(focused(cx, &target));
    // The arrow keys on a button move like Tab: from the close button to the auth buttons.
    cx.simulate_keystrokes("tab tab");
    let close = cx.update(|w, cx| w.focused(cx)).unwrap();
    cx.simulate_keystrokes("right");
    assert!(!focused(cx, &close) && cx.update(|w, cx| dialog.contains_focused(w, cx)));
    cx.simulate_keystrokes("up");
    assert!(focused(cx, &close));
    cx.simulate_keystrokes("shift-tab shift-tab");
    assert!(focused(cx, &target));
    // The tab bar shortcut does not reach behind the dialog.
    cx.dispatch_action(FocusTabs);
    assert!(focused(cx, &target));
    // Enter on Cancel cancels: the form's Enter (connect) does not run as well.
    cx.simulate_keystrokes("shift-tab shift-tab");
    press(cx, "enter");
    assert!(root.read_with(cx, |r, _| r.quick_form.is_none() && r.pending.is_empty()));
}

#[gpui::test]
fn host_key_question_takes_focus(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    cx.update(|w, _| w.activate_window());
    let modal = root.read_with(cx, |r, _| r.modal_focus.clone());
    let (reply, answer) = std::sync::mpsc::sync_channel(1);
    root.read_with(cx, |r, _| {
        r.trust_tx
            .send(TrustRequest {
                attempt: 0,
                host: "127.0.0.1".into(),
                port: 2222,
                fingerprint: "SHA256:x".into(),
                previous: None,
                reply,
            })
            .ok()
    });
    cx.run_until_parked();
    assert!(cx.update(|w, cx| modal.contains_focused(w, cx)));
    // Tab reaches its buttons and stays there; Escape says no.
    cx.simulate_keystrokes("tab");
    assert!(cx.update(|w, cx| modal.contains_focused(w, cx) && !modal.is_focused(w)));
    cx.simulate_keystrokes("escape");
    assert_eq!(answer.try_recv(), Ok(false));
    assert!(root.read_with(cx, |r, _| r.trust.is_empty()));
}

#[gpui::test]
fn rebound_shortcut_replaces_the_default(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    let default = crate::platform::default_shortcut("quick_connect").unwrap();
    crate::prefs::update(|s| {
        s.shortcuts = Some([("quick_connect".to_string(), "ctrl-alt-j".to_string())].into());
    })
    .unwrap();
    root.update(cx, Workspace::settings_changed);
    // The old keys do nothing any more, the new ones open the dialog.
    cx.simulate_keystrokes(default);
    assert!(root.read_with(cx, |r, _| r.quick_form.is_none()));
    cx.simulate_keystrokes("ctrl-alt-j");
    assert!(root.read_with(cx, |r, _| r.quick_form.is_some()));
}

/// Open a terminal tab with quick connect on the first SSH test server.
fn quick_shell(root: &Entity<Workspace>, cx: &mut VisualTestContext, tabs: usize) {
    cx.dispatch_action(QuickConnect);
    type_text(cx, "tester@127.0.0.1:2222");
    cx.simulate_keystrokes("tab");
    type_text(cx, "hunter2");
    cx.simulate_keystrokes("enter");
    wait(cx, root, "a quick connect shell", |r, _| {
        r.tabs.len() == tabs
    });
}

#[gpui::test]
fn tabs_merge_into_one_split_from_the_menu(cx: &mut TestAppContext) {
    let _g = setup();
    if !docker() {
        eprintln!("skipped (set SSHC_DOCKER_TESTS=1)");
        return;
    }
    let (root, cx) = open_root(cx);
    quick_shell(&root, cx, 1);
    quick_shell(&root, cx, 2);
    let (first, second) = root.read_with(cx, |r, _| (r.tabs[0].id, r.tabs[1].id));

    // The split menu opens from the keyboard and closes with Escape.
    cx.dispatch_action(SplitMenu);
    assert!(root.read_with(cx, |r, _| r.split_menu.is_some()));
    cx.simulate_keystrokes("escape");
    assert!(root.read_with(cx, |r, _| r.split_menu.is_none()));

    // Merging moves the second tab's pane into the first and closes the second.
    root.update_in(cx, |r, w, cx| r.merge_tab(second, first, w, cx));
    root.read_with(cx, |r, _| {
        assert_eq!(r.tabs.len(), 1);
        assert_eq!(r.tabs[0].id, first);
        assert!(matches!(&r.tabs[0].kind, TabKind::Terminal(t) if t.panes.len() == 2));
        assert_eq!(r.active, 0);
    });
    // Fill the tab up to four panes; a fifth is refused.
    for panes in 3..=4 {
        quick_shell(&root, cx, 2);
        let tab = root.read_with(cx, |r, _| r.tabs[1].id);
        root.update_in(cx, |r, w, cx| r.merge_tab(tab, first, w, cx));
        root.read_with(cx, |r, _| {
            assert!(matches!(&r.tabs[0].kind, TabKind::Terminal(t) if t.panes.len() == panes));
        });
    }
    quick_shell(&root, cx, 2);
    let tab = root.read_with(cx, |r, _| r.tabs[1].id);
    root.update_in(cx, |r, w, cx| r.merge_tab(tab, first, w, cx));
    assert!(status(&root, cx).contains("at most 4"));
    assert_eq!(root.read_with(cx, |r, _| r.tabs.len()), 2);
}

#[gpui::test]
fn login_question_is_asked_and_answered(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    cx.update(|w, _| w.activate_window());
    let modal = root.read_with(cx, |r, _| r.modal_focus.clone());
    let ask = |cx: &mut VisualTestContext| {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        root.read_with(cx, |r, _| {
            r.code_tx
                .send(CodeRequest {
                    attempt: 7,
                    host: "bastion".into(),
                    text: "Open your authenticator app.\u{1b}[31m".into(),
                    questions: vec![Question {
                        text: "Verification code: ".into(),
                        hidden: true,
                    }],
                    reply,
                })
                .ok()
        });
        cx.run_until_parked();
        answer
    };
    let answer = ask(cx);
    // It takes the keyboard like any dialog; the server's escape sequence is not shown.
    assert!(cx.update(|w, cx| modal.contains_focused(w, cx)));
    root.read_with(cx, |r, cx| {
        let shown = r.code.as_ref().unwrap().read(cx);
        assert!(!shown.text().contains('\u{1b}'));
    });
    type_text(cx, "123456");
    cx.simulate_keystrokes("enter");
    assert_eq!(answer.try_recv(), Ok(Some(vec!["123456".to_string()])));
    assert!(root.read_with(cx, |r, _| r.code.is_none() && r.codes.is_empty()));

    // Escape cancels the login.
    let answer = ask(cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(answer.try_recv(), Ok(None));

    // So does giving up on the connect it belongs to, whichever host is asking.
    let answer = ask(cx);
    root.update(cx, |r, _| r.dismiss_codes(7));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(None));
    assert!(root.read_with(cx, |r, _| r.code.is_none() && r.codes.is_empty()));
}

#[gpui::test]
fn deleting_a_jump_host_lets_its_users_connect_directly(cx: &mut TestAppContext) {
    let _g = setup();
    let (root, cx) = open_root(cx);
    let bastion = test_host("jumper");
    let inner = Host {
        name: "inner".into(),
        jump: Some(bastion.account()),
        ..test_host("inner")
    };
    root.update_in(cx, |r, w, cx| {
        r.cfg.hosts = vec![bastion, inner];
        r.confirm_delete = Some(0);
        r.answer_delete_host(true, w, cx);
    });
    root.read_with(cx, |r, _| {
        assert_eq!(r.cfg.hosts.len(), 1);
        assert_eq!(r.cfg.hosts[0].jump, None);
    });
    assert!(status(&root, cx).contains("inner now connects directly"));
}
