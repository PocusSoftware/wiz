//! The accent color row: preset swatches and a #RRGGBB field.

use super::*;

/// Colors offered as swatches, the default first.
const SWATCHES: &[(&str, u32)] = &[
    ("Orange", DEFAULT_ACCENT),
    ("Red", 0xe5484d),
    ("Pink", 0xd6409f),
    ("Purple", 0x8e4ec6),
    ("Blue", 0x3e63dd),
    ("Sky", 0x0090ff),
    ("Teal", 0x12a594),
    ("Green", 0x46a758),
];

impl SettingsView {
    pub(super) fn accent_row(&self, cx: &mut Context<Self>) -> gpui::Div {
        let current = accent();
        Self::row(
            "Accent color",
            "Buttons, the active tab, the cursor and focus. Pick one or type any #RRGGBB.",
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(SWATCHES.iter().enumerate().map(|(i, &(name, rgb))| {
                    div()
                        .id(("accent", i))
                        .size(px(20.))
                        .rounded(px(2.))
                        .bg(c(rgb))
                        .border_2()
                        .border_color(c(if rgb == current { TEXT } else { BG }))
                        .cursor_pointer()
                        .tooltip(tip(name))
                        .on_click(cx.listener(move |t, _, _, cx| t.pick_accent(rgb, cx)))
                }))
                .child(div().ml_1().w(px(90.)).child(self.accent.clone())),
        )
    }

    fn pick_accent(&mut self, rgb: u32, cx: &mut Context<Self>) {
        let hex = format!("#{rgb:06x}");
        self.accent.update(cx, |i, _| i.set_value(&hex));
        self.sync_inputs(cx);
    }
}
