//! The folder field of the host form: typed, or picked from the folders already in use.

use gpui::{
    AnchoredPositionMode, ClickEvent, Context, anchored, deferred, div, point, prelude::*, px,
};

use super::HostForm;
use crate::privacy;
use crate::theme::*;
use crate::widgets::{icon, icon_button, label, surface};

/// Folder names sorted and without twins that differ only in case.
pub(super) fn folder_names(mut names: Vec<String>) -> Vec<String> {
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by(|a, b| a.to_lowercase() == b.to_lowercase());
    names
}

impl HostForm {
    /// The folder input, with a button listing the existing folders when there are any.
    pub(super) fn folder_field(&self, cx: &Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label("Folder"))
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(div().flex_1().min_w_0().child(self.group.clone()))
                            .when(!self.groups.is_empty(), |d| {
                                d.child(
                                    icon_button(
                                        "folder-pick",
                                        "chevron-down",
                                        "Pick a folder",
                                        palette().muted,
                                    )
                                    .on_click(cx.listener(
                                        |t, _: &ClickEvent, _, cx| {
                                            t.folders_open = true;
                                            cx.notify();
                                        },
                                    )),
                                )
                            }),
                    )
                    .when(self.folders_open, |d| {
                        d.child(deferred(
                            anchored()
                                .position_mode(AnchoredPositionMode::Local)
                                .position(point(px(0.), px(28.)))
                                .snap_to_window_with_margin(px(8.))
                                .child(self.folder_list(cx)),
                        ))
                    }),
            )
    }

    /// Close the folder list, putting `pick` in the folder field.
    pub(super) fn close_folders(&mut self, pick: Option<&str>, cx: &mut Context<Self>) {
        if let Some(p) = pick {
            self.group.update(cx, |t, _| t.set_value(p));
        }
        self.folders_open = false;
        cx.notify();
    }

    fn folder_list(&self, cx: &Context<Self>) -> gpui::Div {
        let current = self.group.read(cx).value().trim().to_lowercase();
        div()
            .w(px(200.))
            .flex()
            .flex_col()
            .map(surface)
            .occlude()
            .on_mouse_down_out(cx.listener(|t, _, _, cx| t.close_folders(None, cx)))
            .child(
                div()
                    .id("folder-list")
                    .max_h(px(200.))
                    .overflow_y_scroll()
                    .py_1()
                    .children(self.groups.iter().enumerate().map(|(i, name)| {
                        let pick = name.clone();
                        div()
                            .id(("folder-row", i))
                            .px_2()
                            .py_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .when(name.to_lowercase() == current, |d| {
                                d.bg(c(palette().active))
                            })
                            .hover(|s| s.bg(c(palette().active)))
                            .child(icon("folder", 13., palette().muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(privacy::redact(name).into_owned()),
                            )
                            .on_click(cx.listener(move |t, _: &ClickEvent, _, cx| {
                                t.close_folders(Some(&pick), cx)
                            }))
                    })),
            )
    }
}
