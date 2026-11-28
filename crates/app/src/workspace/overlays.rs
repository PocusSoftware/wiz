//! Modal dialogs over the window: host key trust, host form, quick connect, master password,
//! and the close, delete and quit questions. A click outside closes all but host key trust,
//! which needs an answer.

use gpui::{MouseButton, MouseDownEvent};
use wiz_core::config::WindowControls;

use crate::dialogs::snippets::SnippetEvent;

use super::*;

/// What a click outside a dialog does.
type Dismiss = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

impl Workspace {
    /// Answer the host key question in front, then give focus back once none is left.
    pub(super) fn answer_trust(&mut self, yes: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self.trust.pop_front() {
            let _ = r.reply.send(yes);
        }
        if self.modal_open() {
            cx.notify();
        } else if self.page == Page::Tab {
            self.select_tab(self.active, window, cx);
        } else {
            self.show_page(self.page, window, cx);
        }
    }

    /// The open dialogs, bottom to top.
    pub(super) fn overlays(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let controls_left = crate::prefs::get().window_controls() == WindowControls::Left;
        // `dismiss` runs on a click outside the dialog, for dialogs that may close that way.
        let overlay = |id: &'static str, child: AnyElement, dismiss: Option<Dismiss>| {
            fade_in(
                div()
                    .absolute()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::hsla(0., 0., 0., 0.6))
                    // Clicks, hovers and scrolls stop here, not on the window behind.
                    .occlude()
                    .when_some(dismiss, |d, f| d.on_mouse_down(MouseButton::Left, f))
                    // The window still moves by its top edge. The strip leaves out the window
                    // buttons: Windows' hit test would take them for title bar otherwise.
                    .child(crate::platform::title_bar(
                        div()
                            .id("modal-drag")
                            .absolute()
                            .top_0()
                            .left(px(if controls_left { CONTROLS_W } else { 0. }))
                            .right(px(if controls_left { 0. } else { CONTROLS_W }))
                            .h(px(TITLEBAR_H))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                    ))
                    .child(pop_in(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(child),
                        (id, 1usize),
                    )),
                (id, 0usize),
            )
        };
        let trust_dialog =
            self.trust.front().map(|req| {
                overlay(
                    "trust",
                    div()
                        .w(px(380.))
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded(px(3.))
                        .bg(c(palette().elevated))
                        .border_1()
                        .border_color(c(palette().border))
                        .child(
                            div().text_sm().child(
                                // Name the saved host: with hiding on, addresses alone all look alike.
                                match (
                                    self.pending
                                        .iter()
                                        .find(|p| p.host == req.host && p.port == req.port),
                                    req.previous.is_some(),
                                ) {
                                    (Some(p), false) => {
                                        format!("Trust {}?", privacy::redact(&p.name))
                                    }
                                    (Some(p), true) => {
                                        format!("Host key of {} changed", privacy::redact(&p.name))
                                    }
                                    (None, false) => "Trust this host?".to_string(),
                                    (None, true) => "Host key changed".to_string(),
                                },
                            ),
                        )
                        .child(
                            div().child(
                                privacy::redact(&format!(
                                    "{}:{} {}",
                                    req.host,
                                    privacy::port(req.port),
                                    if req.previous.is_some() {
                                        "presented a different key than before. This can be a reinstalled server, or someone intercepting the connection."
                                    } else {
                                        "has not been seen before."
                                    }
                                ))
                                .into_owned(),
                            ),
                        )
                        .children(req.previous.clone().map(|p| fingerprint("Previous fingerprint (SHA256)", p)))
                        .child(fingerprint(
                            if req.previous.is_some() { "New fingerprint (SHA256)" } else { "Fingerprint (SHA256)" },
                            req.fingerprint.clone(),
                        ))
                        .child({
                            let no = cx.listener(|t, _, w, cx| t.answer_trust(false, w, cx));
                            let yes = cx.listener(|t, _, w, cx| t.answer_trust(true, w, cx));
                            // A changed key is more likely an attack than a new one, so there
                            // the safe answer is the primary button.
                            let (no, yes) = if req.previous.is_some() {
                                (
                                    primary_button("trust-no", "Cancel").on_click(no),
                                    button("trust-yes", "Replace key and connect").on_click(yes),
                                )
                            } else {
                                (
                                    button("trust-no", "Cancel").on_click(no),
                                    primary_button("trust-yes", "Trust and connect").on_click(yes),
                                )
                            };
                            div().flex().justify_end().gap_2().child(no).child(yes)
                        })
                        .into_any_element(),
                    None,
                )
            });
        let top = self.overlay.as_ref().and_then(|o| match o {
            Overlay::Form(f) => {
                let cancel = f.clone();
                Some(overlay(
                    "form",
                    f.clone().into_any_element(),
                    Some(Box::new(cx.listener(move |_, _, _, cx| {
                        cancel.update(cx, |_, cx| cx.emit(FormEvent::Cancelled))
                    }))),
                ))
            }
            Overlay::Quick(f) => {
                let cancel = f.clone();
                Some(overlay(
                    "quick",
                    f.clone().into_any_element(),
                    Some(Box::new(cx.listener(move |_, _, _, cx| {
                        cancel.update(cx, |_, cx| cx.emit(QuickEvent::Cancelled))
                    }))),
                ))
            }
            Overlay::Snippets(f) => {
                let cancel = f.clone();
                Some(overlay(
                    "snippets",
                    f.clone().into_any_element(),
                    Some(Box::new(cx.listener(move |_, _, _, cx| {
                        cancel.update(cx, |_, cx| cx.emit(SnippetEvent::Cancelled))
                    }))),
                ))
            }
            Overlay::Unlock(f) => {
                let cancel = f.clone();
                Some(overlay(
                    "unlock",
                    f.clone().into_any_element(),
                    Some(Box::new(cx.listener(move |_, _, _, cx| {
                        cancel.update(cx, |_, cx| cx.emit(UnlockEvent::Cancelled))
                    }))),
                ))
            }
            &Overlay::ConfirmDelete(ix) => {
                let h = self.cfg.hosts.get(ix)?;
                Some(overlay(
                    "confirm-delete",
                    div()
                        .w(px(340.))
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded(px(3.))
                        .bg(c(palette().elevated))
                        .border_1()
                        .border_color(c(palette().border))
                        .child(
                            div()
                                .text_sm()
                                .child(format!("Delete {}?", privacy::redact(&h.name))),
                        )
                        .child(
                            div()
                                .text_color(c(palette().muted))
                                .child("This removes the host and its saved passwords."),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(button("del-no", "Cancel").on_click(
                                    cx.listener(|t, _, w, cx| t.answer_delete_host(false, w, cx)),
                                ))
                                .child(primary_button("del-yes", "Delete").on_click(
                                    cx.listener(|t, _, w, cx| t.answer_delete_host(true, w, cx)),
                                )),
                        )
                        .into_any_element(),
                    Some(Box::new(
                        cx.listener(|t, _, w, cx| t.answer_delete_host(false, w, cx)),
                    )),
                ))
            }
            &Overlay::ConfirmClose(id, pane, dont_ask) => {
                let tab = self.tabs.iter().find(|t| t.id == id)?;
                let what = match pane {
                    Some(_) => "this terminal pane",
                    None => "this tab",
                };
                Some(overlay(
                    "confirm",
                    div()
                        .w(px(340.))
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded(px(3.))
                        .bg(c(palette().elevated))
                        .border_1()
                        .border_color(c(palette().border))
                        .child(
                            div()
                                .text_sm()
                                .child(format!("Close {}?", privacy::redact(&tab.label))),
                        )
                        .child(
                            div()
                                .text_color(c(palette().muted))
                                .child(format!("Closing {what} ends its session.")),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    checkbox("close-dont-ask", dont_ask, "Don't ask again")
                                        .on_click(cx.listener(|t, _, _, cx| {
                                            if let Some(Overlay::ConfirmClose(_, _, d)) =
                                                &mut t.overlay
                                            {
                                                *d = !*d;
                                                cx.notify();
                                            }
                                        })),
                                )
                                .child(div().flex_1())
                                .child(button("close-no", "Cancel").on_click(
                                    cx.listener(|t, _, w, cx| t.answer_close(false, w, cx)),
                                ))
                                .child(primary_button("close-yes", "Close").on_click(
                                    cx.listener(|t, _, w, cx| t.answer_close(true, w, cx)),
                                )),
                        )
                        .into_any_element(),
                    Some(Box::new(
                        cx.listener(|t, _, w, cx| t.answer_close(false, w, cx)),
                    )),
                ))
            }
            Overlay::Help => Some(overlay(
                "help",
                self.help_view(cx),
                Some(Box::new(cx.listener(|t, _, w, cx| t.close_help(w, cx)))),
            )),
            Overlay::Update { busy, error } => Some(overlay(
                "update",
                self.update_view(*busy, error.clone(), cx),
                Some(Box::new(cx.listener(|t, _, _, cx| t.close_update(cx)))),
            )),
        });
        // No dismissing by a click outside: that would cancel the login by accident.
        let code = self
            .code
            .clone()
            .map(|d| overlay("code", d.into_any_element(), None));
        let quit_dialog = self.quit_notice.map(|n| {
            overlay(
                "quit-notice",
                div()
                    .w(px(340.))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded(px(3.))
                    .bg(c(palette().elevated))
                    .border_1()
                    .border_color(c(palette().border))
                    .child(div().text_sm().child("Can't quit yet"))
                    .child(div().text_color(c(palette().muted)).child(format!(
                        "{} open. Close {} first, or allow quitting in Settings.",
                        if n == 1 {
                            "1 session is".to_string()
                        } else {
                            format!("{n} sessions are")
                        },
                        if n == 1 { "it" } else { "them" },
                    )))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                button("quit-all", "Close all and quit")
                                    .on_click(cx.listener(|t, _, _, cx| t.close_all_and_quit(cx))),
                            )
                            .child(
                                primary_button("quit-ok", "OK").on_click(
                                    cx.listener(|t, _, w, cx| t.dismiss_quit_notice(w, cx)),
                                ),
                            ),
                    )
                    .into_any_element(),
                Some(Box::new(
                    cx.listener(|t, _, w, cx| t.dismiss_quit_notice(w, cx)),
                )),
            )
        });
        let mut open: Vec<AnyElement> = [
            trust_dialog.map(IntoElement::into_any_element),
            top.map(IntoElement::into_any_element),
            code.map(IntoElement::into_any_element),
            quit_dialog.map(IntoElement::into_any_element),
        ]
        .into_iter()
        .flatten()
        .collect();
        // Tab stays inside the top one (`move_focus`). An open menu is above them all
        // and takes the handle instead.
        if !self.menu_open()
            && let Some(top) = open.pop()
        {
            open.push(
                div()
                    .absolute()
                    .size_full()
                    .track_focus(&self.modal_focus)
                    .child(top)
                    .into_any_element(),
            );
        }
        open
    }
}

/// A labeled SHA256 fingerprint in the host key question.
fn fingerprint(label: &'static str, value: String) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(c(palette().muted)).child(label))
        .child(div().text_xs().font_family(mono()).child(value))
}
