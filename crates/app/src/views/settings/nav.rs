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
                button("reset", "Reset to defaults").on_click(cx.listener(|t, _, _, cx| {
                    // The secret store stays: switching it means moving secrets.
                    t.apply(cx, |s| {
                        *s = Settings {
                            secret_store: s.secret_store.take(),
                            ..Default::default()
                        }
                    });
                    t.load_inputs(cx);
                })),
            )
            .child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(c(PLACEHOLDER))
                    .child(
                        ssh_core::config_dir()
                            .map(|d| {
                                format!("Saved automatically to {}/settings.toml", d.display())
                            })
                            .unwrap_or_default(),
                    )
                    .child(format!("Wiz {}", crate::version())),
            )
    }

    /// Switch to another page.
    pub(super) fn show(&mut self, category: Category, cx: &mut Context<Self>) {
        self.category = category;
        cx.notify();
    }
}
