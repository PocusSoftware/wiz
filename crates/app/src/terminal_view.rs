//! Terminal tab: paints the alacritty grid with gpui and maps keys and mouse back to the PTY.

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use gpui::{
    BorderStyle, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, ScrollWheelEvent, SharedString, StrikethroughStyle, TextRun, UnderlineStyle,
    Window, canvas, div, fill, font, outline, point, prelude::*, px, size,
};
use ssh_core::session::{Shell, ShellEvent, ShellInput};
use ssh_core::terminal::Terminal;
use tokio::sync::mpsc;

use crate::theme::*;

pub struct TerminalView {
    term: Terminal,
    input: mpsc::Sender<ShellInput>,
    focus: FocusHandle,
    cols: u16,
    rows: u16,
    cell_w: Pixels,
    cell_h: Pixels,
    origin: Point<Pixels>,
    exited: Option<Option<u32>>,
    title: Option<SharedString>,
    saved_password: Option<String>,
    selecting: bool,
}

pub enum TerminalEvent {
    TitleChanged,
    Exited,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

const FONT_SIZE: f32 = 14.0;

impl TerminalView {
    pub fn new(shell: Shell, saved_password: Option<String>, cx: &mut Context<Self>) -> Self {
        let Shell { input, mut events } = shell;
        let (cols, rows) = (100, 30);
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
                    cx.notify();
                })
                .ok()?;
            }
            None::<()>
        })
        .detach();
        Self {
            term: Terminal::new(cols, rows),
            input,
            focus: cx.focus_handle(),
            cols,
            rows,
            cell_w: px(8.),
            cell_h: px(16.),
            origin: point(px(0.), px(0.)),
            exited: None,
            title: None,
            saved_password,
            selecting: false,
        }
    }

    pub fn title(&self) -> Option<SharedString> {
        self.title.clone()
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn on_event(&mut self, ev: ShellEvent, cx: &mut Context<Self>) {
        match ev {
            ShellEvent::Data(d) => {
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
                cx.emit(TerminalEvent::Exited);
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

    fn grid_point(&self, p: Point<Pixels>) -> (GridPoint, Side) {
        let x = (p.x - self.origin.x).max(px(0.));
        let y = (p.y - self.origin.y).max(px(0.));
        let col = ((x / self.cell_w) as usize).min(self.cols as usize - 1);
        let row = ((y / self.cell_h) as usize).min(self.rows as usize - 1);
        let off = self.term.term.grid().display_offset() as i32;
        let side = if (x % self.cell_w) < self.cell_w / 2. {
            Side::Left
        } else {
            Side::Right
        };
        (GridPoint::new(Line(row as i32 - off), Column(col)), side)
    }

    fn paste(&mut self, text: &str, cx: &mut Context<Self>) {
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

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.platform {
            match (k.key.as_str(), m.shift) {
                ("c", false) => {
                    if let Some(s) = self.term.term.selection_to_string() {
                        cx.write_to_clipboard(ClipboardItem::new_string(s));
                    }
                    cx.stop_propagation();
                }
                ("v", false) => {
                    if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        self.paste(&t, cx);
                    }
                    cx.stop_propagation();
                }
                // Type the saved keychain password (for sudo and friends).
                ("p", true) => {
                    if let Some(pw) = self.saved_password.clone() {
                        self.write(format!("{pw}\r").into_bytes(), cx);
                    }
                    cx.stop_propagation();
                }
                _ => cx.propagate(),
            }
            return;
        }
        let app_cursor = self.term.term.mode().contains(TermMode::APP_CURSOR);
        let Some(bytes) = key_bytes(&k.key, k.key_char.as_deref(), m, app_cursor) else {
            cx.propagate();
            return;
        };
        self.term.term.scroll_display(Scroll::Bottom);
        self.term.term.selection = None;
        self.write(bytes, cx);
        cx.stop_propagation();
        cx.notify();
    }
}

fn key_bytes(
    key: &str,
    key_char: Option<&str>,
    m: &gpui::Modifiers,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let csi = |c: char| -> Vec<u8> {
        let (intro, mods) = (
            if app_cursor && !m.modified() {
                "\x1bO"
            } else {
                "\x1b["
            },
            1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8,
        );
        if mods > 1 {
            format!("\x1b[1;{mods}{c}").into_bytes()
        } else {
            format!("{intro}{c}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> { format!("\x1b[{n}~").into_bytes() };
    let b: Vec<u8> = match key {
        "enter" => b"\r".to_vec(),
        "tab" if m.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "backspace" if m.alt => b"\x1b\x7f".to_vec(),
        "backspace" if m.control => b"\x08".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "up" => csi('A'),
        "down" => csi('B'),
        "right" => csi('C'),
        "left" => csi('D'),
        "home" => csi('H'),
        "end" => csi('F'),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "delete" => tilde(3),
        "insert" => tilde(2),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        _ => {
            if m.control {
                let ch = key.chars().next().filter(|_| key.chars().count() == 1)?;
                let code = match ch.to_ascii_lowercase() {
                    c @ 'a'..='z' => c as u8 - b'a' + 1,
                    '@' | ' ' | '2' => 0,
                    '[' => 27,
                    '\\' => 28,
                    ']' => 29,
                    '^' | '6' => 30,
                    '_' | '-' => 31,
                    _ => return None,
                };
                let mut v = Vec::new();
                if m.alt {
                    v.push(0x1b);
                }
                v.push(code);
                v
            } else {
                let text = key_char?;
                if text.chars().any(char::is_control) {
                    return None;
                }
                let mut v = Vec::new();
                // Option that composes a different character (| @ [ ] { } ~ on German
                // layouts) sends that character, like Terminal.app. Only plain Option+key
                // becomes an ESC prefix.
                if m.alt && text == key {
                    v.push(0x1b);
                }
                v.extend_from_slice(text.as_bytes());
                v
            }
        }
    };
    Some(b)
}

// ---------- colors ----------

const ANSI: [u32; 16] = [
    0x0a0a0b, 0xf66335, 0xdfbda0, 0xf05a29, 0xa0a0a6, 0xf87c49, 0xb6b6bb, 0xd7d7db, 0x626269,
    0xf87c49, 0xe8cdb6, 0xf87c49, 0xc4c4c9, 0xf9926a, 0xd7d7db, 0xfafafb,
];

fn indexed(i: u8) -> u32 {
    match i {
        0..=15 => ANSI[i as usize],
        16..=231 => {
            let i = i as u32 - 16;
            let lvl = |n: u32| if n == 0 { 0 } else { 55 + 40 * n };
            (lvl(i / 36) << 16) | (lvl((i / 6) % 6) << 8) | lvl(i % 6)
        }
        _ => {
            let v = 8 + 10 * (i as u32 - 232);
            (v << 16) | (v << 8) | v
        }
    }
}

fn resolve(color: Color, default: u32) -> u32 {
    match color {
        Color::Spec(rgb) => ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32,
        Color::Indexed(i) => indexed(i),
        Color::Named(n) => {
            let i = n as usize;
            match n {
                NamedColor::Foreground
                | NamedColor::BrightForeground
                | NamedColor::DimForeground => TEXT,
                NamedColor::Background => BG,
                NamedColor::Cursor => TEXT,
                _ if i < 16 => ANSI[i],
                _ if i >= NamedColor::DimBlack as usize => {
                    ANSI[(i - NamedColor::DimBlack as usize).min(7)]
                }
                _ => default,
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: u32,
    bg: Option<u32>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    dim: bool,
}

struct Run {
    col: usize,
    text: String,
    style: Style,
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
        let exited = self.exited;

        let grid = canvas(
            move |bounds, window, cx| {
                let ts = window.text_system();
                let fid = ts.resolve_font(&font(mono()));
                let fs = px(FONT_SIZE);
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
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    let (p, side) = this.grid_point(ev.position);
                    this.term.term.selection = Some(Selection::new(SelectionType::Simple, p, side));
                    this.selecting = true;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.selecting && ev.pressed_button == Some(MouseButton::Left) {
                    let (p, side) = this.grid_point(ev.position);
                    if let Some(sel) = this.term.term.selection.as_mut() {
                        sel.update(p, side);
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.selecting = false;
                    // A plain click leaves an empty selection behind.
                    if this
                        .term
                        .term
                        .selection_to_string()
                        .is_none_or(|s| s.is_empty())
                    {
                        this.term.term.selection = None;
                    }
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let lines = (ev.delta.pixel_delta(this.cell_h).y / this.cell_h).round() as i32;
                if lines == 0 {
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
            .size_full()
            .flex()
            .flex_col()
            .bg(c(BG))
            .child(div().flex_1().p_1().child(grid))
            .when_some(exited, |d, status| {
                d.child(
                    div()
                        .px_3()
                        .py_1()
                        .bg(c(PANEL))
                        .text_xs()
                        .text_color(c(YELLOW))
                        .child(match status {
                            Some(0) | None => "Session closed".to_string(),
                            Some(n) => format!("Session closed (exit status {n})"),
                        }),
                )
            })
    }
}

struct Snapshot {
    lines: Vec<Vec<(char, Style)>>,
    cw: Pixels,
    ch: Pixels,
    cols: usize,
    cursor_pt: GridPoint,
    cursor_visible: bool,
}

fn snapshot(t: &TerminalView, focused: bool) -> Snapshot {
    let (cw, ch) = (t.cell_w, t.cell_h);
    let cols = t.cols as usize;
    let rows = t.rows as usize;
    let content = t.term.term.renderable_content();
    let off = content.display_offset as i32;
    let cursor_visible =
        content.cursor.shape != alacritty_terminal::vte::ansi::CursorShape::Hidden && off == 0;
    let cursor_pt = content.cursor.point;
    let selection = content.selection;

    let default = Style {
        fg: TEXT,
        bg: None,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        dim: false,
    };
    let mut lines: Vec<Vec<(char, Style)>> = vec![vec![(' ', default); cols]; rows];

    for cell in content.display_iter {
        let row = cell.point.line.0 + off;
        let col = cell.point.column.0;
        if row < 0 || row as usize >= rows || col >= cols {
            continue;
        }
        let f = cell.cell.flags;
        if f.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            lines[row as usize][col] = ('\0', default);
            continue;
        }
        let mut fg = resolve(cell.cell.fg, TEXT);
        let mut bg = resolve(cell.cell.bg, BG);
        if f.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let selected = selection.is_some_and(|s| s.contains(cell.point));
        let at_cursor = focused && cursor_visible && cell.point == cursor_pt;
        if selected {
            bg = SELECTION;
        }
        if at_cursor {
            bg = ACCENT;
            fg = BG;
        }
        let ch_ = if f.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.cell.c
        };
        lines[row as usize][col] = (
            ch_,
            Style {
                fg,
                bg: (bg != BG).then_some(bg),
                bold: f.contains(Flags::BOLD),
                italic: f.contains(Flags::ITALIC),
                underline: f.intersects(Flags::ALL_UNDERLINES),
                strike: f.contains(Flags::STRIKEOUT),
                dim: f.contains(Flags::DIM),
            },
        );
    }

    Snapshot {
        lines,
        cw,
        ch,
        cols,
        cursor_pt,
        cursor_visible,
    }
}

fn paint_grid(
    snap: Snapshot,
    bounds: gpui::Bounds<Pixels>,
    focused: bool,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let Snapshot {
        lines,
        cw,
        ch,
        cols,
        cursor_pt,
        cursor_visible,
    } = snap;
    window.paint_quad(fill(bounds, c(BG)));
    let ts = window.text_system().clone();
    let fs = px(FONT_SIZE);

    for (r, line) in lines.iter().enumerate() {
        let y = bounds.origin.y + ch * r as f32;
        let mut runs: Vec<Run> = Vec::new();
        // Background spans.
        let mut c0 = 0;
        while c0 < cols {
            let bg = line[c0].1.bg;
            let mut c1 = c0 + 1;
            while c1 < cols && line[c1].1.bg == bg {
                c1 += 1;
            }
            if let Some(bg) = bg {
                window.paint_quad(fill(
                    gpui::Bounds::new(
                        point(bounds.origin.x + cw * c0 as f32, y),
                        size(cw * (c1 - c0) as f32, ch),
                    ),
                    c(bg),
                ));
            }
            c0 = c1;
        }
        // Text runs split on style change and on blanks (blanks need no glyphs).
        for (col, (chr, style)) in line.iter().enumerate() {
            if *chr == '\0' || *chr == ' ' {
                continue;
            }
            match runs.last_mut() {
                Some(r) if r.style == *style && r.col + r.text.chars().count() == col => {
                    r.text.push(*chr)
                }
                _ => runs.push(Run {
                    col,
                    text: chr.to_string(),
                    style: *style,
                }),
            }
        }
        for run in runs {
            let s = run.style;
            let mut color: Hsla = c(s.fg);
            if s.dim {
                color.a = 0.6;
            }
            let mut f = font(mono());
            if s.bold {
                f.weight = FontWeight::BOLD;
            }
            if s.italic {
                f.style = gpui::FontStyle::Italic;
            }
            let tr = TextRun {
                len: run.text.len(),
                font: f,
                color,
                background_color: None,
                underline: s.underline.then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: s.strike.then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(color),
                }),
            };
            let shaped = ts.shape_line(run.text.into(), fs, &[tr], None);
            let _ = shaped.paint(
                point(bounds.origin.x + cw * run.col as f32, y),
                ch,
                window,
                cx,
            );
        }
    }

    // Hollow cursor when unfocused.
    if !focused && cursor_visible {
        let b = gpui::Bounds::new(
            point(
                bounds.origin.x + cw * cursor_pt.column.0 as f32,
                bounds.origin.y + ch * cursor_pt.line.0 as f32,
            ),
            size(cw, ch),
        );
        window.paint_quad(outline(b, c(ACCENT), BorderStyle::Solid));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn alt() -> Modifiers {
        Modifiers {
            alt: true,
            ..Default::default()
        }
    }

    #[test]
    fn option_composed_chars_have_no_esc_prefix() {
        // German layout: Option+7 is "|", Option+L is "@".
        assert_eq!(key_bytes("7", Some("|"), &alt(), false).unwrap(), b"|");
        assert_eq!(key_bytes("l", Some("@"), &alt(), false).unwrap(), b"@");
        // Plain Option+key where the OS gives the same char becomes Meta (ESC prefix).
        assert_eq!(key_bytes("b", Some("b"), &alt(), false).unwrap(), b"\x1bb");
    }

    #[test]
    fn basic_keys() {
        let none = Modifiers::default();
        assert_eq!(key_bytes("a", Some("a"), &none, false).unwrap(), b"a");
        assert_eq!(key_bytes("enter", None, &none, false).unwrap(), b"\r");
        assert_eq!(key_bytes("up", None, &none, false).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("up", None, &none, true).unwrap(), b"\x1bOA");
        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(key_bytes("c", Some("c"), &ctrl, false).unwrap(), b"\x03");
    }
}
