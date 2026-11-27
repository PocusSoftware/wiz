//! Mouse: selecting text, and reporting clicks, drags and the wheel to programs that turn
//! it on (btop, vim).

use ssh_core::terminal::{MouseAction, MouseButton as Report};

use super::*;

/// How often a drag past the top or bottom edge scrolls.
const AUTOSCROLL_EVERY: std::time::Duration = std::time::Duration::from_millis(50);

/// Lines to scroll while dragging a selection at height `y` over a grid starting at `top`
/// and `height` tall: positive scrolls up into history, farther out scrolls faster.
pub(super) fn autoscroll_lines(y: Pixels, top: Pixels, height: Pixels, cell_h: Pixels) -> i32 {
    let past = if y < top {
        top - y
    } else if y > top + height {
        top + height - y
    } else {
        return 0;
    };
    let lines = (past / cell_h).abs().ceil().min(5.) as i32;
    if past > px(0.) { lines } else { -lines }
}

fn report_button(b: MouseButton) -> Option<Report> {
    match b {
        MouseButton::Left => Some(Report::Left),
        MouseButton::Middle => Some(Report::Middle),
        MouseButton::Right => Some(Report::Right),
        MouseButton::Navigate(_) => None,
    }
}

impl TerminalView {
    /// While a selection drag is held, scroll when the mouse is above or below the grid and
    /// extend the selection. Mouse move events stop at the edge, so this polls the position.
    pub(super) fn autoscroll_selection(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSCROLL_EVERY).await;
                let Ok(true) = this.update_in(cx, |t, window, cx| {
                    if !t.selecting {
                        return false;
                    }
                    let pos = window.mouse_position();
                    let height = t.cell_h * t.rows as f32;
                    let lines = autoscroll_lines(pos.y, t.origin.y, height, t.cell_h);
                    if lines != 0 {
                        t.term.term.scroll_display(Scroll::Delta(lines));
                        let (p, side) = t.grid_point(pos);
                        if let Some(sel) = t.term.term.selection.as_mut() {
                            sel.update(p, side);
                        }
                        cx.notify();
                    }
                    true
                }) else {
                    return;
                };
            }
        })
        .detach();
    }

    /// The left button went up: stop selecting and drop an empty selection (a plain click).
    fn end_selection(&mut self, cx: &mut Context<Self>) {
        self.selecting = false;
        if self
            .term
            .term
            .selection_to_string()
            .is_none_or(|s| s.is_empty())
        {
            self.term.term.selection = None;
        }
        cx.notify();
    }

    fn reports_mouse(&self, m: gpui::Modifiers) -> bool {
        !m.shift && self.term.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn send_mouse(
        &self,
        b: Option<Report>,
        action: MouseAction,
        cell: (usize, usize),
        cx: &mut Context<Self>,
    ) {
        if let Some(bytes) = self.term.mouse_report(b, action, cell.0, cell.1) {
            self.write(bytes, cx);
        }
    }

    pub(super) fn mouse_press(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.reports_mouse(ev.modifiers) {
            return false;
        }
        let cell = self.viewport_cell(ev.position);
        self.mouse_held = Some(ev.button);
        self.mouse_cell = Some(cell);
        self.send_mouse(report_button(ev.button), MouseAction::Press, cell, cx);
        true
    }

    pub(super) fn mouse_release(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.mouse_held == Some(ev.button) {
            self.mouse_held = None;
            let cell = self.viewport_cell(ev.position);
            self.send_mouse(report_button(ev.button), MouseAction::Release, cell, cx);
        } else if ev.button == MouseButton::Left {
            self.end_selection(cx);
        }
    }

    pub(super) fn mouse_motion(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        if self.selecting || !self.reports_mouse(ev.modifiers) {
            return false;
        }
        let cell = self.viewport_cell(ev.position);
        if self.mouse_cell != Some(cell) {
            self.mouse_cell = Some(cell);
            let held = ev.pressed_button.and_then(report_button);
            self.send_mouse(held, MouseAction::Motion, cell, cx);
        }
        true
    }

    pub(super) fn mouse_wheel(&self, ev: &ScrollEvent, lines: i32, cx: &mut Context<Self>) -> bool {
        if !self.reports_mouse(ev.modifiers) {
            return false;
        }
        let b = if lines > 0 {
            Report::WheelUp
        } else {
            Report::WheelDown
        };
        let cell = self.viewport_cell(ev.position);
        for _ in 0..lines.unsigned_abs() {
            self.send_mouse(Some(b), MouseAction::Press, cell, cx);
        }
        true
    }
}
