//! The logo field of the host form: automatic (found when connecting) or picked from a list.

use gpui::{
    AnchoredPositionMode, ClickEvent, Context, anchored, deferred, div, point, prelude::*, px,
};
use ssh_core::os::{LOGOS, Logo, logo};

use super::HostForm;
use crate::theme::*;
use crate::widgets::{icon, keyboard, label};

impl HostForm {
    /// A button showing the current choice, opening the list of every logo.
    pub(super) fn logo_field(&self, cx: &Context<Self>) -> gpui::Div {
        let detected = self
            .old
            .as_ref()
            .and_then(|h| h.os.as_deref())
            .and_then(logo);
        let auto = match detected {
            Some(d) => (format!("os/{}", d.slug), format!("Automatic ({})", d.title)),
            None => ("server".into(), "Automatic".into()),
        };
        let (shown, text) = match self.logo.and_then(logo) {
            Some(l) => (format!("os/{}", l.slug), l.title.to_string()),
            None => auto.clone(),
        };
        div().flex().flex_col().gap_1().child(label("Logo")).child(
            div()
                .relative()
                .child(
                    keyboard(div().id("logo-pick"))
                        .h(px(26.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(2.))
                        .bg(c(ELEMENT))
                        .border_2()
                        .border_color(c(BORDER))
                        .cursor_pointer()
                        .child(icon(&shown, 14., accent_text()))
                        .child(div().flex_1().truncate().child(text))
                        .child(icon("chevron-down", 12., MUTED))
                        .on_click(cx.listener(|t, _: &ClickEvent, _, cx| {
                            t.logo_open = true;
                            cx.notify();
                        })),
                )
                .when(self.logo_open, |d| {
                    // Deferred, so it paints over the fields below; anchored, so it stays
                    // inside the window.
                    d.child(deferred(
                        anchored()
                            .position_mode(AnchoredPositionMode::Local)
                            .position(point(px(0.), px(28.)))
                            .snap_to_window_with_margin(px(8.))
                            .child(self.logo_list(auto, cx)),
                    ))
                }),
        )
    }

    /// Every logo by name, after `auto` (the icon and text of the automatic choice).
    fn logo_list(&self, auto: (String, String), cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let mut logos: Vec<&Logo> = LOGOS.iter().collect();
        logos.sort_by_key(|l| l.title.to_lowercase());
        let row = |ix: usize, shown: &str, text: &str, pick: Option<&'static str>| {
            div()
                .id(("logo-row", ix))
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .when(self.logo == pick, |d| d.bg(c(ACTIVE)))
                .hover(|s| s.bg(c(ACTIVE)))
                .child(icon(shown, 14., accent_text()))
                .child(text.to_string())
                .on_click(cx.listener(move |t, _: &ClickEvent, _, cx| {
                    t.logo = pick;
                    t.logo_open = false;
                    cx.notify();
                }))
        };
        div()
            .id("logo-list")
            .w(px(240.))
            .max_h(px(280.))
            .overflow_y_scroll()
            .py_1()
            .rounded(px(3.))
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|t, _, _, cx| {
                t.logo_open = false;
                cx.notify();
            }))
            .child(row(0, &auto.0, &auto.1, None))
            .children(
                logos
                    .into_iter()
                    .enumerate()
                    .map(|(i, l)| row(i + 1, &format!("os/{}", l.slug), l.title, Some(l.slug))),
            )
    }
}
