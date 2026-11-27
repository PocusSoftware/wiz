//! The right-click menu of a files pane, and filtering the folder by name.

use gpui::{AnyElement, ClipboardItem, Pixels, Point, anchored, deferred};

use super::*;
use crate::widgets::input::InputEvent;
use crate::widgets::{menu_panel, menu_row};

const MENU_W: f32 = 210.;

impl SftpView {
    pub(super) fn open_context(
        &mut self,
        side: Side,
        row: Option<usize>,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active = side;
        window.focus(&self.pane(side).focus, cx);
        if let Some(ix) = row {
            let p = self.pane(side);
            let kept = p
                .entries
                .get(ix)
                .is_some_and(|e| p.selected.contains(&e.name));
            if !kept {
                self.select_row(side, ix, false, false);
            }
        }
        self.menu = Some(Menu::Context {
            side,
            at,
            on_file: row.is_some(),
        });
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Some(Menu::Context { side, at, on_file }) = self.menu else {
            return None;
        };
        let p = self.pane(side);
        let item = |id: &'static str,
                    icon_name: &str,
                    text: &str,
                    note: SharedString,
                    run: fn(&mut Self, Side, &mut Window, &mut Context<Self>)| {
            menu_row(id, icon_name, text.to_string(), note).on_click(cx.listener(
                move |t, _, w, cx| {
                    t.menu = None;
                    run(t, side, w, cx);
                    cx.notify();
                },
            ))
        };
        let mut items: Vec<AnyElement> = Vec::new();
        if on_file {
            let single = p.selected.len() == 1;
            let to = match side {
                Side::Left => "arrow-right",
                Side::Right => "arrow-left",
            };
            if single {
                items.push(
                    item(
                        "ctx-open",
                        "external-link",
                        "Open",
                        "↵".into(),
                        |t, s, _, cx| {
                            let ix = t.pane(s).cursor;
                            t.open_dir(s, ix, cx)
                        },
                    )
                    .into_any_element(),
                );
            }
            items.push(
                item(
                    "ctx-copy",
                    to,
                    "Copy to the other side",
                    "F5".into(),
                    |t, s, _, cx| t.begin_transfer(s, cx),
                )
                .into_any_element(),
            );
            if single {
                items.push(
                    item(
                        "ctx-rename",
                        "pencil",
                        "Rename",
                        "F2".into(),
                        Self::ask_rename,
                    )
                    .into_any_element(),
                );
                items.push(
                    item("ctx-path", "copy", "Copy path", "".into(), |t, s, _, cx| {
                        let p = t.pane(s);
                        if let Some(name) = p.selected.iter().next() {
                            let path = p.source.join(&p.path, name);
                            cx.write_to_clipboard(ClipboardItem::new_string(path));
                        }
                    })
                    .into_any_element(),
                );
            }
            items.push(separator());
            items.push(
                item(
                    "ctx-delete",
                    "trash",
                    "Delete",
                    "Delete".into(),
                    Self::ask_delete,
                )
                .text_color(c(danger()))
                .into_any_element(),
            );
        } else {
            items.push(
                item(
                    "ctx-new",
                    "folder-plus",
                    "New folder",
                    crate::platform::view_shortcut(true, "N").into(),
                    Self::ask_new_folder,
                )
                .into_any_element(),
            );
            items.push(
                item(
                    "ctx-filter",
                    "search",
                    "Filter this folder",
                    crate::platform::view_shortcut(false, "F").into(),
                    Self::open_filter,
                )
                .into_any_element(),
            );
            items.push(
                item(
                    "ctx-refresh",
                    "refresh",
                    "Refresh",
                    crate::platform::view_shortcut(false, "R").into(),
                    |t, s, _, cx| t.refresh(s, cx),
                )
                .into_any_element(),
            );
            items.push(
                item(
                    "ctx-dots",
                    if p.hidden { "eye" } else { "eye-off" },
                    if p.hidden {
                        "Hide dotfiles"
                    } else {
                        "Show dotfiles"
                    },
                    "".into(),
                    |t, s, _, _| {
                        let p = t.pane_mut(s);
                        p.hidden = !p.hidden;
                        p.rebuild();
                    },
                )
                .into_any_element(),
            );
        }
        let menu = menu_panel(MENU_W)
            .id("sftp-context")
            .occlude()
            .on_mouse_down_out(cx.listener(|t, _, _, cx| {
                t.menu = None;
                cx.notify();
            }))
            .children(items);
        Some(
            deferred(
                anchored()
                    .position(at)
                    .snap_to_window_with_margin(px(4.))
                    .child(fade_in(menu, "sftp-context")),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    pub(super) fn open_filter(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let input = match &self.pane(side).filter {
            Some(input) => input.clone(),
            None => {
                let input = cx.new(|cx| TextInput::new("Filter this folder", false, cx));
                cx.subscribe(&input, move |t, input, _: &InputEvent, cx| {
                    let q = input.read(cx).value().to_lowercase();
                    let p = t.pane_mut(side);
                    p.query = q;
                    p.rebuild();
                    cx.notify();
                })
                .detach();
                self.pane_mut(side).filter = Some(input.clone());
                input
            }
        };
        window.focus(&input.focus_handle(cx), cx);
        cx.notify();
    }

    fn close_filter(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.pane_mut(side);
        p.filter = None;
        p.query.clear();
        p.rebuild();
        window.focus(&p.focus, cx);
        cx.notify();
    }

    pub(super) fn filter_bar(&self, side: Side, cx: &mut Context<Self>) -> Option<AnyElement> {
        let input = self.pane(side).filter.clone()?;
        Some(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(c(palette().border_variant))
                .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, w, cx| {
                    match ev.keystroke.key.as_str() {
                        "escape" => t.close_filter(side, w, cx),
                        "enter" | "down" => w.focus(&t.pane(side).focus, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                }))
                .child(icon("search", 13., palette().muted))
                .child(div().flex_1().min_w_0().child(input))
                .child(
                    icon_button(
                        (side.tag(), 1007usize),
                        "x",
                        "Clear filter (Esc)",
                        palette().muted,
                    )
                    .on_click(cx.listener(move |t, _, w, cx| t.close_filter(side, w, cx))),
                )
                .into_any_element(),
        )
    }
}

fn separator() -> AnyElement {
    div()
        .my_1()
        .h(px(1.))
        .bg(c(palette().border_variant))
        .into_any_element()
}
