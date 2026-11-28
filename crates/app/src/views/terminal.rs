//! Terminal tab: paints the alacritty grid with gpui and maps keys and mouse back to the PTY.

mod keys;
mod mouse;
mod paint;
mod search;
#[cfg(test)]
mod tests;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use gpui::{
    BorderStyle, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, ScrollEvent, SharedString, StrikethroughStyle, TextRun, UnderlineStyle, Window,
    canvas, div, fill, font, outline, point, prelude::*, px, size,
};
use tokio::sync::mpsc;
use wiz_core::secrets::Secret;
use wiz_core::session::{Shell, ShellEvent, ShellInput};
use wiz_core::terminal::{Link, Match, Terminal};

use crate::theme::*;
use crate::widgets::icon_button;
use crate::widgets::input::{InputEvent, TextInput};
use paint::{paint_grid, snapshot};

/// Find-in-scrollback bar state.
struct Search {
    input: gpui::Entity<TextInput>,
    matches: Vec<Match>,
    cur: usize,
}

/// How long the shell must stay quiet before the startup command is typed.
const STARTUP_QUIET_MS: u64 = 300;

pub struct TerminalView {
    search: Option<Search>,
    /// Link under the mouse while the link modifier (`platform::link_click`) is held.
    hover_link: Option<Link>,
    term: Terminal,
    input: mpsc::Sender<ShellInput>,
    focus: FocusHandle,
    cols: u16,
    rows: u16,
    cell_w: Pixels,
    cell_h: Pixels,
    origin: Point<Pixels>,
    exited: Option<Option<u32>>,
    /// The connection dropped under this terminal (as opposed to the shell exiting).
    lost: bool,
    /// Status line shown at the bottom, like "Reconnecting in 4s". Replaces the exit banner.
    notice: Option<SharedString>,
    /// After a reconnect, drop the server's "System is booting up" notice until this time.
    strip_boot_until: Option<std::time::Instant>,
    /// Enter asks for another reconnect attempt.
    retry_ready: bool,
    /// Command this terminal runs instead of a login shell (`docker exec`), re-run on reconnect.
    pub command: Option<String>,
    /// The host's startup command, typed once the shell has drawn its prompt.
    startup: Option<String>,
    /// Bumped by each output while `startup` waits, so only the last quiet spell types it.
    startup_seq: u64,
    title: Option<SharedString>,
    saved_password: Option<Secret>,
    selecting: bool,
    mouse_held: Option<MouseButton>,
    mouse_cell: Option<(usize, usize)>,
}

