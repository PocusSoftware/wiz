//! Two-pane SFTP browser: local on the left, remote on the right.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use gpui::{
    ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, Render, SharedString, Window, div, prelude::*, px,
    uniform_list, UniformListScrollHandle, ScrollStrategy,
};
use ssh_core::sftp::{self, Entry, Progress, Sftp, local};

use crate::input::TextInput;
use crate::runtime;
use crate::theme::*;
use crate::ui::{button, icon, icon_button, primary_button};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Local,
    Remote,
}

impl Side {
    fn other(self) -> Side {
        match self {
            Side::Local => Side::Remote,
            Side::Remote => Side::Local,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Side::Local => "Local",
            Side::Remote => "Remote",
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Side::Local => "l",
            Side::Remote => "r",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Size,
    Modified,
}

/// Payload while dragging rows between panes.
#[derive(Clone)]
struct DragEntries {
    from: Side,
    names: Vec<String>,
}

struct DragLabel(SharedString);

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py(px(3.))
            .rounded(px(2.))
            .bg(c(ACCENT))
            .text_color(c(0xffffff))
            .child(self.0.clone())
    }
}

struct Pane {
    side: Side,
    path: String,
    input: Entity<TextInput>,
    raw: Vec<Entry>,
    entries: Vec<Entry>,
    selected: BTreeSet<String>,
    cursor: usize,
    anchor: usize,
    history: Vec<String>,
    loading: bool,
    error: Option<SharedString>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    sort: SortKey,
    asc: bool,
    hidden: bool,
    nav: u64,
    typed: (String, Instant),
}

impl Pane {
    fn new(side: Side, cx: &mut Context<SftpView>) -> Self {
        Self {
            side,
            path: String::new(),
            input: cx.new(|cx| TextInput::new("path", false, cx)),
            raw: vec![],
            entries: vec![],
            selected: BTreeSet::new(),
            cursor: 0,
            anchor: 0,
            history: vec![],
            loading: false,
            error: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            sort: SortKey::Name,
            asc: true,
            hidden: true,
            nav: 0,
            typed: (String::new(), Instant::now()),
        }
    }

    /// Rebuild the visible list: hidden filter, then sort (folders always first).
    fn rebuild(&mut self) {
        let mut v: Vec<Entry> = self
            .raw
            .iter()
            .filter(|e| self.hidden || !e.name.starts_with('.'))
            .cloned()
            .collect();
        let asc = self.asc;
        let key = self.sort;
        v.sort_by(|a, b| {
            let ord = match key {
                SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Modified => a.modified.cmp(&b.modified),
            };
            b.is_dir
                .cmp(&a.is_dir)
                .then(if asc { ord } else { ord.reverse() })
        });
        self.entries = v;
        self.selected.retain(|n| self.entries.iter().any(|e| &e.name == n));
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
    }

    fn selected_names(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| self.selected.contains(&e.name))
            .map(|e| e.name.clone())
            .collect()
    }
}

enum Prompt {
    NewFolder(Side),
    Rename(Side, String),
    Delete(Side, Vec<String>),
    Conflict { from: Side, names: Vec<String>, existing: Vec<String> },
}

enum TState {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

struct Transfer {
    id: u64,
    label: String,
    up: bool,
    progress: Arc<Progress>,
    state: TState,
    finished: Option<Instant>,
    last_done: u64,
    speed: f64,
}

pub struct SftpView {
    sftp: Arc<Sftp>,
    local: Pane,
    remote: Pane,
    active: Side,
    prompt: Option<Prompt>,
    prompt_input: Entity<TextInput>,
    transfers: Vec<Transfer>,
    next_id: u64,
}

fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", U[i]) }
}

fn fmt_time(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string(),
        None => String::new(),
    }
}

