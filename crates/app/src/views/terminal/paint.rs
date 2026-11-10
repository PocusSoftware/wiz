//! Drawing the grid: colors, a snapshot of the visible cells, and painting it in runs.

use super::*;

fn indexed(i: u8) -> u32 {
    match i {
        0..=15 => palette().ansi[i as usize],
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
                | NamedColor::DimForeground
                | NamedColor::Cursor => palette().text,
                NamedColor::Background => bg(),
                _ if i < 16 => palette().ansi[i],
                _ if i >= NamedColor::DimBlack as usize => {
                    palette().ansi[(i - NamedColor::DimBlack as usize).min(7)]
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

pub(super) struct Snapshot {
    lines: Vec<Vec<(char, Style)>>,
    cw: Pixels,
    ch: Pixels,
    cols: usize,
    cursor_pt: GridPoint,
    cursor_visible: bool,
}

pub(super) fn snapshot(t: &TerminalView, focused: bool) -> Snapshot {
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
        fg: palette().text,
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
        let mut fg = resolve(cell.cell.fg, palette().text);
        let mut back = resolve(cell.cell.bg, bg());
        if f.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut back);
        }
        let selected = selection.is_some_and(|s| s.contains(cell.point));
        let at_cursor = focused && cursor_visible && cell.point == cursor_pt;
        if selected {
            back = crate::theme::selection();
        }
        if at_cursor {
            back = accent();
            fg = bg();
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
                bg: (back != bg()).then_some(back),
                bold: f.contains(Flags::BOLD),
                italic: f.contains(Flags::ITALIC),
                underline: f.intersects(Flags::ALL_UNDERLINES),
                strike: f.contains(Flags::STRIKEOUT),
                dim: f.contains(Flags::DIM),
            },
        );
    }

    if let Some(l) = &t.hover_link {
        for &(line, col) in &l.cells {
            let row = line + off;
            if row >= 0 && (row as usize) < rows && col < cols {
                let st = &mut lines[row as usize][col].1;
                st.underline = true;
                st.fg = accent_text();
            }
        }
    }

    if let Some(s) = &t.search {
        for (i, m) in s.matches.iter().enumerate() {
            let row = m.line + off;
            if row < 0 || row as usize >= rows {
                continue;
            }
            let (bg, fg) = if i == s.cur {
                (accent(), bg())
            } else {
                (palette().match_bg, palette().text)
            };
            let end = (m.col + m.len).min(cols);
            for (_, style) in lines[row as usize][m.col.min(end)..end].iter_mut() {
                style.bg = Some(bg);
                style.fg = fg;
            }
        }
    }

    // Mask hostnames, IPs and usernames in place (same length, so columns never shift).
    if crate::privacy::hide() {
        for row in lines.iter_mut() {
            let chars: Vec<char> = row.iter().map(|(ch, _)| *ch).collect();
            for r in crate::privacy::ranges(&chars) {
                for i in r {
                    if row[i].0 != '\0' {
                        row[i].0 = crate::privacy::mask_char();
                    }
                }
            }
        }
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

pub(super) fn paint_grid(
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
    window.paint_quad(fill(bounds, c(bg())));
    let ts = window.text_system().clone();
    let fs = px(mono_size());

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
                gpui::TextAlign::Left,
                None,
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
        window.paint_quad(outline(b, c(accent()), BorderStyle::Solid));
    }
}
