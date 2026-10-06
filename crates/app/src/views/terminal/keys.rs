//! Keyboard input: shortcuts handled by the view, everything else turned into the bytes a
//! terminal program expects.

use super::*;

pub(super) fn key_bytes(
    key: &str,
    key_char: Option<&str>,
    m: gpui::Modifiers,
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
            if m.control && !crate::platform::alt_gr(m, key, key_char) {
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

impl TerminalView {
    pub(super) fn on_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if let Some(variant) = crate::platform::terminal_chord(*m) {
            match (k.key.as_str(), variant) {
                ("c", false) => {
                    if let Some(s) = self.term.term.selection_to_string() {
                        cx.write_to_clipboard(ClipboardItem::new_string(s));
                    }
                    cx.stop_propagation();
                }
                ("a", false) => {
                    let grid = self.term.term.grid();
                    let top = GridPoint::new(grid.topmost_line(), Column(0));
                    let bottom = GridPoint::new(grid.bottommost_line(), grid.last_column());
                    let mut sel = Selection::new(SelectionType::Simple, top, Side::Left);
                    sel.update(bottom, Side::Right);
                    self.term.term.selection = Some(sel);
                    cx.stop_propagation();
                    cx.notify();
                }
                ("v", false) => {
                    if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        self.paste(&t, cx);
                    }
                    cx.stop_propagation();
                }
                ("f", false) => {
                    self.open_search(window, cx);
                    cx.stop_propagation();
                }
                ("g", shift) if self.search.is_some() => {
                    self.step_match(!shift, cx);
                    cx.stop_propagation();
                }
                // Type the saved password (for sudo and friends).
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
        if self.retry_ready && k.key == "enter" {
            self.retry_ready = false;
            cx.emit(TerminalEvent::Retry);
            cx.stop_propagation();
            return;
        }
        let app_cursor = self.term.term.mode().contains(TermMode::APP_CURSOR);
        let Some(bytes) = key_bytes(&k.key, k.key_char.as_deref(), *m, app_cursor) else {
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
