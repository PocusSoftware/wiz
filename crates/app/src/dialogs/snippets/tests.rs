use super::*;

fn snip(name: &str, command: &str) -> Snippet {
    Snippet {
        name: name.into(),
        command: command.into(),
    }
}

#[test]
fn filter_matches_name_or_command_ignoring_case() {
    let items = [
        snip("Disk usage", "df -h"),
        snip("Logs", "journalctl -u SSH"),
    ];
    assert_eq!(matching(&items, ""), vec![0, 1]);
    assert_eq!(matching(&items, "  DISK "), vec![0]);
    assert_eq!(matching(&items, "ssh"), vec![1]);
    assert!(matching(&items, "nope").is_empty());
}

#[gpui::test]
fn picker_pastes_the_chosen_command_without_enter(cx: &mut gpui::TestAppContext) {
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: tests touching the environment hold GLOBALS, nothing else reads it meanwhile.
    unsafe { std::env::set_var("SSHC_CONFIG_DIR", dir.path()) };
    let items = vec![snip("Disk usage", "df -h"), snip("Logs", "journalctl -f")];
    Snippets { items }
        .save(&Snippets::default_path().unwrap())
        .unwrap();

    let picker = cx.new(SnippetPicker::new);
    let pasted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = pasted.clone();
    cx.update(|cx| {
        cx.subscribe(&picker, move |_, ev: &SnippetEvent, _| {
            if let SnippetEvent::Paste(c) = ev {
                log.borrow_mut().push(c.clone());
            }
        })
        .detach();
    });
    picker.update(cx, |p, cx| {
        p.step(true, cx);
        p.paste(p.selected, cx);
    });
    assert_eq!(*pasted.borrow(), vec!["journalctl -f".to_string()]);
}

#[gpui::test]
fn picker_saves_a_new_snippet(cx: &mut gpui::TestAppContext) {
    let _g = crate::testing::GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: tests touching the environment hold GLOBALS, nothing else reads it meanwhile.
    unsafe { std::env::set_var("SSHC_CONFIG_DIR", dir.path()) };

    let (picker, cx) = cx.add_window_view(|_, cx| SnippetPicker::new(cx));
    picker.update_in(cx, |p, window, cx| {
        p.start_adding(window, cx);
        p.name.update(cx, |i, _| i.set_value("Disk usage"));
        p.command.update(cx, |i, _| i.set_value("df -h"));
        p.save_new(window, cx);
        assert!(!p.adding);
        assert_eq!(p.items, vec![snip("Disk usage", "df -h")]);
    });
    let saved = Snippets::load(&Snippets::default_path().unwrap()).unwrap();
    assert_eq!(saved.items, vec![snip("Disk usage", "df -h")]);
}
