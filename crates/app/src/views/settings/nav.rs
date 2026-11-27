//! The list of settings pages on the left, with reset and the version below it.

use super::*;

impl SettingsView {
    pub(super) fn nav(&self, cx: &mut Context<Self>) -> gpui::Div {
        let items = Category::ALL.iter().enumerate().map(|(i, &cat)| {
            let current = cat == self.category;
            keyboard(div().id(("settings-nav", i)))
                .px_2()
                .py(px(5.))
                .rounded(px(4.))
                .border_1()
                .border_color(gpui::transparent_black())
                .cursor_pointer()
                .text_color(c(if current {
                    palette().text
                } else {
                    palette().muted
                }))
                .when(current, |d| d.bg(c(palette().active)))
                .when(!current, |d| d.hover(|s| s.bg(c(palette().hover))))
                .child(cat.name())
                .on_click(cx.listener(move |t, _, _, cx| t.show(cat, cx)))
        });
        div()
            .w(px(200.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap_0p5()
            .p_2()
            .pt_4()
            .border_r_1()
            .border_color(c(palette().border_variant))
            .children(items)
            .child(div().flex_1())
            .child(
                div()
                    .pt_2()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .border_t_1()
                    .border_color(c(palette().border_variant))
                    .when(!self.confirm_reset, |d| {
                        d.child(
                            action("reset", "rotate-ccw", "Reset to defaults", palette().muted)
                                .on_click(cx.listener(|t, _, _, cx| {
                                    t.confirm_reset = true;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.confirm_reset, |d| {
                        d.child(
                            div()
                                .px_2()
                                .py_1()
                                .text_xs()
                                .text_color(c(palette().muted))
                                .child("Reset every setting?"),
                        )
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .flex()
                                .gap_1()
                                .child(div().flex_1().child(button("reset-no", "Cancel").on_click(
                                    cx.listener(|t, _, _, cx| {
                                        t.confirm_reset = false;
                                        cx.notify();
                                    }),
                                )))
                                .child(
                                    div().flex_1().child(
                                        primary_button("reset-yes", "Reset")
                                            .on_click(cx.listener(|t, _, _, cx| t.reset(cx))),
                                    ),
                                ),
                        )
                    })
                    .child(
                        action("open-file", "file", "Open settings file", palette().muted)
                            .on_click(cx.listener(|t, _, _, cx| t.open_file(cx))),
                    )
                    .child(
                        match self.update_check.clone().or_else(|| {
                            crate::update::available().map(|r| UpdateCheck::Found(r.version))
                        }) {
                            Some(UpdateCheck::Checking) => action(
                                "check-update",
                                "refresh",
                                "Checking...",
                                palette().placeholder,
                            ),
                            Some(UpdateCheck::Found(v)) => action(
                                "check-update",
                                "download",
                                &format!("Update to {v}"),
                                accent_text(),
                            )
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenUpdate)),
                            ),
                            _ => action(
                                "check-update",
                                "refresh",
                                "Check for updates",
                                palette().muted,
                            )
                            .on_click(cx.listener(|t, _, _, cx| t.check_for_updates(cx))),
                        },
                    ),
            )
            .child(
                div()
                    .mt_3()
                    .px_2()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .text_xs()
                    .text_color(c(palette().placeholder))
                    .child(format!("Wiz {}", crate::version()))
                    .child("Changes save automatically"),
            )
    }

    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        self.update_check = Some(UpdateCheck::Checking);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async { wiz_core::update::newer_release() })
                .await;
            this.update(cx, |t, cx| t.finish_check(r, cx)).ok();
        })
        .detach();
    }

    /// Show what the update check found: a newer version in the sidebar, anything else in
    /// the status line.
    pub(super) fn finish_check(
        &mut self,
        r: wiz_core::Result<Option<wiz_core::update::Release>>,
        cx: &mut Context<Self>,
    ) {
        self.update_check = None;
        match r {
            Ok(Some(release)) => {
                self.update_check = Some(UpdateCheck::Found(release.version.clone()));
                crate::update::set_available(release, cx);
            }
            Ok(None) => cx.emit(SettingsEvent::Notify("Wiz is up to date.".into(), false)),
            Err(e) => cx.emit(SettingsEvent::Notify(
                format!("Could not check for updates: {e}"),
                true,
            )),
        }
        cx.notify();
    }

    /// Back to the defaults. The secret store stays: switching it means moving secrets.
    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        self.confirm_reset = false;
        self.apply(cx, |s| {
            *s = Settings {
                secret_store: s.secret_store.take(),
                ..Default::default()
            }
        });
        self.load_inputs(cx);
    }

    /// Open `settings.toml` in the editor chosen under Files.
    fn open_file(&mut self, cx: &mut Context<Self>) {
        let opened = wiz_core::config_dir().and_then(|d| {
            crate::platform::open_in_editor(&d.join("settings.toml"), &prefs::get().editor_app())
                .map_err(Into::into)
        });
        if let Err(e) = opened {
            self.error = Some(format!("Could not open settings file: {e}").into());
            cx.notify();
        }
    }

    /// Switch to another page.
    pub(super) fn show(&mut self, category: Category, cx: &mut Context<Self>) {
        self.category = category;
        cx.notify();
    }
}

/// A row below the pages, styled like them so it reads as part of the sidebar.
fn action(id: &'static str, name: &str, text: &str, color: u32) -> gpui::Stateful<gpui::Div> {
    keyboard(div().id(id))
        .px_2()
        .py(px(5.))
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(4.))
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .text_color(c(color))
        .hover(|s| s.bg(c(palette().hover)))
        .child(crate::widgets::icon(name, 14., color))
        .child(text.to_string())
}
