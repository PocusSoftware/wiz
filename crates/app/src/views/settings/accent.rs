//! The accent color row: preset swatches, a #RRGGBB field and a color picker.

use std::{cell::Cell, rc::Rc};

use gpui::{
    Anchor, AnchoredPositionMode, Bounds, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, anchored, canvas, deferred, linear_color_stop, linear_gradient,
    point,
};

use super::*;

const PICKER_W: f32 = 200.;
const PLANE_H: f32 = 120.;
const HUE_H: f32 = 14.;

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
    ("White", 0xffffff),
];

/// Which part of the picker the mouse is dragging.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Plane,
    Hue,
}

/// The color picker below the accent row: a saturation/brightness square and a hue bar.
#[derive(Default)]
pub(super) struct Picker {
    open: bool,
    drag: Option<Drag>,
    plane: Rc<Cell<Bounds<Pixels>>>,
    hue: Rc<Cell<Bounds<Pixels>>>,
}

/// Hue (0 to 360), saturation and value (0 to 1) as #RRGGBB.
pub(super) fn hsv_to_rgb((h, s, v): (f32, f32, f32)) -> u32 {
    let channel = |n: f32| {
        let k = (n + h / 60.) % 6.;
        let x = v - v * s * k.min(4. - k).clamp(0., 1.);
        (x * 255.).round() as u32
    };
    channel(5.) << 16 | channel(3.) << 8 | channel(1.)
}

/// Inverse of `hsv_to_rgb`.
pub(super) fn rgb_to_hsv(rgb: u32) -> (f32, f32, f32) {
    let [r, g, b] = [16, 8, 0].map(|shift| ((rgb >> shift) & 0xff) as f32 / 255.);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = match () {
        _ if d == 0. => 0.,
        _ if max == r => ((g - b) / d).rem_euclid(6.),
        _ if max == g => (b - r) / d + 2.,
        _ => (r - g) / d + 4.,
    };
    (h * 60., if max == 0. { 0. } else { d / max }, max)
}

impl SettingsView {
    /// The accent picker. It shows the saved accent, which the gray theme does not use, so
    /// it is dimmed until colorful is on.
    pub(super) fn accent_row(&self, cx: &mut Context<Self>) -> gpui::Div {
        let s = prefs::get();
        let current = parse_hex(&s.accent_color()).unwrap_or(DEFAULT_ACCENT);
        Self::row(
            "Accent color",
            if s.colorful() {
                "Buttons, the active tab, the cursor and focus. Pick one or type any #RRGGBB."
            } else {
                "Used when Colorful is on. Pick one or type any #RRGGBB."
            },
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(SWATCHES.iter().enumerate().map(|(i, &(name, rgb))| {
                    keyboard(div().id(("accent", i)))
                        // The picked swatch already has the light border.
                        .focus(|s| s.border_color(c(palette().muted)))
                        .size(px(20.))
                        .rounded(px(2.))
                        .bg(c(rgb))
                        .border_2()
                        .border_color(c(if rgb == current { palette().text } else { bg() }))
                        .cursor_pointer()
                        .tooltip(tip(name))
                        .on_click(cx.listener(move |t, _, _, cx| t.pick_accent(rgb, cx)))
                }))
                .child(
                    div()
                        .relative()
                        .child(
                            keyboard(div().id("accent-custom"))
                                .focus(|s| s.border_color(c(palette().muted)))
                                .size(px(20.))
                                .rounded(px(2.))
                                .border_2()
                                .border_color(c(if self.picker.open {
                                    palette().text
                                } else {
                                    bg()
                                }))
                                .bg(linear_gradient(
                                    45.,
                                    linear_color_stop(c(0xe5484d), 0.),
                                    linear_color_stop(c(0x0090ff), 1.),
                                ))
                                .cursor_pointer()
                                .tooltip(tip("Custom color"))
                                .on_click(cx.listener(|t, _, _, cx| {
                                    t.picker.open = true;
                                    cx.notify();
                                })),
                        )
                        .when(self.picker.open, |d| {
                            // Deferred, so it paints over the rows below; anchored, so it
                            // stays inside the window.
                            d.child(deferred(
                                anchored()
                                    .position_mode(AnchoredPositionMode::Local)
                                    .position(point(px(20.), px(26.)))
                                    .anchor(Anchor::TopRight)
                                    .snap_to_window_with_margin(px(8.))
                                    .child(self.picker_panel(cx)),
                            ))
                        }),
                )
                .child(div().ml_1().w(px(90.)).child(self.accent.clone())),
        )
        // Dimmed, not disabled, like the privacy options: it can be set up while off.
        .when(!s.colorful(), |d| d.opacity(0.45))
    }

