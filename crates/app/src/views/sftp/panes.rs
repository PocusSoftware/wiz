//! Drawing a pane: the file rows, sort header, source picker and drag and drop.

use super::*;

/// Payload while dragging rows between panes.
#[derive(Clone)]
struct DragEntries {
    from: Side,
    names: Vec<String>,
}

struct DragLabel(SharedString);

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py(px(3.))
            .rounded(px(2.))
            .bg(c(accent()))
            .text_color(c(on_accent()))
            .child(self.0.clone())
    }
}

fn fmt_time(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => String::new(),
    }
}

impl SftpView {
    fn render_rows(
        &mut self,
        side: Side,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let p = self.pane(side);
        let active = self.active == side;
        range
            .filter_map(|ix| p.entries.get(ix).map(|e| (ix, e.clone())))
            .map(|(ix, e)| {
                let selected = p.selected.contains(&e.name);
                let is_cursor = active && p.cursor == ix;
                let drag_names = if selected {
                    p.selected_names()
                } else {
                    vec![e.name.clone()]
                };
                let drag_label: SharedString = if drag_names.len() == 1 {
                    drag_names[0].clone().into()
                } else {
                    format!("{} items", drag_names.len()).into()
                };
                div()
                    .id((side.tag(), ix))
                    .w_full()
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(selected, |d| d.bg(accent_alpha(0.4)))
                    .border_l_2()
                    .border_color(c(if is_cursor { accent() } else { 0x00000000 }))
                    .hover(|s| s.bg(c(HOVER)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |t, ev: &MouseDownEvent, window, cx| {
                            t.active = side;
                            t.menu = None;
                            window.focus(&t.pane(side).focus);
                            let m = ev.modifiers;
                            let already = t.pane(side).selected.iter().len() > 1
                                && t.pane(side)
                                    .entries
                                    .get(ix)
                                    .is_some_and(|e| t.pane(side).selected.contains(&e.name));
                            if !(already && !m.shift && !m.secondary()) {
                                t.select_row(side, ix, m.shift, m.secondary());
                            }
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |t, ev: &ClickEvent, _, cx| {
                        if ev.click_count() >= 2 {
                            t.open_dir(side, ix, cx);
                        } else if ev.modifiers() == Default::default()
                            && t.pane(side).selected.len() > 1
                        {
                            // Plain click inside a multi-selection collapses it (after drag could start).
                            t.select_row(side, ix, false, false);
                            cx.notify();
                        }
                    }))
                    .on_drag(
                        DragEntries {
                            from: side,
                            names: drag_names,
                        },
                        move |_, _, _, cx| cx.new(|_| DragLabel(drag_label.clone())),
                    )
                    .child(icon(
                        if e.is_dir { "folder" } else { "file" },
                        14.,
                        if e.is_dir { accent_text() } else { MUTED },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(div().whitespace_nowrap().child(e.name.clone())),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .text_color(c(PLACEHOLDER))
                            .child(if e.is_dir {
                                String::new()
                            } else {
                                human_size(e.size)
                            }),
                    )
                    .child(
                        div()
                            .w(px(104.))
                            .flex_none()
                            .text_color(c(PLACEHOLDER))
                            .whitespace_nowrap()
                            .child(fmt_time(e.modified)),
                    )
            })
            .collect()
    }

    fn sort_header(&self, side: Side, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pane(side);
        let cell = |id: &'static str, text: &'static str, key: SortKey| {
            let active = p.sort == key;
            keyboard(div().id((id, side as usize)))
                .focus(|s| s.bg(c(HOVER)))
                .flex()
                .items_center()
                .gap(px(2.))
                .cursor_pointer()
                .text_color(c(if active { TEXT } else { PLACEHOLDER }))
                .hover(|s| s.text_color(c(TEXT)))
                .on_click(cx.listener(move |t, _, _, cx| {
                    let p = t.pane_mut(side);
                    if p.sort == key {
                        p.asc = !p.asc;
                    } else {
                        p.sort = key;
                        p.asc = true;
                    }
                    p.rebuild();
                    cx.notify();
                }))
                .child(text)
                .when(active, |d| {
                    d.child(icon(
                        if p.asc { "chevron-up" } else { "chevron-down" },
                        10.,
                        TEXT,
                    ))
                })
        };
        div()
            .h(px(20.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(c(BORDER_VARIANT))
            .text_xs()
            .child(div().w(px(14.)).flex_none())
            .child(div().flex_1().child(cell("h-name", "Name", SortKey::Name)))
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(cell("h-size", "Size", SortKey::Size)),
            )
            .child(div().w(px(104.)).flex_none().child(cell(
                "h-mod",
                "Modified",
                SortKey::Modified,
            )))
    }

    pub(super) fn pane_view(&self, side: Side, cx: &mut Context<Self>) -> gpui::AnyElement {
        let p = self.pane(side);
        let active = self.active == side;
        let count = p.entries.len();
        let sel = p.selected.len();
        let can_back = !p.history.is_empty();

        let toolbar = div()
            .h(px(32.))
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(2.))
            .border_b_1()
            .border_color(c(BORDER_VARIANT))
            .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "enter" => {
                        t.submit_path(side, cx);
                        window.focus(&t.pane(side).focus);
                        cx.stop_propagation();
                    }
                    "escape" => {
                        let cur = t.pane(side).path.clone();
                        t.pane(side).input.update(cx, |i, _| i.set_value(&cur));
                        window.focus(&t.pane(side).focus);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .child(
                icon_button(
                    (side.tag(), 1000usize),
                    "arrow-left",
                    "Back",
                    if can_back { MUTED } else { ACTIVE },
                )
                .on_click(cx.listener(move |t, _, _, cx| t.back(side, cx))),
            )
            .child(
                icon_button(
                    (side.tag(), 1001usize),
                    "arrow-up",
                    format!(
                        "Parent folder ({})",
                        crate::platform::view_shortcut(false, "↑")
                    ),
                    MUTED,
                )
                .on_click(cx.listener(move |t, _, _, cx| t.up(side, cx))),
            )
            .child(
                icon_button((side.tag(), 1002usize), "home", "Home", MUTED)
                    .on_click(cx.listener(move |t, _, _, cx| t.go(side, "~".into(), true, cx))),
            )
            .child(div().flex_1().min_w_0().child(p.input.clone()))
            .child(
                icon_button(
                    (side.tag(), 1003usize),
                    "refresh",
                    format!("Refresh ({})", crate::platform::view_shortcut(false, "R")),
                    MUTED,
                )
                .on_click(cx.listener(move |t, _, _, cx| t.refresh(side, cx))),
            )
            .child(
                icon_button(
                    (side.tag(), 1004usize),
                    if p.hidden { "eye" } else { "eye-off" },
                    if p.hidden {
                        "Hide dotfiles"
                    } else {
                        "Show dotfiles"
                    },
                    MUTED,
                )
                .on_click(cx.listener(move |t, _, _, cx| {
                    let p = t.pane_mut(side);
                    p.hidden = !p.hidden;
                    p.rebuild();
                    cx.notify();
                })),
            )
            .child(
                icon_button(
                    (side.tag(), 1005usize),
                    "folder-plus",
                    format!("New folder ({})", crate::platform::view_shortcut(true, "N")),
                    MUTED,
                )
                .on_click(cx.listener(move |t, _, w, cx| t.ask_new_folder(side, w, cx))),
            );

        let list = uniform_list(
            (side.tag(), 2000usize),
            count,
            cx.processor(move |t: &mut SftpView, range, _, cx| t.render_rows(side, range, cx)),
        )
        .track_scroll(p.scroll.clone())
        .flex_1()
        .min_h_0();

        let body = if p.loading && p.entries.is_empty() {
            div()
                .flex_1()
                .p_3()
                .text_color(c(PLACEHOLDER))
                .child("Loading...")
                .into_any_element()
        } else if count == 0 {
            div()
                .flex_1()
                .p_3()
                .text_color(c(PLACEHOLDER))
                .child("Empty folder")
                .into_any_element()
        } else {
            list.into_any_element()
        };

        div()
            .id((side.tag(), 3000usize))
            .track_focus(&p.focus)
            .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, w, cx| t.on_key(side, ev, w, cx)))
            // Buttons keep focus from moving on a click; a click anywhere in the pane still
            // makes it the one the keys go to.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |t, _, w, _| w.focus(&t.pane(side).focus)),
            )
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_2()
            .border_color(c(if active { BORDER_VARIANT } else { 0x00000000 }))
            .drag_over::<DragEntries>(move |s, d, _, _| {
                if d.from != side {
                    s.border_color(c(accent())).bg(accent_alpha(0.08))
                } else {
                    s
                }
            })
            .on_drop(cx.listener(move |t, d: &DragEntries, _, cx| {
                if d.from != side {
                    t.begin_transfer_names(d.from, d.names.clone(), cx);
                }
            }))
            .child(
                div()
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .bg(c(PANEL))
                    .child(
                        keyboard(div().id((side.tag(), 4000usize)))
                            .focus(|s| s.bg(c(HOVER)))
                            .h_full()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .px(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(c(HOVER)))
                            .on_click(cx.listener(move |t, _, _, cx| {
                                t.menu = if t.menu == Some(side) {
                                    None
                                } else {
                                    Some(side)
                                };
                                cx.notify();
                            }))
                            .child(icon(
                                if matches!(p.source, Source::Local) {
                                    "folder"
                                } else {
                                    "server"
                                },
                                13.,
                                if active { accent_text() } else { MUTED },
                            ))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(p.source.label()),
                            )
                            .child(icon("chevron-down", 10., MUTED)),
                    )
                    .child(div().flex_1())
                    .when(p.loading, |d| {
                        d.child(div().text_color(c(PLACEHOLDER)).child("loading"))
                    })
                    .child(div().text_color(c(PLACEHOLDER)).child(if sel > 0 {
                        format!("{sel} of {count} selected")
                    } else if p.secret_hidden > 0 {
                        format!("{count} items · {} secret hidden", p.secret_hidden)
                    } else {
                        format!("{count} items")
                    })),
            )
            .child(toolbar)
            .when_some(p.error.clone(), |d, e| {
                d.child(
                    div()
                        .px_2()
                        .py(px(3.))
                        .text_color(c(RED))
                        .bg(c(0xe5534b1a))
                        .child(e),
                )
            })
            .child(self.sort_header(side, cx))
            .child(body)
            .children(self.source_menu(side, cx))
            .into_any_element()
    }

    fn source_menu(&self, side: Side, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.menu != Some(side) {
            return None;
        }
        let current = self.pane(side).source.label();
        let mut items: Vec<(Option<String>, String)> = vec![(None, "Local".into())];
        items.extend(
            self.hosts
                .iter()
                .cloned()
                .map(|(acct, name)| (Some(acct), name)),
        );
        let rows = items.into_iter().enumerate().map(|(n, (account, name))| {
            let is_cur = name == current;
            let is_local = account.is_none();
            keyboard(div().id((side.tag(), 5000 + n)))
                .focus(|s| s.bg(c(ACTIVE)))
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(|s| s.bg(c(ACTIVE)))
                .on_click(cx.listener(move |t, _, _, cx| {
                    t.menu = None;
                    cx.emit(SftpEvent::PickHost {
                        side,
                        account: account.clone(),
                    });
                    cx.notify();
                }))
                .child(icon(if is_local { "folder" } else { "server" }, 13., MUTED))
                .child(div().flex_1().child(name))
                .when(is_cur, |d| d.child(icon("check", 12., accent_text())))
        });
        Some(
            pop_in(
                div()
                    .absolute()
                    .occlude()
                    .top(px(22.))
                    .left_0()
                    .w(px(220.))
                    .bg(c(ELEVATED))
                    .border_2()
                    .border_color(c(BORDER))
                    .children(rows),
                (side.tag(), 6000usize),
            )
            .into_any_element(),
        )
    }
}
