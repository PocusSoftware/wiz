//! The keyboard shortcuts panel, opened from the title bar: every shortcut of the app, the
//! terminal and the files view, as they are bound right now.

use crate::platform::{PASTE_KEYS, terminal_shortcut, view_shortcut};
use crate::shortcuts::{COMMANDS, GROUPS, keycaps};
use crate::widgets::keycap;

use super::*;

pub(super) struct Row {
    pub(super) name: &'static str,
    pub(super) keys: Vec<String>,
}

fn row(name: &'static str, keys: &[String]) -> Row {
    Row {
        name,
        keys: keys.to_vec(),
    }
}

fn add(rows: &mut Vec<Row>, name: &'static str, key: String) {
    match rows.iter_mut().find(|r| r.name == name) {
        Some(r) => r.keys.push(key),
        None => rows.push(row(name, &[key])),
    }
}

fn named(name: &'static str, keys: &[&str]) -> Row {
    Row {
        name,
        keys: keys.iter().map(ToString::to_string).collect(),
    }
}

pub(super) fn sections() -> Vec<(&'static str, Vec<Row>)> {
    let mut out: Vec<(&'static str, Vec<Row>)> = GROUPS
        .iter()
        .map(|&(group, ids)| {
            let rows = ids
                .iter()
                .filter_map(|id| {
                    let (_, name) = COMMANDS.iter().find(|c| c.0 == *id)?;
                    let keys = crate::shortcuts::keys(id)?;
                    Some(row(name, &[crate::platform::format_keys(&keys)]))
                })
                .collect();
            (group, rows)
        })
        .collect();
    if let Some((_, tabs)) = out.iter_mut().find(|(g, _)| *g == "Tabs") {
        add(tabs, "Next tab", crate::platform::format_keys("ctrl-tab"));
        add(
            tabs,
            "Previous tab",
            crate::platform::format_keys("ctrl-shift-tab"),
        );
        tabs.push(row("Tab 1 to 9", &[view_shortcut(false, "1-9")]));
    }
    if let Some((_, app)) = out.iter_mut().find(|(g, _)| *g == "App") {
        app.push(row(
            "Bigger, smaller, normal text",
            &[
                view_shortcut(false, "="),
                view_shortcut(false, "-"),
                view_shortcut(false, "0"),
            ],
        ));
        app.push(named("Close a dialog or menu", &["Esc"]));
        app.push(named("Quit", &[crate::platform::QUIT_KEYS]));
    }
    let mut paste = vec![terminal_shortcut(false, "V")];
    paste.extend(PASTE_KEYS.iter().map(ToString::to_string));
    out.push((
        "Terminal",
        vec![
            row("Copy", &[terminal_shortcut(false, "C")]),
            row("Paste", &paste),
            row("Select all", &[terminal_shortcut(false, "A")]),
            row("Find", &[terminal_shortcut(false, "F")]),
            row(
                "Next, previous match",
                &[terminal_shortcut(false, "G"), terminal_shortcut(true, "G")],
            ),
            row("Type the saved password", &[terminal_shortcut(true, "P")]),
            row("Open a link", &[view_shortcut(false, "Click")]),
        ],
    ));
    out.push((
        "Files",
        vec![
            named("Open folder or file", &["Enter"]),
            row(
                "Parent folder",
                &["Backspace".into(), view_shortcut(false, "↑")],
            ),
            row(
                "Back, forward",
                &[
                    view_shortcut(false, "←"),
                    view_shortcut(false, "→"),
                    "Mouse back".into(),
                    "Mouse forward".into(),
                ],
            ),
            named("Other side", &["←", "→"]),
            named("Jump to a name", &["Type it"]),
            row("Select all", &[view_shortcut(false, "A")]),
            row("Refresh", &[view_shortcut(false, "R")]),
            row("New folder", &[view_shortcut(true, "N")]),
            named("Rename", &["F2"]),
            named("Copy to the other side", &["F5"]),
            named("Delete", &["Delete"]),
        ],
    ));
    out
}

impl Workspace {
    pub(super) fn toggle_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::Help)) {
            self.close_help(window, cx);
        } else if !self.modal_open() {
            self.overlay = Some(Overlay::Help);
            window.focus(&self.modal_focus, cx);
            cx.notify();
        }
    }

    pub(super) fn close_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::Help)) {
            self.overlay = None;
        }
        if self.page == Page::Tab {
            self.select_tab(self.active, window, cx);
        } else {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(super) fn help_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let sections = sections().into_iter().map(|(group, rows)| {
            let rows = rows.into_iter().map(|r| {
                let mut keys = div().flex().flex_none().items_center().gap_1();
                for (n, label) in r.keys.iter().enumerate() {
                    if n > 0 {
                        keys = keys.child(
                            div()
                                .text_xs()
                                .text_color(c(palette().placeholder))
                                .child("or"),
                        );
                    }
                    keys = keys.children(keycaps(label).into_iter().map(|k| keycap(k, false)));
                }
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .py(px(3.))
                    .child(div().flex_1().min_w_0().child(r.name))
                    .child(keys)
            });
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .mt_3()
                        .mb_1()
                        .text_xs()
                        .text_color(c(palette().muted))
                        .child(group.to_uppercase()),
                )
                .children(rows)
        });
        div()
            .w(px(520.))
            .max_h(px(560.))
            .flex()
            .flex_col()
            .rounded(px(3.))
            .bg(c(palette().elevated))
            .border_1()
            .border_color(c(palette().border))
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .flex()
                    .items_center()
                    .child(div().flex_1().text_sm().child("Keyboard shortcuts"))
                    .child(
                        icon_button("help-close", "x", "Close (Esc)", palette().muted)
                            .on_click(cx.listener(|t, _, w, cx| t.close_help(w, cx))),
                    ),
            )
            .child(
                div()
                    .id("help-list")
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .pb_3()
                    .overflow_y_scroll()
                    .children(sections),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(c(palette().border_variant))
                    .text_xs()
                    .text_color(c(palette().placeholder))
                    .child("Change them under Shortcuts in Settings."),
            )
            .into_any_element()
    }
}
