//! The update notice in the tab bar and the dialog that installs a new release.

use gpui::{HighlightStyle, StyledText};

use super::*;
use crate::update::{NoteKind, NoteLine};
use crate::widgets::{close_button, dialog, dialog_header};

impl Workspace {
    pub(super) fn update_chip(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let r = crate::update::available()?;
        Some(
            keyboard(div().id("update-btn"))
                .h(px(22.))
                .px_2()
                .flex()
                .items_center()
                .gap_1()
                .rounded(px(3.))
                .bg(accent_alpha(0.18))
                .text_xs()
                .text_color(c(accent_text()))
                .cursor_pointer()
                .hover(|s| s.bg(accent_alpha(0.3)))
                .tooltip(crate::widgets::tip(format!(
                    "Wiz {} is available",
                    r.version
                )))
                .child(icon("arrow-down", 12., accent_text()))
                .child("Update")
                .on_click(cx.listener(|t, _, w, cx| t.open_update(w, cx))),
        )
    }

    fn open_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_some() {
            return;
        }
        self.overlay = Some(Overlay::Update {
            busy: false,
            error: None,
        });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn close_update(&mut self, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::Update { busy: false, .. })) {
            self.overlay = None;
            cx.notify();
        }
    }

    fn start_update(&mut self, cx: &mut Context<Self>) {
        let Some(release) = crate::update::available() else {
            return;
        };
        self.overlay = Some(Overlay::Update {
            busy: true,
            error: None,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { crate::update::install(&release) })
                .await;
            match r {
                Ok(()) => cx.update(|cx| cx.quit()),
                Err(e) => {
                    this.update(cx, |t, cx| {
                        t.overlay = Some(Overlay::Update {
                            busy: false,
                            error: Some(e.to_string().into()),
                        });
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    pub(super) fn update_view(
        &self,
        busy: bool,
        error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(r) = crate::update::available() else {
            return div().into_any_element();
        };
        let page = r.page.clone();
        let lines = crate::update::note_lines(&r.notes);
        let close = if busy {
            div().into_any_element()
        } else {
            close_button("update-close", "Later (Esc)")
                .on_click(cx.listener(|t, _, _, cx| t.close_update(cx)))
                .into_any_element()
        };
        div()
            .map(|d| dialog(d, 460.))
            .child(dialog_header(
                format!("Wiz {} is available", r.version),
                close,
            ))
            .child(
                div()
                    .id("update-notes")
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .p_2()
                    .rounded(px(3.))
                    .bg(c(palette().panel))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .when(lines.is_empty(), |d| d.child("No notes for this release."))
                    .children(lines.into_iter().map(note_line)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(c(palette().muted))
                    .child(format!(
                        "You have {}. Updating closes open sessions and restarts Wiz.",
                        ssh_core::update::CURRENT
                    )),
            )
            .when_some(error, |d, e| {
                d.child(div().text_xs().text_color(c(danger())).child(e))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        button("update-page", "Release page")
                            .on_click(move |_, _, cx| cx.open_url(&page)),
                    )
                    .child(div().flex_1())
                    .when(!busy, |d| {
                        d.child(
                            button("update-later", "Later")
                                .on_click(cx.listener(|t, _, _, cx| t.close_update(cx))),
                        )
                    })
                    .child(if busy {
                        button("update-go", "Updating...")
                    } else {
                        primary_button("update-go", "Update and restart")
                            .on_click(cx.listener(|t, _, _, cx| t.start_update(cx)))
                    }),
            )
            .into_any_element()
    }
}

fn note_line(line: NoteLine) -> Div {
    let mut cuts: Vec<usize> = line
        .bold
        .iter()
        .chain(&line.code)
        .flat_map(|r| [r.start, r.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let styles = cuts.windows(2).filter_map(|w| {
        let range = w[0]..w[1];
        let inside = |rs: &[std::ops::Range<usize>]| {
            rs.iter()
                .any(|r| r.start <= range.start && range.end <= r.end)
        };
        let (bold, code) = (inside(&line.bold), inside(&line.code));
        (bold || code).then(|| {
            (
                range,
                HighlightStyle {
                    font_weight: bold.then_some(FontWeight::SEMIBOLD),
                    color: code.then(|| c(accent_text())),
                    background_color: code.then(|| c(palette().hover)),
                    ..Default::default()
                },
            )
        })
    });
    let text = StyledText::new(line.text.clone()).with_highlights(styles);
    match line.kind {
        NoteKind::Heading => div()
            .mt_1()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .child(text),
        NoteKind::Bullet => div()
            .flex()
            .gap_2()
            .child(div().text_color(c(palette().muted)).child("•"))
            .child(div().flex_1().min_w_0().child(text)),
        NoteKind::Text => div().child(text),
    }
}
