//! Small, quick motion: ease-out fades and a few pixels of rise. Nothing loops.

use std::time::Duration;

use gpui::{Animation, AnimationExt, ElementId, IntoElement, Styled, ease_out_quint, px};

/// Fade in over 140 ms. Use an id that changes when the animation should replay.
pub fn fade_in<E: Styled + IntoElement + 'static>(
    el: E,
    id: impl Into<ElementId>,
) -> impl IntoElement {
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(140)).with_easing(ease_out_quint()),
        |el, t| el.opacity(t),
    )
}

/// Fade in while rising 8 px, for dialogs and menus.
pub fn pop_in<E: Styled + IntoElement + 'static>(
    el: E,
    id: impl Into<ElementId>,
) -> impl IntoElement {
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(170)).with_easing(ease_out_quint()),
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
        Animation::new(Duration::from_millis(total as u64)).with_easing(move |t| {
            let local = ((t * total - delay) / DUR).clamp(0.0, 1.0);
            ease(local)
        }),
        |el, t| el.opacity(t).mt(px((1.0 - t) * 8.0)),
    )
}