async fn fetch(
    side: Side,
    sftp: Arc<Sftp>,
    path: String,
) -> Result<(String, Vec<Entry>), ssh_core::Error> {
    match side {
        Side::Local => {
            runtime::spawn(async move {
                tokio::task::spawn_blocking(move || {
                    let p = if path == "~" || path.starts_with("~/") {
                        local::home().join(path.trim_start_matches('~').trim_start_matches('/'))
                    } else {
                        PathBuf::from(&path)
                    };
                    let canon = std::fs::canonicalize(&p)?;
                    let list = local::list(&canon)?;
                    Ok((canon.to_string_lossy().into_owned(), list))
                })
                .await
                .map_err(|e| ssh_core::Error::Sftp(e.to_string()))?
            })
            .await
        }
        Side::Remote => {
            runtime::spawn(async move {
                let path = if path == "~" || path.starts_with("~/") {
                    let home = sftp.home().await?;
                    sftp::join(&home, path.trim_start_matches('~').trim_start_matches('/'))
                } else {
                    path
                };
                let canon = sftp.canonical(&path).await?;
                let list = sftp.list(&canon).await?;
                Ok((canon, list))
            })
            .await
        }
    }
}

impl SftpView {
    pub fn new(sftp: Arc<Sftp>, cx: &mut Context<Self>) -> Self {
        let this = Self {
            sftp,
            local: Pane::new(Side::Local, cx),
            remote: Pane::new(Side::Remote, cx),
            active: Side::Remote,
            prompt: None,
            prompt_input: cx.new(|cx| TextInput::new("name", false, cx)),
            transfers: vec![],
            next_id: 1,
        };
        // Speed and auto-dismiss ticker.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let alive = this
                    .update(cx, |t, cx| {
                        let now = Instant::now();
                        let before = t.transfers.len();
                        for tr in t.transfers.iter_mut() {
                            if matches!(tr.state, TState::Running) {
                                let done = tr.progress.done.load(Ordering::Relaxed);
                                tr.speed = (done - tr.last_done) as f64 * 2.0;
                                tr.last_done = done;
                            }
                        }
                        t.transfers.retain(|tr| match (&tr.state, tr.finished) {
                            (TState::Done, Some(f)) => now.duration_since(f) < Duration::from_secs(6),
                            _ => true,
                        });
                        if !t.transfers.is_empty() || before != t.transfers.len() {
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        this.go_initial(cx);
        this
    }

    fn go_initial(&self, cx: &mut Context<Self>) {
        let sftp = self.sftp.clone();
        cx.spawn(async move |this, cx| {
            let remote_home = runtime::spawn(async move { sftp.home().await }).await.unwrap_or("/".into());
            this.update(cx, |t, cx| {
                t.go(Side::Local, "~".into(), false, cx);
                t.go(Side::Remote, remote_home, false, cx);
            })
            .ok();
        })
        .detach();
    }

    fn pane(&self, s: Side) -> &Pane {
        match s {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        }
    }

    fn pane_mut(&mut self, s: Side) -> &mut Pane {
        match s {
            Side::Local => &mut self.local,
            Side::Remote => &mut self.remote,
        }
    }

    // ---------- navigation ----------

    fn go(&mut self, side: Side, path: String, push_history: bool, cx: &mut Context<Self>) {
        let sftp = self.sftp.clone();
        let p = self.pane_mut(side);
        p.nav += 1;
        let nav = p.nav;
        p.loading = true;
        let prev = p.path.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = fetch(side, sftp, path).await;
            this.update(cx, |t, cx| {
                let p = t.pane_mut(side);
                if p.nav != nav {
                    return; // a newer navigation superseded this one
                }
                p.loading = false;
                match r {
                    Ok((canon, list)) => {
                        if push_history && !prev.is_empty() && prev != canon {
                            p.history.push(prev);
                        }
                        p.path = canon.clone();
                        p.raw = list;
                        p.error = None;
                        p.selected.clear();
                        p.cursor = 0;
                        p.anchor = 0;
                        p.rebuild();
                        p.input.update(cx, |i, _| i.set_value(&canon));
                        p.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }
                    Err(e) => {
                        p.error = Some(e.to_string().into());
                        let cur = p.path.clone();
                        p.input.update(cx, |i, _| i.set_value(&cur));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh(&mut self, side: Side, cx: &mut Context<Self>) {
        let path = self.pane(side).path.clone();
        if !path.is_empty() {
            self.go(side, path, false, cx);
        }
    }

    fn up(&mut self, side: Side, cx: &mut Context<Self>) {
        let p = self.pane(side);
        let parent = match side {
            Side::Remote => sftp::parent(&p.path),
            Side::Local => std::path::Path::new(&p.path)
                .parent()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".into()),
        };
        self.go(side, parent, true, cx);
    }

    fn back(&mut self, side: Side, cx: &mut Context<Self>) {
        if let Some(prev) = self.pane_mut(side).history.pop() {
            self.go(side, prev, false, cx);
        }
    }

    fn open_dir(&mut self, side: Side, ix: usize, cx: &mut Context<Self>) {
        let p = self.pane(side);
        if let Some(e) = p.entries.get(ix)
            && e.is_dir
        {
            let target = match side {
                Side::Remote => sftp::join(&p.path, &e.name),
                Side::Local => PathBuf::from(&p.path).join(&e.name).to_string_lossy().into_owned(),
            };
            self.go(side, target, true, cx);
        }
    }

    fn submit_path(&mut self, side: Side, cx: &mut Context<Self>) {
        let v = self.pane(side).input.read(cx).value().trim().to_string();
        if !v.is_empty() {
            self.go(side, v, true, cx);
        }
    }

    // ---------- selection ----------

    fn select_row(&mut self, side: Side, ix: usize, shift: bool, toggle: bool) {
        let p = self.pane_mut(side);
        let Some(name) = p.entries.get(ix).map(|e| e.name.clone()) else { return };
        if shift {
            let (a, b) = (p.anchor.min(ix), p.anchor.max(ix));
            p.selected = p.entries[a..=b].iter().map(|e| e.name.clone()).collect();
        } else if toggle {
            if !p.selected.remove(&name) {
                p.selected.insert(name);
            }
            p.anchor = ix;
        } else {
            p.selected.clear();
            p.selected.insert(name);
            p.anchor = ix;
        }
        p.cursor = ix;
    }

    fn move_cursor(&mut self, side: Side, delta: isize, extend: bool) {
        let n = self.pane(side).entries.len();
        if n == 0 {
            return;
        }
        let cur = self.pane(side).cursor as isize;
        let next = (cur + delta).clamp(0, n as isize - 1) as usize;
        self.select_row(side, next, extend, false);
        if extend {
            self.pane_mut(side).cursor = next;
        }
        self.pane(side).scroll.scroll_to_item(next, ScrollStrategy::Nearest);
    }

    fn on_key(&mut self, side: Side, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        self.active = side;
        let cur = self.pane(side).cursor;
        match k.key.as_str() {
            "up" if m.platform => self.up(side, cx),
            "down" if m.platform => self.open_dir(side, cur, cx),
            "up" => self.move_cursor(side, -1, m.shift),
            "down" => self.move_cursor(side, 1, m.shift),
            "pageup" => self.move_cursor(side, -15, m.shift),
            "pagedown" => self.move_cursor(side, 15, m.shift),
            "home" => self.move_cursor(side, isize::MIN / 2, m.shift),
            "end" => self.move_cursor(side, isize::MAX / 2, m.shift),
            "enter" => self.open_dir(side, cur, cx),
            "backspace" if m.platform => self.ask_delete(side, window, cx),
            "backspace" => self.up(side, cx),
            "delete" => self.ask_delete(side, window, cx),
            "left" if m.platform => self.back(side, cx),
            "tab" => {
                let o = side.other();
                self.active = o;
                window.focus(&self.pane(o).focus);
            }
            "escape" => {
                self.pane_mut(side).selected.clear();
            }
            "f2" => self.ask_rename(side, window, cx),
            "f5" => self.begin_transfer(side, cx),
            "r" if m.platform => self.refresh(side, cx),
            "a" if m.platform => {
                let p = self.pane_mut(side);
                p.selected = p.entries.iter().map(|e| e.name.clone()).collect();
            }
            "n" if m.platform && m.shift => self.ask_new_folder(side, window, cx),
            _ => {
                // Type-ahead: jump to the first name starting with what was typed.
                match &k.key_char {
                    Some(ch) if !m.platform && !m.control && !ch.chars().any(char::is_control) => {
                        let p = self.pane_mut(side);
                        if p.typed.1.elapsed() > Duration::from_millis(900) {
                            p.typed.0.clear();
                        }
                        p.typed.0.push_str(&ch.to_lowercase());
                        p.typed.1 = Instant::now();
                        let q = p.typed.0.clone();
                        if let Some(ix) = p.entries.iter().position(|e| e.name.to_lowercase().starts_with(&q)) {
                            self.select_row(side, ix, false, false);
                            self.pane(side).scroll.scroll_to_item(ix, ScrollStrategy::Center);
                        }
                    }
                    _ => {
                        cx.propagate();
                        return;
                    }
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    // ---------- prompts ----------

    fn open_prompt(&mut self, prompt: Prompt, initial: &str, focus_input: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_input.update(cx, |i, _| i.set_value(initial));
        self.prompt = Some(prompt);
        if focus_input {
            window.focus(&self.prompt_input.focus_handle(cx));
        }
        cx.notify();
    }

    fn close_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = None;
        window.focus(&self.pane(self.active).focus);
        cx.notify();
    }

    fn ask_new_folder(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        self.open_prompt(Prompt::NewFolder(side), "", true, window, cx);
    }

    fn ask_rename(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let names = self.pane(side).selected_names();
        if let [name] = names.as_slice() {
            let name = name.clone();
            self.open_prompt(Prompt::Rename(side, name.clone()), &name, true, window, cx);
        }
    }

    fn ask_delete(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let names = self.pane(side).selected_names();
        if !names.is_empty() {
            self.open_prompt(Prompt::Delete(side, names), "", false, window, cx);
        }
    }

    fn confirm_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.prompt_input.read(cx).value().trim().to_string();
        match self.prompt.take() {
            Some(Prompt::NewFolder(side)) if !value.is_empty() => self.fs_op(side, FsOp::Mkdir(value), cx),
            Some(Prompt::Rename(side, old)) if !value.is_empty() && value != old => {
                self.fs_op(side, FsOp::Rename(old, value), cx)
            }
            Some(Prompt::Delete(side, names)) => self.fs_op(side, FsOp::Delete(names), cx),
            _ => {}
        }
        window.focus(&self.pane(self.active).focus);
        cx.notify();
    }

    // ---------- file operations ----------

    fn fs_op(&mut self, side: Side, op: FsOp, cx: &mut Context<Self>) {
        let sftp = self.sftp.clone();
        let dir = self.pane(side).path.clone();
        cx.spawn(async move |this, cx| {
            let r: Result<(), ssh_core::Error> = match side {
                Side::Remote => {
                    runtime::spawn(async move {
                        match op {
                            FsOp::Mkdir(n) => sftp.mkdir(&sftp::join(&dir, &n)).await,
                            FsOp::Rename(a, b) => sftp.rename(&sftp::join(&dir, &a), &sftp::join(&dir, &b)).await,
                            FsOp::Delete(names) => {
                                for n in names {
                                    sftp.remove(&sftp::join(&dir, &n)).await?;
                                }
                                Ok(())
                            }
                        }
                    })
                    .await
                }
                Side::Local => {
                    runtime::spawn(async move {
                        tokio::task::spawn_blocking(move || {
                            let d = PathBuf::from(dir);
                            match op {
                                FsOp::Mkdir(n) => local::mkdir(&d.join(n)),
                                FsOp::Rename(a, b) => local::rename(&d.join(a), &d.join(b)),
                                FsOp::Delete(names) => {
                                    for n in names {
                                        local::remove(&d.join(n))?;
                                    }
                                    Ok(())
                                }
                            }
                        })
                        .await
                        .map_err(|e| ssh_core::Error::Sftp(e.to_string()))?
                    })
                    .await
                }
            };
            this.update(cx, |t, cx| {
                if let Err(e) = r {
                    t.pane_mut(side).error = Some(e.to_string().into());
                }
                t.refresh(side, cx);
            })
            .ok();
        })
        .detach();
    }

    // ---------- transfers ----------

    /// Copy the selection of `from` into the other pane's directory.
    fn begin_transfer(&mut self, from: Side, cx: &mut Context<Self>) {
        let names = self.pane(from).selected_names();
        self.begin_transfer_names(from, names, cx);
    }

    fn begin_transfer_names(&mut self, from: Side, names: Vec<String>, cx: &mut Context<Self>) {
        if names.is_empty() {
            return;
        }
        let to = from.other();
        let dest_dir = self.pane(to).path.clone();
        let sftp = self.sftp.clone();
        let check = names.clone();
        cx.spawn(async move |this, cx| {
            // Which names already exist at the destination?
            let existing: Vec<String> = match to {
                Side::Local => check
                    .iter()
                    .filter(|n| PathBuf::from(&dest_dir).join(n).exists())
                    .cloned()
                    .collect(),
                Side::Remote => {
                    let d = dest_dir.clone();
                    let c = check.clone();
                    runtime::spawn(async move {
                        let mut v = vec![];
                        for n in c {
                            if sftp.exists(&sftp::join(&d, &n)).await.unwrap_or(false) {
                                v.push(n);
                            }
                        }
                        v
                    })
                    .await
                }
            };
            this.update(cx, |t, cx| {
                if existing.is_empty() {
                    t.run_transfer(from, names, cx);
                } else {
                    t.prompt = Some(Prompt::Conflict { from, names, existing });
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn run_transfer(&mut self, from: Side, names: Vec<String>, cx: &mut Context<Self>) {
        if names.is_empty() {
            return;
        }
        let up = from == Side::Local;
        let to = from.other();
        let src_dir = self.pane(from).path.clone();
        let dst_dir = self.pane(to).path.clone();
        let sftp = self.sftp.clone();
        let progress = Progress::new();
        let id = self.next_id;
        self.next_id += 1;
        let label = if names.len() == 1 { names[0].clone() } else { format!("{} items", names.len()) };
        self.transfers.push(Transfer {
            id,
            label,
            up,
            progress: progress.clone(),
            state: TState::Running,
            finished: None,
            last_done: 0,
            speed: 0.0,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = runtime::spawn(async move {
                for n in names {
                    if up {
                        sftp.upload(&PathBuf::from(&src_dir).join(&n), &sftp::join(&dst_dir, &n), &progress)
                            .await?;
                    } else {
                        sftp.download(&sftp::join(&src_dir, &n), &PathBuf::from(&dst_dir).join(&n), &progress)
                            .await?;
                    }
                }
                Ok::<_, ssh_core::Error>(())
            })
            .await;
            this.update(cx, |t, cx| {
                if let Some(tr) = t.transfers.iter_mut().find(|x| x.id == id) {
                    tr.finished = Some(Instant::now());
                    tr.state = match r {
                        Ok(()) => TState::Done,
                        Err(ssh_core::Error::Cancelled) => TState::Cancelled,
                        Err(e) => TState::Failed(e.to_string()),
                    };
                }
                t.refresh(to, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---------- rendering ----------

    fn render_rows(&mut self, side: Side, range: std::ops::Range<usize>, cx: &mut Context<Self>) -> Vec<gpui::Stateful<gpui::Div>> {
        let p = self.pane(side);
        let active = self.active == side;
        range
            .filter_map(|ix| p.entries.get(ix).map(|e| (ix, e.clone())))
            .map(|(ix, e)| {
                let selected = p.selected.contains(&e.name);
                let is_cursor = active && p.cursor == ix;
                let drag_names = if selected { p.selected_names() } else { vec![e.name.clone()] };
                let drag_label: SharedString = if drag_names.len() == 1 {
                    drag_names[0].clone().into()
                } else {
                    format!("{} items", drag_names.len()).into()
                };
                div()
                    .id((side.tag(), ix))
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(selected, |d| d.bg(c(SELECTED_ROW)))
                    .when(is_cursor, |d| d.border_l_2().border_color(c(ACCENT)))
                    .hover(|s| s.bg(c(HOVER)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |t, ev: &MouseDownEvent, window, cx| {
                            t.active = side;
                            window.focus(&t.pane(side).focus);
                            let m = ev.modifiers;
                            let already = t.pane(side).selected.iter().len() > 1
                                && t.pane(side).entries.get(ix).is_some_and(|e| t.pane(side).selected.contains(&e.name));
                            if !(already && !m.shift && !m.platform) {
                                t.select_row(side, ix, m.shift, m.platform);
                            }
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |t, ev: &ClickEvent, _, cx| {
                        if ev.click_count() >= 2 {
                            t.open_dir(side, ix, cx);
                        } else if ev.modifiers() == Default::default() && t.pane(side).selected.len() > 1 {
                            // Plain click inside a multi-selection collapses it (after drag could start).
                            t.select_row(side, ix, false, false);
                            cx.notify();
                        }
                    }))
                    .on_drag(DragEntries { from: side, names: drag_names }, move |_, _, _, cx| {
                        cx.new(|_| DragLabel(drag_label.clone()))
                    })
                    .child(icon(
                        if e.is_dir { "folder" } else { "file" },
                        14.,
                        if e.is_dir { ACCENT_TEXT } else { MUTED },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(div().whitespace_nowrap().child(e.name.clone())),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .text_color(c(PLACEHOLDER))
                            .child(if e.is_dir { String::new() } else { human_size(e.size) }),
                    )
                    .child(
                        div()
                            .w(px(104.))
                            .flex_none()
                            .text_color(c(PLACEHOLDER))
                            .whitespace_nowrap()
                            .child(fmt_time(e.modified)),
                    )
            })
            .collect()
    }

    fn sort_header(&self, side: Side, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.pane(side);
        let cell = |id: &'static str, text: &'static str, key: SortKey| {
            let active = p.sort == key;
            div()
                .id((id, side as usize))
                .flex()
                .items_center()
                .gap(px(2.))
                .cursor_pointer()
                .text_color(c(if active { TEXT } else { PLACEHOLDER }))
                .hover(|s| s.text_color(c(TEXT)))
                .on_click(cx.listener(move |t, _, _, cx| {
                    let p = t.pane_mut(side);
                    if p.sort == key {
                        p.asc = !p.asc;
                    } else {
                        p.sort = key;
                        p.asc = true;
                    }
                    p.rebuild();
                    cx.notify();
                }))
                .child(text)
                .when(active, |d| d.child(icon(if p.asc { "chevron-up" } else { "chevron-down" }, 10., TEXT)))
        };
        div()
            .h(px(20.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(c(BORDER_VARIANT))
            .text_xs()
            .child(div().w(px(14.)).flex_none())
            .child(div().flex_1().child(cell("h-name", "Name", SortKey::Name)))
            .child(div().w(px(64.)).flex_none().flex().justify_end().child(cell("h-size", "Size", SortKey::Size)))
            .child(div().w(px(104.)).flex_none().child(cell("h-mod", "Modified", SortKey::Modified)))
    }

    fn pane_view(&self, side: Side, cx: &mut Context<Self>) -> gpui::AnyElement {
        let p = self.pane(side);
        let active = self.active == side;
        let count = p.entries.len();
        let sel = p.selected.len();
        let can_back = !p.history.is_empty();

        let toolbar = div()
            .h(px(32.))
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(2.))
            .border_b_1()
            .border_color(c(BORDER_VARIANT))
            .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, window, cx| match ev.keystroke.key.as_str() {
                "enter" => {
                    t.submit_path(side, cx);
                    window.focus(&t.pane(side).focus);
                    cx.stop_propagation();
                }
                "escape" => {
                    let cur = t.pane(side).path.clone();
                    t.pane(side).input.update(cx, |i, _| i.set_value(&cur));
                    window.focus(&t.pane(side).focus);
                    cx.stop_propagation();
                    cx.notify();
                }
                _ => {}
            }))
            .child(icon_button((side.tag(), 1000usize), "arrow-left", "Back", if can_back { MUTED } else { ACTIVE }).on_click(
                cx.listener(move |t, _, _, cx| t.back(side, cx)),
            ))
            .child(icon_button((side.tag(), 1001usize), "arrow-up", "Parent folder (⌘↑)", MUTED).on_click(
                cx.listener(move |t, _, _, cx| t.up(side, cx)),
            ))
            .child(icon_button((side.tag(), 1002usize), "home", "Home", MUTED).on_click(
                cx.listener(move |t, _, _, cx| t.go(side, "~".into(), true, cx)),
            ))
            .child(div().flex_1().min_w_0().child(p.input.clone()))
            .child(icon_button((side.tag(), 1003usize), "refresh", "Refresh (⌘R)", MUTED).on_click(
                cx.listener(move |t, _, _, cx| t.refresh(side, cx)),
            ))
            .child(
                icon_button(
                    (side.tag(), 1004usize),
                    if p.hidden { "eye" } else { "eye-off" },
                    if p.hidden { "Hide dotfiles" } else { "Show dotfiles" },
                    MUTED,
                )
                .on_click(cx.listener(move |t, _, _, cx| {
                    let p = t.pane_mut(side);
                    p.hidden = !p.hidden;
                    p.rebuild();
                    cx.notify();
                })),
            )
            .child(icon_button((side.tag(), 1005usize), "folder-plus", "New folder (⇧⌘N)", MUTED).on_click(
                cx.listener(move |t, _, w, cx| t.ask_new_folder(side, w, cx)),
            ));

        let list = uniform_list(
            (side.tag(), 2000usize),
            count,
            cx.processor(move |t: &mut SftpView, range, _, cx| t.render_rows(side, range, cx)),
        )
        .track_scroll(p.scroll.clone())
        .flex_1()
        .min_h_0();

        let body = if p.loading && p.entries.is_empty() {
            div().flex_1().p_3().text_color(c(PLACEHOLDER)).child("Loading...").into_any_element()
        } else if count == 0 {
            div().flex_1().p_3().text_color(c(PLACEHOLDER)).child("Empty folder").into_any_element()
        } else {
            list.into_any_element()
        };

        div()
            .id((side.tag(), 3000usize))
            .track_focus(&p.focus)
            .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, w, cx| t.on_key(side, ev, w, cx)))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_2()
            .border_color(c(if active { BORDER_VARIANT } else { 0x00000000 }))
            .drag_over::<DragEntries>(move |s, d, _, _| if d.from != side { s.border_color(c(ACCENT)).bg(c(0xf05a2914)) } else { s })
            .on_drop(cx.listener(move |t, d: &DragEntries, _, cx| {
                if d.from != side {
                    t.begin_transfer_names(d.from, d.names.clone(), cx);
                }
            }))
            .child(
                div()
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .bg(c(PANEL))
                    .child(icon(if side == Side::Local { "folder" } else { "server" }, 13., if active { ACCENT_TEXT } else { MUTED }))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(side.name()))
                    .child(div().flex_1())
                    .when(p.loading, |d| d.child(div().text_color(c(PLACEHOLDER)).child("loading")))
                    .child(div().text_color(c(PLACEHOLDER)).child(if sel > 0 {
                        format!("{sel} of {count} selected")
                    } else {
                        format!("{count} items")
                    })),
            )
            .child(toolbar)
            .when_some(p.error.clone(), |d, e| {
                d.child(div().px_2().py(px(3.)).text_color(c(RED)).bg(c(0xe5534b1a)).child(e))
            })
            .child(self.sort_header(side, cx))
            .child(body)
            .into_any_element()
    }

    fn prompt_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let prompt = self.prompt.as_ref()?;
        let cancel = button("p-cancel", "Cancel").on_click(cx.listener(|t, _, w, cx| t.close_prompt(w, cx)));
        let bar = div()
            .px_2()
            .py(px(5.))
            .flex()
            .items_center()
            .gap_2()
            .border_t_1()
            .border_color(c(BORDER))
            .bg(c(ELEVATED))
            .on_key_down(cx.listener(|t, ev: &KeyDownEvent, w, cx| match ev.keystroke.key.as_str() {
                "enter" => {
                    t.confirm_prompt(w, cx);
                    cx.stop_propagation();
                }
                "escape" => {
                    t.close_prompt(w, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }));
        Some(match prompt {
            Prompt::NewFolder(side) => bar
                .child(div().child(format!("New folder in {}:", side.name())))
                .child(div().w(px(240.)).child(self.prompt_input.clone()))
                .child(primary_button("p-ok", "Create").on_click(cx.listener(|t, _, w, cx| t.confirm_prompt(w, cx))))
                .child(cancel)
                .into_any_element(),
            Prompt::Rename(_, old) => bar
                .child(div().child(format!("Rename {old} to:")))
                .child(div().w(px(240.)).child(self.prompt_input.clone()))
                .child(primary_button("p-ok", "Rename").on_click(cx.listener(|t, _, w, cx| t.confirm_prompt(w, cx))))
                .child(cancel)
                .into_any_element(),
            Prompt::Delete(side, names) => bar
                .child(
                    div().text_color(c(RED)).child(if names.len() == 1 {
                        format!("Delete {} permanently from {}?", names[0], side.name())
                    } else {
                        format!("Delete {} items permanently from {}?", names.len(), side.name())
                    }),
                )
                .child(div().flex_1())
                .child(
                    button("p-ok", "Delete")
                        .bg(c(RED))
                        .text_color(c(0xffffff))
                        .on_click(cx.listener(|t, _, w, cx| t.confirm_prompt(w, cx))),
                )
                .child(cancel)
                .into_any_element(),
            Prompt::Conflict { from, names, existing } => {
                let (from, names, existing) = (*from, names.clone(), existing.clone());
                let (n2, e2) = (names.clone(), existing.clone());
                let preview: Vec<String> = existing.iter().take(3).cloned().collect();
                bar.child(div().child(format!(
                    "{} already exist{} in {}: {}{}",
                    existing.len(),
                    if existing.len() == 1 { "s" } else { "" },
                    from.other().name(),
                    preview.join(", "),
                    if existing.len() > 3 { ", ..." } else { "" }
                )))
                .child(div().flex_1())
                .child(primary_button("p-over", "Overwrite").on_click(cx.listener(move |t, _, w, cx| {
                    let n = names.clone();
                    t.close_prompt(w, cx);
                    t.run_transfer(from, n, cx);
                })))
                .child(button("p-skip", "Skip existing").on_click(cx.listener(move |t, _, w, cx| {
                    let n: Vec<String> = n2.iter().filter(|x| !e2.contains(x)).cloned().collect();
                    t.close_prompt(w, cx);
                    t.run_transfer(from, n, cx);
                })))
                .child(cancel)
                .into_any_element()
            }
        })
    }

    fn transfer_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.transfers.is_empty() {
            return None;
        }
        let rows = self.transfers.iter().map(|tr| {
            let frac = tr.progress.fraction();
            let running = matches!(tr.state, TState::Running);
            let id = tr.id;
            let status: (String, u32) = match &tr.state {
                TState::Running => (
                    format!(
                        "{:.0}%  {}/{}  {}/s",
                        frac * 100.0,
                        human_size(tr.progress.done.load(Ordering::Relaxed)),
                        human_size(tr.progress.total.load(Ordering::Relaxed)),
                        human_size(tr.speed as u64)
                    ),
                    MUTED,
                ),
                TState::Done => ("Done".into(), GREEN),
                TState::Cancelled => ("Cancelled".into(), YELLOW),
                TState::Failed(e) => (e.clone(), RED),
            };
            let bar_color = match tr.state {
                TState::Failed(_) => RED,
                TState::Done => GREEN,
                _ => ACCENT,
            };
            div()
                .h(px(24.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(icon(if tr.up { "upload" } else { "download" }, 13., ACCENT_TEXT))
                .child(div().w(px(160.)).overflow_hidden().child(div().whitespace_nowrap().child(tr.label.clone())))
                .child(
                    div()
                        .w(px(140.))
                        .h(px(6.))
                        .flex_none()
                        .bg(c(ACTIVE))
                        .child(div().h_full().w(gpui::relative(if matches!(tr.state, TState::Done) { 1.0 } else { frac })).bg(c(bar_color))),
                )
                .child(div().flex_1().min_w_0().overflow_hidden().text_color(c(status.1)).child(div().whitespace_nowrap().child(status.0)))
                .child(
                    icon_button(("t-x", id as usize), "x", if running { "Cancel" } else { "Dismiss" }, MUTED).on_click(cx.listener(
                        move |t, _, _, cx| {
                            if let Some(tr) = t.transfers.iter().find(|x| x.id == id) {
                                if matches!(tr.state, TState::Running) {
                                    tr.progress.cancel();
                                } else {
                                    t.transfers.retain(|x| x.id != id);
                                }
                            }
                            cx.notify();
                        },
                    )),
                )
        });
        Some(
            div()
                .border_t_1()
                .border_color(c(BORDER))
                .bg(c(PANEL))
                .children(rows)
                .into_any_element(),
        )
    }
}

enum FsOp {
    Mkdir(String),
    Rename(String, String),
    Delete(Vec<String>),
}

impl Focusable for SftpView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.pane(self.active).focus.clone()
    }
}

impl Render for SftpView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mid = div()
            .w(px(30.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .bg(c(PANEL))
            .child(
                icon_button("xfer-up", "arrow-right", "Upload selected to remote (F5)", ACCENT_TEXT)
                    .on_click(cx.listener(|t, _, _, cx| t.begin_transfer(Side::Local, cx))),
            )
            .child(
                icon_button("xfer-down", "arrow-left", "Download selected to local (F5)", ACCENT_TEXT)
                    .on_click(cx.listener(|t, _, _, cx| t.begin_transfer(Side::Remote, cx))),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.pane_view(Side::Local, cx))
                    .child(mid)
                    .child(self.pane_view(Side::Remote, cx)),
            )
            .children(self.prompt_bar(cx))
            .children(self.transfer_bar(cx))
    }
}