pub enum TerminalEvent {
    TitleChanged,
    /// The shell ended, with its exit status if the server sent one: either it exited, or the
    /// connection dropped under it.
    Ended(Option<u32>),
    /// Enter was pressed on a lost terminal.
    Retry,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl TerminalView {
    pub fn new(shell: Shell, saved_password: Option<Secret>, cx: &mut Context<Self>) -> Self {
        let Shell { input, events } = shell;
        let (cols, rows) = (100, 30);
        let this = Self {
            search: None,
            hover_link: None,
            term: Terminal::with_scrollback(
                cols,
                rows,
                crate::prefs::get().terminal_scrollback() as usize,
            ),
            input,
            // Reachable with Tab.
            focus: cx.focus_handle().tab_stop(true),
            cols,
            rows,
            cell_w: px(8.),
            cell_h: px(16.),
            origin: point(px(0.), px(0.)),
            exited: None,
            title: None,
            saved_password,
            selecting: false,
            mouse_held: None,
            mouse_cell: None,
            lost: false,
            notice: None,
            retry_ready: false,
            strip_boot_until: None,
            command: None,
            startup: None,
            startup_seq: 0,
        };
        this.pump(events, cx);
        this
    }

    /// Feed shell events into the terminal until the stream ends.
    fn pump(&self, mut events: mpsc::Receiver<ShellEvent>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(first) = events.recv().await {
                // Coalesce bursts into one repaint.
                let mut batch = vec![first];
                while batch.len() < 128 {
                    match events.try_recv() {
                        Ok(ev) => batch.push(ev),
                        Err(_) => break,
                    }
                }
                this.update(cx, |t, cx| {
                    for ev in batch {
                        t.on_event(ev, cx);
                    }
                    // One scan per batch, not per event.
                    if t.search.is_some() {
                        t.research(cx, false);
                    }
                    cx.notify();
                })
                .ok()?;
            }
            None::<()>
        })
        .detach();
    }

    /// Show (or clear) the status line. With `retry`, Enter asks for another attempt.
    pub fn set_notice(&mut self, text: Option<String>, retry: bool, cx: &mut Context<Self>) {
        self.notice = text.map(SharedString::from);
        self.retry_ready = retry;
        cx.notify();
    }

    /// Continue on a fresh shell after a reconnect, keeping the scrollback.
    pub fn attach(&mut self, shell: Shell, password: Option<Secret>, cx: &mut Context<Self>) {
        self.saved_password = password;
        let Shell { input, events } = shell;
        self.input = input;
        self.exited = None;
        self.lost = false;
        self.notice = None;
        self.retry_ready = false;
        // Start from a clean terminal: the old screen and scrollback belong to the dead shell.
        self.term.advance(wiz_core::terminal::RESET_MODES);
        self.term.advance(wiz_core::terminal::CLEAR_ALL);
        self.term.term.selection = None;
        self.term.term.scroll_display(Scroll::Bottom);
        // A server that is still booting prints a notice even to users it lets in.
        self.strip_boot_until =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
        let _ = self.input.try_send(ShellInput::Resize {
            cols: self.cols,
            rows: self.rows,
        });
        self.pump(events, cx);
        cx.notify();
    }

    /// Type `line` once the shell goes quiet after its first output (the prompt). Typed
    /// earlier, the terminal echoes it above the prompt and it shows twice.
    pub fn run_on_prompt(&mut self, line: String) {
        self.startup = Some(line);
    }

    /// Wait for output to stop, then type the startup command.
    fn type_startup_when_quiet(&mut self, cx: &mut Context<Self>) {
        if self.startup.is_none() {
            return;
        }
        self.startup_seq += 1;
        let seq = self.startup_seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(STARTUP_QUIET_MS))
                .await;
            this.update(cx, |t, cx| {
                if t.startup_seq == seq
                    && let Some(line) = t.startup.take()
                {
                    t.paste(&format!("{line}\n"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn title(&self) -> Option<SharedString> {
        self.title.clone()
    }

    fn on_event(&mut self, ev: ShellEvent, cx: &mut Context<Self>) {
        match ev {
            ShellEvent::Data(mut d) => {
                if let Some(until) = self.strip_boot_until {
                    if std::time::Instant::now() > until {
                        self.strip_boot_until = None;
                    } else if let Some(clean) = wiz_core::terminal::strip_boot_notice(&d) {
                        d = clean;
                        self.strip_boot_until = None;
                    }
                }
                self.type_startup_when_quiet(cx);
                let reply = self.term.advance(&d);
                if !reply.is_empty() {
                    self.write(reply, cx);
                }
                let t = self.term.title().map(SharedString::from);
                if t != self.title {
                    self.title = t;
                    cx.emit(TerminalEvent::TitleChanged);
                }
            }
            ShellEvent::Exit(status) => {
                self.exited = Some(status);
                self.lost = status.is_none();
                // The owner decides whether the connection dropped or the shell just exited.
                cx.emit(TerminalEvent::Ended(status));
            }
        }
    }

    fn write(&self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if let Err(mpsc::error::TrySendError::Full(msg)) =
            self.input.try_send(ShellInput::Data(bytes))
        {
            let tx = self.input.clone();
            cx.background_spawn(async move {
                let _ = tx.send(msg).await;
            })
            .detach();
        }
    }

    fn ensure_size(&mut self, cols: u16, rows: u16, cw: Pixels, ch: Pixels, origin: Point<Pixels>) {
        self.cell_w = cw;
        self.cell_h = ch;
        self.origin = origin;
        if (cols, rows) != (self.cols, self.rows) && cols > 1 && rows > 1 {
            self.cols = cols;
            self.rows = rows;
            self.term.resize(cols, rows);
            let _ = self.input.try_send(ShellInput::Resize { cols, rows });
        }
    }

    fn viewport_cell(&self, p: Point<Pixels>) -> (usize, usize) {
        let x = (p.x - self.origin.x).max(px(0.));
        let y = (p.y - self.origin.y).max(px(0.));
        let col = ((x / self.cell_w) as usize).min(self.cols as usize - 1);
        let row = ((y / self.cell_h) as usize).min(self.rows as usize - 1);
        (col, row)
    }

    fn grid_point(&self, p: Point<Pixels>) -> (GridPoint, Side) {
        let (col, row) = self.viewport_cell(p);
        let x = (p.x - self.origin.x).max(px(0.));
        let off = self.term.term.grid().display_offset() as i32;
        let side = if (x % self.cell_w) < self.cell_w / 2. {
            Side::Left
        } else {
            Side::Right
        };
        (GridPoint::new(Line(row as i32 - off), Column(col)), side)
    }

    fn link_under(&self, p: Point<Pixels>) -> Option<Link> {
        let (pt, _) = self.grid_point(p);
        self.term.link_at(pt.line.0, pt.column.0)
    }

    /// Copies the selection to the clipboard; false when nothing is selected.
    fn copy_selection(&self, cx: &mut Context<Self>) -> bool {
        let Some(s) = self
            .term
            .term
            .selection_to_string()
            .filter(|s| !s.is_empty())
        else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(s));
        true
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.paste(&t, cx);
        }
    }

    pub fn paste(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut out = Vec::new();
        let bracketed = self.term.term.mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            out.extend_from_slice(b"\x1b[200~");
        }
        // Terminals expect CR for newlines.
        out.extend_from_slice(text.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
        if bracketed {
            out.extend_from_slice(b"\x1b[201~");
        }
        self.write(out, cx);
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let view2 = view.clone();
        let focused = self.focus.is_focused(window);
        let notice = self.notice.clone();
        let exited = self.exited.filter(|_| notice.is_none());
        let search_bar = self.search_bar(cx);

        let grid = canvas(
            move |bounds, window, cx| {
                let ts = window.text_system();
                let fid = ts.resolve_font(&font(mono()));
                let fs = px(mono_size());
                let cw = ts.advance(fid, fs, 'm').map(|s| s.width).unwrap_or(px(8.));
                let ch = (fs * 1.3).round();
                let cols = (bounds.size.width / cw) as u16;
                let rows = (bounds.size.height / ch) as u16;
                view.update(cx, |t, _| t.ensure_size(cols, rows, cw, ch, bounds.origin));
            },
            move |bounds, _, window, cx| {
                let snap = snapshot(view2.read(cx), focused);
                paint_grid(snap, bounds, focused, window, cx);
            },
        )
        .size_full();

        div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(
                cx.listener(|this, ev: &KeyDownEvent, window, cx| this.on_key(ev, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    // Cmd-click (Ctrl-click on Windows) opens a link instead of selecting.
                    if crate::platform::link_click(ev.modifiers)
                        && let Some(l) = this.link_under(ev.position)
                    {
                        cx.open_url(&l.url);
                        return;
                    }
                    if this.mouse_press(ev, cx) {
                        return;
                    }
                    let (p, side) = this.grid_point(ev.position);
                    let ty = match ev.click_count {
                        2 => SelectionType::Semantic,
                        c if c >= 3 => SelectionType::Lines,
                        _ => SelectionType::Simple,
                    };
                    this.term.term.selection = Some(Selection::new(ty, p, side));
                    this.selecting = true;
                    this.autoscroll_selection(window, cx);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    if this.mouse_press(ev, cx) {
                        return;
                    }
                    // Right click copies a selection, otherwise pastes.
                    if this.copy_selection(cx) {
                        this.term.term.selection = None;
                        cx.notify();
                    } else {
                        this.paste_clipboard(cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.mouse_press(ev, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                let link = if crate::platform::link_click(ev.modifiers) && !this.selecting {
                    this.link_under(ev.position)
                } else {
                    None
                };
                if link != this.hover_link {
                    this.hover_link = link;
                    cx.notify();
                }
                if this.mouse_motion(ev, cx) {
                    return;
                }
                if this.selecting && ev.pressed_button == Some(MouseButton::Left) {
                    let (p, side) = this.grid_point(ev.position);
                    if let Some(sel) = this.term.term.selection.as_mut() {
                        sel.update(p, side);
                    }
                    cx.notify();
                }
            }))
            .capture_any_mouse_up(cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                this.mouse_release(ev, cx);
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _, cx| this.mouse_release(ev, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseUpEvent, _, cx| this.mouse_release(ev, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Middle,
                cx.listener(|this, ev: &MouseUpEvent, _, cx| this.mouse_release(ev, cx)),
            )
            .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| {
                let lines = (ev.delta.pixel_delta(this.cell_h).y / this.cell_h).round() as i32;
                if lines == 0 || this.mouse_wheel(ev, lines, cx) {
                    return;
                }
                let mode = *this.term.term.mode();
                if mode.contains(TermMode::ALT_SCREEN) {
                    // Full-screen apps (less, vim): scroll wheel becomes arrow keys.
                    let key = if lines > 0 { "\x1b[A" } else { "\x1b[B" };
                    let seq = if mode.contains(TermMode::APP_CURSOR) {
                        key.replace('[', "O")
                    } else {
                        key.to_string()
                    };
                    this.write(seq.repeat(lines.unsigned_abs() as usize).into_bytes(), cx);
                } else {
                    this.term.term.scroll_display(Scroll::Delta(lines));
                }
                cx.notify();
            }))
            .when(self.hover_link.is_some(), Styled::cursor_pointer)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(bg_fill())
            .child(div().flex_1().px(px(6.)).py(px(2.)).child(grid))
            .children(search_bar)
            .when_some(notice, |d, text| {
                d.child(
                    div()
                        .px_3()
                        .py_1()
                        .bg(c(palette().panel))
                        .text_xs()
                        .text_color(c(palette().yellow))
                        .child(text),
                )
            })
            .when_some(exited, |d, status| {
                d.child(
                    div()
                        .px_3()
                        .py_1()
                        .bg(c(palette().panel))
                        .text_xs()
                        .text_color(c(palette().yellow))
                        .child(match status {
                            Some(0) | None => "Session closed".to_string(),
                            Some(n) => format!("Session closed (exit status {n})"),
                        }),
                )
            })
    }
}
