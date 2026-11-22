//! Small, quick motion: ease-out fades and a few pixels of rise. Nothing loops.

#[cfg(test)]
mod tests;

use std::time::Duration;

/// Duration in ms, or effectively instant when animations are switched off.
fn ms(n: u64) -> Duration {
    if crate::prefs::get().animations() {
        Duration::from_millis(n)
    } else {
        Duration::from_millis(1)
    }
}

use gpui::{Animation, AnimationExt, ElementId, IntoElement, Styled, ease_out_quint, px};

/// Fade in over 140 ms. Use an id that changes when the animation should replay.
pub fn fade_in<E: Styled + IntoElement + 'static>(
    el: E,
    id: impl Into<ElementId>,
) -> impl IntoElement {
    el.with_animation(
        id,
        Animation::new(ms(140)).with_easing(ease_out_quint()),
        Styled::opacity,
    )
}

/// Short fade for switching pages and tabs. It starts half visible: from nothing, even a
/// quick fade reads as a blank pause before the page shows up.
pub fn page_in<E: Styled + IntoElement + 'static>(
    el: E,
    id: impl Into<ElementId>,
) -> impl IntoElement {
    el.with_animation(
        id,
        Animation::new(ms(80)).with_easing(ease_out_quint()),
        |el, t| el.opacity(0.5 + 0.5 * t),
    )
}

/// Fade in while rising 8 px, for dialogs and menus.
pub fn pop_in<E: Styled + IntoElement + 'static>(
    el: E,
    id: impl Into<ElementId>,
) -> impl IntoElement {
    el.with_animation(
        id,
        Animation::new(ms(170)).with_easing(ease_out_quint()),
        |el, t| el.opacity(t).mt(px((1.0 - t) * 8.0)),
    )
}

/// Like `pop_in`, started `index * 35 ms` later so a row of cards cascades.
pub fn stagger_in<E: Styled + IntoElement + 'static>(
    el: E,
    id: impl Into<ElementId>,
    index: usize,
) -> impl IntoElement {
    const DUR: f32 = 170.0;
    let delay = (index.min(8) as f32) * 35.0;
    let total = DUR + delay;
    let ease = ease_out_quint();
    el.with_animation(
        id,
        Animation::new(ms(total as u64)).with_easing(move |t| {
            let local = ((t * total - delay) / DUR).clamp(0.0, 1.0);
            ease(local)
        }),
        |el, t| el.opacity(t).mt(px((1.0 - t) * 8.0)),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
}

impl Spring {
    const RESPONSE: f32 = 0.3;

    pub fn new(value: f32) -> Self {
        Self {
            value,
            velocity: 0.,
            target: value,
        }
    }

    pub fn step(&mut self, dt: f32) -> bool {
        let k = (std::f32::consts::TAU / Self::RESPONSE).powi(2);
        let c = 4. * std::f32::consts::PI / Self::RESPONSE;
        let dt = dt.min(1. / 30.);
        self.velocity += (-k * (self.value - self.target) - c * self.velocity) * dt;
        self.value += self.velocity * dt;
        let settled = (self.value - self.target).abs() < 1e-3 && self.velocity.abs() < 1e-2;
        if settled || !crate::prefs::get().animations() {
            *self = Self::new(self.target);
            return false;
        }
        true
    }
}
