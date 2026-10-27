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
                .border_color(c(0x00000000))
                .cursor_pointer()
                .text_color(c(if current { TEXT } else { MUTED }))
                .when(current, |d| d.bg(c(ACTIVE)))
                .when(!current, |d| d.hover(|s| s.bg(c(HOVER))))
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
            .border_color(c(BORDER_VARIANT))
            .children(items)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .child(button("reset", "Reset to defaults").on_click(cx.listener(
                                |t, _, _, cx| {
                                    // The secret store stays: switching it means moving secrets.
                                    t.apply(cx, |s| {
                                        *s = Settings {
                                            secret_store: s.secret_store.take(),
                                            ..Default::default()
                                        }
                                    });
                                    t.load_inputs(cx);
                                },
                            ))),
                    )
                    .child(
                        icon_button("open-file", "external-link", "Open settings file", MUTED)
                            .on_click(cx.listener(|t, _, _, cx| t.open_file(cx))),
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
                    .text_color(c(PLACEHOLDER))
                    .child(format!("Wiz {}", crate::version()))
                    .child("Changes save automatically"),
            )
    }

    /// Open `settings.toml` in the editor chosen under Files.
    fn open_file(&mut self, cx: &mut Context<Self>) {
        let opened = ssh_core::config_dir().and_then(|d| {
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