    fn picker_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let rgb = parse_hex(self.accent.read(cx).value()).unwrap_or_else(accent);
        let (h, s, v) = rgb_to_hsv(rgb);
        let hue_color = |deg: f32| c(hsv_to_rgb((deg, 1., 1.)));
        let (plane, hue) = (self.picker.plane.clone(), self.picker.hue.clone());
        let marker = |left: f32, top: f32, w: f32, h: f32| {
            div()
                .absolute()
                .left(px(left - w / 2.))
                .top(px(top - h / 2.))
                .w(px(w))
                .h(px(h))
                .rounded_full()
                .border_2()
                .border_color(c(0xffffff))
        };
        div()
            .p_3()
            .rounded(px(6.))
            .bg(c(palette().elevated))
            .border_1()
            .border_color(c(palette().border))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|t, _, _, cx| {
                t.picker.open = false;
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|t, ev: &MouseMoveEvent, _, cx| {
                if t.picker.drag.is_some() {
                    t.pick_at(ev.position, cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_drag))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::end_drag))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w(px(PICKER_W))
                    .child(
                        div()
                            .relative()
                            .w(px(PICKER_W))
                            .h(px(PLANE_H))
                            .rounded(px(2.))
                            .overflow_hidden()
                            .cursor_crosshair()
                            .bg(hue_color(h))
                            .child(div().absolute().size_full().bg(linear_gradient(
                                90.,
                                linear_color_stop(gpui::white(), 0.),
                                linear_color_stop(gpui::white().opacity(0.), 1.),
                            )))
                            .child(div().absolute().size_full().bg(linear_gradient(
                                180.,
                                linear_color_stop(gpui::black().opacity(0.), 0.),
                                linear_color_stop(gpui::black(), 1.),
                            )))
                            .child(marker(s * PICKER_W, (1. - v) * PLANE_H, 10., 10.))
                            .child(
                                canvas(move |b, _, _| plane.set(b), |_, _, _, _| {})
                                    .absolute()
                                    .size_full(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|t, ev: &MouseDownEvent, _, cx| {
                                    t.picker.drag = Some(Drag::Plane);
                                    t.pick_at(ev.position, cx);
                                }),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .w(px(PICKER_W))
                            .h(px(HUE_H))
                            .rounded(px(2.))
                            .overflow_hidden()
                            .cursor_pointer()
                            .flex()
                            .children((0..6).map(|k| {
                                div().flex_1().h_full().bg(linear_gradient(
                                    90.,
                                    linear_color_stop(hue_color(k as f32 * 60.), 0.),
                                    linear_color_stop(hue_color((k + 1) as f32 * 60.), 1.),
                                ))
                            }))
                            .child(marker(h / 360. * PICKER_W, HUE_H / 2., 6., HUE_H))
                            .child(
                                canvas(move |b, _, _| hue.set(b), |_, _, _, _| {})
                                    .absolute()
                                    .size_full(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|t, ev: &MouseDownEvent, _, cx| {
                                    t.picker.drag = Some(Drag::Hue);
                                    t.pick_at(ev.position, cx);
                                }),
                            ),
                    ),
            )
    }

    /// Move the dragged part of the picker to the mouse. Only the field changes while
    /// dragging, the setting is saved when the button is released.
    fn pick_at(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.picker.drag else {
            return;
        };
        let share = |b: Bounds<Pixels>| {
            (
                ((pos.x - b.origin.x) / b.size.width).clamp(0., 1.),
                ((pos.y - b.origin.y) / b.size.height).clamp(0., 1.),
            )
        };
        let (h, s, v) = rgb_to_hsv(parse_hex(self.accent.read(cx).value()).unwrap_or_else(accent));
        let hsv = match drag {
            Drag::Plane => {
                let (x, y) = share(self.picker.plane.get());
                (h, x, 1. - y)
            }
            Drag::Hue => (share(self.picker.hue.get()).0 * 360., s, v),
        };
        let hex = format!("#{:06x}", hsv_to_rgb(hsv));
        self.accent.update(cx, |i, _| i.set_value(&hex));
        cx.notify();
    }

    fn end_drag(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.picker.drag.take().is_some() {
            self.sync_inputs(cx);
        }
    }

    fn pick_accent(&mut self, rgb: u32, cx: &mut Context<Self>) {
        let hex = format!("#{rgb:06x}");
        self.accent.update(cx, |i, _| i.set_value(&hex));
        self.sync_inputs(cx);
    }
}
