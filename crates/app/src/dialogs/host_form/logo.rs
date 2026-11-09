//! The logo field of the host form: automatic (found when connecting) or picked from a list.

use gpui::{
    AnchoredPositionMode, App, ClickEvent, Context, Focusable, Window, anchored, deferred, div,
    point, prelude::*, px,
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
                        .on_click(cx.listener(|t, _: &ClickEvent, w, cx| t.open_logos(w, cx))),
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

    fn open_logos(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.logo_search.update(cx, |t, _| t.set_value(""));
        window.focus(&self.logo_search.focus_handle(cx), cx);
        self.logo_open = true;
        cx.notify();
    }

    /// Close the list and set the logo (`None` is automatic), keeping keyboard focus in
    /// the form.
    pub(super) fn close_logos(
        &mut self,
        pick: Option<Option<&'static str>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = pick {
            self.logo = p;
        }
        self.logo_open = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The logos matching the search, by name.
    fn logo_matches(&self, cx: &App) -> Vec<&'static Logo> {
        let query = self.logo_search.read(cx).value().trim().to_lowercase();
        let mut logos: Vec<&Logo> = LOGOS.iter().filter(|l| l.matches(&query)).collect();
        logos.sort_by_key(|l| l.title.to_lowercase());
        logos
    }

    /// What Enter in the search picks: the first match, or automatic when nothing is typed.
    pub(super) fn first_logo(&self, cx: &App) -> Option<Option<&'static str>> {
        if self.logo_search.read(cx).value().trim().is_empty() {
            return Some(None);
        }
        self.logo_matches(cx).first().map(|l| Some(l.slug))
    }

    /// A search field over every logo by name, the automatic choice (`auto`, its icon and
    /// text) first while nothing is typed.
    fn logo_list(&self, auto: (String, String), cx: &Context<Self>) -> gpui::Div {
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
                .on_click(
                    cx.listener(move |t, _: &ClickEvent, w, cx| t.close_logos(Some(pick), w, cx)),
                )
        };
        let searching = !self.logo_search.read(cx).value().trim().is_empty();
        let logos = self.logo_matches(cx);
        div()
            .w(px(240.))
            .flex()
            .flex_col()
            .rounded(px(3.))
            .bg(c(ELEVATED))
            .border_1()
            .border_color(c(BORDER))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|t, _, w, cx| t.close_logos(None, w, cx)))
            .child(div().p_1().child(self.logo_search.clone()))
            .child(
                div()
                    .id("logo-list")
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .pb_1()
                    .when(!searching, |d| d.child(row(0, &auto.0, &auto.1, None)))
                    .when(logos.is_empty(), |d| {
                        d.child(
                            div()
                                .px_2()
                                .py_1()
                                .text_color(c(MUTED))
                                .child("No logo matches."),
                        )
                    })
                    .children(logos.into_iter().enumerate().map(|(i, l)| {
                        row(i + 1, &format!("os/{}", l.slug), l.title, Some(l.slug))
                    })),
            )
    }
}
