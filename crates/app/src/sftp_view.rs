//! Two-pane SFTP browser: local on the left, remote on the right.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use ssh_core::config::Host;
use ssh_core::sftp::{self, Entry, Progress, Sftp, local};

use crate::anim::{fade_in, pop_in};
use crate::input::{Hide, TextInput};
use crate::rsync;
use crate::runtime;
use crate::theme::*;
use crate::ui::{button, icon, icon_button, primary_button};

/// Which pane. Each pane can browse the local disk or any connected server.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Side::Left => "a",
            Side::Right => "b",
        }
    }
}

#[derive(Clone)]
pub struct RemoteRef {
    pub label: String,
    pub host: Host,
    pub sftp: Arc<Sftp>,
}

#[derive(Clone)]
pub enum Source {
    Local,
    Remote(RemoteRef),
}

impl Source {
    fn label(&self) -> String {
        match self {
            Source::Local => "Local".into(),
            Source::Remote(r) => r.label.clone(),
        }
    }

    fn join(&self, dir: &str, name: &str) -> String {
        match self {
            Source::Local => PathBuf::from(dir).join(name).to_string_lossy().into_owned(),
            Source::Remote(_) => sftp::join(dir, name),
        }
    }

    fn parent(&self, path: &str) -> String {
        match self {
            Source::Remote(_) => sftp::parent(path),
            Source::Local => std::path::Path::new(path)
                .parent()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".into()),
        }
    }

    fn endpoint(&self, dir: &str) -> rsync::Endpoint {
        rsync::Endpoint {
            at: match self {
                Source::Local => rsync::Where::Local,
                Source::Remote(r) => rsync::Where::Remote {
                    user: r.host.user.clone(),
                    host: r.host.host.clone(),
                    port: r.host.port,
                    key: match &r.host.auth {
                        ssh_core::config::AuthMethod::Key { path } => {
                            Some(path.to_string_lossy().into_owned())
                        }
                        ssh_core::config::AuthMethod::Password => None,
                    },
                },
            },
            dir: dir.to_string(),
        }
    }
}

impl Source {
    /// True when both sources are the same file system (copying between them can
    /// overwrite the very files being read).
    fn same_fs(&self, other: &Source) -> bool {
        match (self, other) {
            (Source::Local, Source::Local) => true,
            (Source::Remote(a), Source::Remote(b)) => a.host.account() == b.host.account(),
            _ => false,
        }
    }
}

/// Why a copy must be refused: onto itself, or a folder into its own subtree.
fn self_copy_error(
    same_fs: bool,
    src_dir: &str,
    dst_dir: &str,
    names: &[String],
) -> Option<String> {
    if !same_fs {
        return None;
    }
    let (s, d) = (std::path::Path::new(src_dir), std::path::Path::new(dst_dir));
    if s == d {
        return Some("Source and destination are the same folder.".into());
    }
    names
        .iter()
        .find(|n| d.starts_with(s.join(n)))
        .map(|n| format!("Cannot copy {n} into itself."))
}

/// The view asks its owner to connect a host and hand back a `Source`.
pub enum SftpEvent {
    /// `None` selects the local disk, otherwise the host's account string.
    PickHost { side: Side, account: Option<String> },
}

impl EventEmitter<SftpEvent> for SftpView {}

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
    source: Source,
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
    /// Entries left out because they match a secret-file pattern.
    secret_hidden: usize,
    nav: u64,
    typed: (String, Instant),
}

impl Pane {
    fn new(source: Source, cx: &mut Context<SftpView>) -> Self {
        Self {
            source,
            path: String::new(),
            input: {
                let input = cx.new(|cx| TextInput::new("path", false, cx));
                input.update(cx, |i, _| i.set_hide_mode(Hide::Redact));
                input
            },
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
            hidden: crate::prefs::get().show_dotfiles(),
            secret_hidden: 0,
            nav: 0,
            typed: (String::new(), Instant::now()),
        }
    }

    /// Rebuild the visible list: hidden filter, then sort (folders always first).
    fn rebuild(&mut self) {
        self.secret_hidden = self
            .raw
            .iter()
            .filter(|e| {
                (self.hidden || !e.name.starts_with('.')) && crate::privacy::file_hidden(&e.name)
            })
            .count();
        let mut v: Vec<Entry> = self
            .raw
            .iter()
            .filter(|e| {
                (self.hidden || !e.name.starts_with('.')) && !crate::privacy::file_hidden(&e.name)
            })
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
        self.selected
            .retain(|n| self.entries.iter().any(|e| &e.name == n));
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
    Conflict {
        from: Side,
        names: Vec<String>,
        existing: Vec<String>,
    },
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
    route: String,
    progress: Arc<Progress>,
    state: TState,
    finished: Option<Instant>,
    last_done: u64,
    speed: f64,
    rsync: String,
    /// An endpoint uses password auth, so rsync will prompt.
    rsync_prompts: bool,
}

pub struct SftpView {
    left: Pane,
    right: Pane,
    /// (account, display name) of every configured host.
    hosts: Vec<(String, String)>,
    menu: Option<Side>,
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
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn fmt_time(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => String::new(),
    }
}

async fn fetch(source: Source, path: String) -> Result<(String, Vec<Entry>), ssh_core::Error> {
    match source {
        Source::Local => {
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
        Source::Remote(r) => {
            runtime::spawn(async move {
                let sftp = r.sftp;
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
    pub fn new(
        left: Source,
        right: Source,
        hosts: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = Self {
            left: Pane::new(left, cx),
            right: Pane::new(right, cx),
            hosts,
            menu: None,
            active: Side::Right,
            prompt: None,
            prompt_input: cx.new(|cx| TextInput::new("name", false, cx)),
            transfers: vec![],
            next_id: 1,
        };
        // Speed and auto-dismiss ticker.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
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
                            (TState::Done, Some(f)) => {
                                now.duration_since(f) < Duration::from_secs(6)
                            }
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
        this
    }

    /// Start browsing both panes at their home directories.
    /// Settings changed (secret-file patterns, privacy switch): re-filter both lists.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        self.left.rebuild();
        self.right.rebuild();
        cx.notify();
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.go(Side::Left, "~".into(), false, cx);
        self.go(Side::Right, "~".into(), false, cx);
    }

    /// Point a pane at a different local/remote source and open its home.
    pub fn set_source(&mut self, side: Side, source: Source, cx: &mut Context<Self>) {
        let p = self.pane_mut(side);
        p.source = source;
        p.path.clear();
        p.history.clear();
        p.raw.clear();
        p.entries.clear();
        p.selected.clear();
        p.error = None;
        self.menu = None;
        self.go(side, "~".into(), false, cx);
    }

    fn pane(&self, s: Side) -> &Pane {
        match s {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn pane_mut(&mut self, s: Side) -> &mut Pane {
        match s {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    // ---------- navigation ----------

    fn go(&mut self, side: Side, path: String, push_history: bool, cx: &mut Context<Self>) {
        let p = self.pane_mut(side);
        let source = p.source.clone();
        p.nav += 1;
        let nav = p.nav;
        p.loading = true;
        let prev = p.path.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = fetch(source, path).await;
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
        let parent = p.source.parent(&p.path);
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
            let target = p.source.join(&p.path, &e.name);
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
        let Some(name) = p.entries.get(ix).map(|e| e.name.clone()) else {
            return;
        };
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
        self.ensure_visible(side, next);
    }

    /// Scroll just enough to show row `ix` (no jump when it is already visible).
    fn ensure_visible(&self, side: Side, ix: usize) {
        let h = &self.pane(side).scroll;
        let (top, rows) = {
            let st = h.0.borrow();
            let row_h = px(22.);
            let top = (-st.base_handle.offset().y / row_h).floor().max(0.0) as usize;
            let view = st.last_item_size.map(|s| s.item.height).unwrap_or(px(440.));
            (top, ((view / row_h) as usize).max(1))
        };
        if ix < top {
            h.scroll_to_item(ix, ScrollStrategy::Top);
        } else if ix >= top + rows {
            h.scroll_to_item(ix, ScrollStrategy::Bottom);
        }
    }

    fn on_key(
        &mut self,
        side: Side,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                        if let Some(ix) = p
                            .entries
                            .iter()
                            .position(|e| e.name.to_lowercase().starts_with(&q))
                        {
                            self.select_row(side, ix, false, false);
                            self.pane(side)
                                .scroll
                                .scroll_to_item(ix, ScrollStrategy::Center);
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

    fn open_prompt(
        &mut self,
        prompt: Prompt,
        initial: &str,
        focus_input: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            Some(Prompt::NewFolder(side)) if !value.is_empty() => {
                self.fs_op(side, FsOp::Mkdir(value), cx)
            }
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
        let source = self.pane(side).source.clone();
        let dir = self.pane(side).path.clone();
        cx.spawn(async move |this, cx| {
            let r: Result<(), ssh_core::Error> = match source {
                Source::Remote(rr) => {
                    let sftp = rr.sftp;
                    runtime::spawn(async move {
                        match op {
                            FsOp::Mkdir(n) => sftp.mkdir(&sftp::join(&dir, &n)).await,
                            FsOp::Rename(a, b) => {
                                sftp.rename(&sftp::join(&dir, &a), &sftp::join(&dir, &b))
                                    .await
                            }
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
                Source::Local => {
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
        let dest = self.pane(to).source.clone();
        if let Some(msg) = self_copy_error(
            self.pane(from).source.same_fs(&dest),
            &self.pane(from).path,
            &dest_dir,
            &names,
        ) {
            self.pane_mut(to).error = Some(msg.into());
            cx.notify();
            return;
        }
        let check = names.clone();
        cx.spawn(async move |this, cx| {
            // Which names already exist at the destination?
            let existing: Vec<String> = match dest {
                Source::Local => check
                    .iter()
                    .filter(|n| PathBuf::from(&dest_dir).join(n).exists())
                    .cloned()
                    .collect(),
                Source::Remote(r) => {
                    let d = dest_dir.clone();
                    let c = check.clone();
                    runtime::spawn(async move {
                        let mut v = vec![];
                        for n in c {
                            if r.sftp.exists(&sftp::join(&d, &n)).await.unwrap_or(false) {
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
                    t.prompt = Some(Prompt::Conflict {
                        from,
                        names,
                        existing,
                    });
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
        let to = from.other();
        let src_dir = self.pane(from).path.clone();
        let dst_dir = self.pane(to).path.clone();
        let from_src = self.pane(from).source.clone();
        let to_src = self.pane(to).source.clone();
        let rsync_cmd = rsync::command(
            &from_src.endpoint(&src_dir),
            &names,
            &to_src.endpoint(&dst_dir),
        );
        let progress = Progress::new();
        let id = self.next_id;
        self.next_id += 1;
        let label = if names.len() == 1 {
            names[0].clone()
        } else {
            format!("{} items", names.len())
        };
        self.transfers.push(Transfer {
            id,
            label,
            route: format!("{} to {}", from_src.label(), to_src.label()),
            progress: progress.clone(),
            state: TState::Running,
            finished: None,
            last_done: 0,
            speed: 0.0,
            rsync: rsync_cmd,
            rsync_prompts: [&from_src, &to_src].iter().any(|s| {
                matches!(s, Source::Remote(r) if matches!(r.host.auth, ssh_core::config::AuthMethod::Password))
            }),
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = runtime::spawn(async move {
                // Count the whole selection first so the percentage and the "use rsync"
                // hint are right from the start.
                let (mut bytes, mut files) = (0u64, 0u64);
                for n in &names {
                    let sp = from_src.join(&src_dir, n);
                    let (b, f) = match &from_src {
                        Source::Remote(r) => r.sftp.scan(&sp).await?,
                        Source::Local => {
                            let p = PathBuf::from(sp);
                            tokio::task::spawn_blocking(move || local::scan(&p))
                                .await
                                .map_err(|e| ssh_core::Error::Sftp(e.to_string()))??
                        }
                    };
                    bytes += b;
                    files += f;
                }
                progress.set_prescan(bytes, files);
                for n in names {
                    let (sp, dp) = (from_src.join(&src_dir, &n), to_src.join(&dst_dir, &n));
                    match (&from_src, &to_src) {
                        (Source::Local, Source::Remote(r)) => {
                            r.sftp.upload(&PathBuf::from(&sp), &dp, &progress).await?
                        }
                        (Source::Remote(r), Source::Local) => {
                            r.sftp.download(&sp, &PathBuf::from(&dp), &progress).await?
                        }
                        (Source::Remote(a), Source::Remote(b)) => {
                            a.sftp.copy_to(&sp, &b.sftp, &dp, &progress).await?
                        }
                        (Source::Local, Source::Local) => {
                            let p = progress.clone();
                            tokio::task::spawn_blocking(move || {
                                local::copy(&PathBuf::from(sp), &PathBuf::from(dp), &p)
                            })
                            .await
                            .map_err(|e| ssh_core::Error::Sftp(e.to_string()))??;
                        }
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

    fn render_rows(
        &mut self,
        side: Side,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let p = self.pane(side);
        let active = self.active == side;
        range
            .filter_map(|ix| p.entries.get(ix).map(|e| (ix, e.clone())))
            .map(|(ix, e)| {
                let selected = p.selected.contains(&e.name);
                let is_cursor = active && p.cursor == ix;
                let drag_names = if selected {
                    p.selected_names()
                } else {
                    vec![e.name.clone()]
                };
                let drag_label: SharedString = if drag_names.len() == 1 {
                    drag_names[0].clone().into()
                } else {
                    format!("{} items", drag_names.len()).into()
                };
                div()
                    .id((side.tag(), ix))
                    .w_full()
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(selected, |d| d.bg(c(SELECTED_ROW)))
                    .border_l_2()
                    .border_color(c(if is_cursor { ACCENT } else { 0x00000000 }))
                    .hover(|s| s.bg(c(HOVER)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |t, ev: &MouseDownEvent, window, cx| {
                            t.active = side;
                            t.menu = None;
                            window.focus(&t.pane(side).focus);
                            let m = ev.modifiers;
                            let already = t.pane(side).selected.iter().len() > 1
                                && t.pane(side)
                                    .entries
                                    .get(ix)
                                    .is_some_and(|e| t.pane(side).selected.contains(&e.name));
                            if !(already && !m.shift && !m.platform) {
                                t.select_row(side, ix, m.shift, m.platform);
                            }
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |t, ev: &ClickEvent, _, cx| {
                        if ev.click_count() >= 2 {
                            t.open_dir(side, ix, cx);
                        } else if ev.modifiers() == Default::default()
                            && t.pane(side).selected.len() > 1
                        {
                            // Plain click inside a multi-selection collapses it (after drag could start).
                            t.select_row(side, ix, false, false);
                            cx.notify();
                        }
                    }))
                    .on_drag(
                        DragEntries {
                            from: side,
                            names: drag_names,
                        },
                        move |_, _, _, cx| cx.new(|_| DragLabel(drag_label.clone())),
                    )
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
                            .child(if e.is_dir {
                                String::new()
                            } else {
                                human_size(e.size)
                            }),
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
                .when(active, |d| {
                    d.child(icon(
                        if p.asc { "chevron-up" } else { "chevron-down" },
                        10.,
                        TEXT,
                    ))
                })
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
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(cell("h-size", "Size", SortKey::Size)),
            )
            .child(div().w(px(104.)).flex_none().child(cell(
                "h-mod",
                "Modified",
                SortKey::Modified,
            )))
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
            .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
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
                }
            }))
            .child(
                icon_button(
                    (side.tag(), 1000usize),
                    "arrow-left",
                    "Back",
                    if can_back { MUTED } else { ACTIVE },
                )
                .on_click(cx.listener(move |t, _, _, cx| t.back(side, cx))),
            )
            .child(
                icon_button(
                    (side.tag(), 1001usize),
                    "arrow-up",
                    "Parent folder (⌘↑)",
                    MUTED,
                )
                .on_click(cx.listener(move |t, _, _, cx| t.up(side, cx))),
            )
            .child(
                icon_button((side.tag(), 1002usize), "home", "Home", MUTED)
                    .on_click(cx.listener(move |t, _, _, cx| t.go(side, "~".into(), true, cx))),
            )
            .child(div().flex_1().min_w_0().child(p.input.clone()))
            .child(
                icon_button((side.tag(), 1003usize), "refresh", "Refresh (⌘R)", MUTED)
                    .on_click(cx.listener(move |t, _, _, cx| t.refresh(side, cx))),
            )
            .child(
                icon_button(
                    (side.tag(), 1004usize),
                    if p.hidden { "eye" } else { "eye-off" },
                    if p.hidden {
                        "Hide dotfiles"
                    } else {
                        "Show dotfiles"
                    },
                    MUTED,
                )
                .on_click(cx.listener(move |t, _, _, cx| {
                    let p = t.pane_mut(side);
                    p.hidden = !p.hidden;
                    p.rebuild();
                    cx.notify();
                })),
            )
            .child(
                icon_button(
                    (side.tag(), 1005usize),
                    "folder-plus",
                    "New folder (⇧⌘N)",
                    MUTED,
                )
                .on_click(cx.listener(move |t, _, w, cx| t.ask_new_folder(side, w, cx))),
            );

        let list = uniform_list(
            (side.tag(), 2000usize),
            count,
            cx.processor(move |t: &mut SftpView, range, _, cx| t.render_rows(side, range, cx)),
        )
        .track_scroll(p.scroll.clone())
        .flex_1()
        .min_h_0();

        let body = if p.loading && p.entries.is_empty() {
            div()
                .flex_1()
                .p_3()
                .text_color(c(PLACEHOLDER))
                .child("Loading...")
                .into_any_element()
        } else if count == 0 {
            div()
                .flex_1()
                .p_3()
                .text_color(c(PLACEHOLDER))
                .child("Empty folder")
                .into_any_element()
        } else {
            list.into_any_element()
        };

        div()
            .id((side.tag(), 3000usize))
            .track_focus(&p.focus)
            .on_key_down(cx.listener(move |t, ev: &KeyDownEvent, w, cx| t.on_key(side, ev, w, cx)))
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_2()
            .border_color(c(if active { BORDER_VARIANT } else { 0x00000000 }))
            .drag_over::<DragEntries>(move |s, d, _, _| {
                if d.from != side {
                    s.border_color(c(ACCENT)).bg(c(0xf05a2914))
                } else {
                    s
                }
            })
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
                    .child(
                        div()
                            .id((side.tag(), 4000usize))
                            .h_full()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .px(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(c(HOVER)))
                            .on_click(cx.listener(move |t, _, _, cx| {
                                t.menu = if t.menu == Some(side) {
                                    None
                                } else {
                                    Some(side)
                                };
                                cx.notify();
                            }))
                            .child(icon(
                                if matches!(p.source, Source::Local) {
                                    "folder"
                                } else {
                                    "server"
                                },
                                13.,
                                if active { ACCENT_TEXT } else { MUTED },
                            ))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(p.source.label()),
                            )
                            .child(icon("chevron-down", 10., MUTED)),
                    )
                    .child(div().flex_1())
                    .when(p.loading, |d| {
                        d.child(div().text_color(c(PLACEHOLDER)).child("loading"))
                    })
                    .child(div().text_color(c(PLACEHOLDER)).child(if sel > 0 {
                        format!("{sel} of {count} selected")
                    } else if p.secret_hidden > 0 {
                        format!("{count} items · {} secret hidden", p.secret_hidden)
                    } else {
                        format!("{count} items")
                    })),
            )
            .child(toolbar)
            .when_some(p.error.clone(), |d, e| {
                d.child(
                    div()
                        .px_2()
                        .py(px(3.))
                        .text_color(c(RED))
                        .bg(c(0xe5534b1a))
                        .child(e),
                )
            })
            .child(self.sort_header(side, cx))
            .child(body)
            .children(self.source_menu(side, cx))
            .into_any_element()
    }

    fn source_menu(&self, side: Side, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.menu != Some(side) {
            return None;
        }
        let current = self.pane(side).source.label();
        let mut items: Vec<(Option<String>, String)> = vec![(None, "Local".into())];
        items.extend(
            self.hosts
                .iter()
                .cloned()
                .map(|(acct, name)| (Some(acct), name)),
        );
        let rows = items.into_iter().enumerate().map(|(n, (account, name))| {
            let is_cur = name == current;
            let is_local = account.is_none();
            div()
                .id((side.tag(), 5000 + n))
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(|s| s.bg(c(ACTIVE)))
                .on_click(cx.listener(move |t, _, _, cx| {
                    t.menu = None;
                    cx.emit(SftpEvent::PickHost {
                        side,
                        account: account.clone(),
                    });
                    cx.notify();
                }))
                .child(icon(if is_local { "folder" } else { "server" }, 13., MUTED))
                .child(div().flex_1().child(name))
                .when(is_cur, |d| d.child(icon("check", 12., ACCENT_TEXT)))
        });
        Some(
            pop_in(
                div()
                    .absolute()
                    .occlude()
                    .top(px(22.))
                    .left_0()
                    .w(px(220.))
                    .bg(c(ELEVATED))
                    .border_2()
                    .border_color(c(BORDER))
                    .children(rows),
                (side.tag(), 6000usize),
            )
            .into_any_element(),
        )
    }

    fn prompt_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let prompt = self.prompt.as_ref()?;
        let cancel =
            button("p-cancel", "Cancel").on_click(cx.listener(|t, _, w, cx| t.close_prompt(w, cx)));
        let bar = div()
            .px_2()
            .py(px(5.))
            .flex()
            .items_center()
            .gap_2()
            .border_t_1()
            .border_color(c(BORDER))
            .bg(c(ELEVATED))
            .on_key_down(cx.listener(
                |t, ev: &KeyDownEvent, w, cx| match ev.keystroke.key.as_str() {
                    "enter" => {
                        t.confirm_prompt(w, cx);
                        cx.stop_propagation();
                    }
                    "escape" => {
                        t.close_prompt(w, cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                },
            ));
        Some(match prompt {
            Prompt::NewFolder(side) => bar
                .child(div().child(format!(
                    "New folder in {}:",
                    self.pane(*side).source.label()
                )))
                .child(div().w(px(240.)).child(self.prompt_input.clone()))
                .child(
                    primary_button("p-ok", "Create")
                        .on_click(cx.listener(|t, _, w, cx| t.confirm_prompt(w, cx))),
                )
                .child(cancel)
                .into_any_element(),
            Prompt::Rename(_, old) => bar
                .child(div().child(format!("Rename {old} to:")))
                .child(div().w(px(240.)).child(self.prompt_input.clone()))
                .child(
                    primary_button("p-ok", "Rename")
                        .on_click(cx.listener(|t, _, w, cx| t.confirm_prompt(w, cx))),
                )
                .child(cancel)
                .into_any_element(),
            Prompt::Delete(side, names) => bar
                .child(div().text_color(c(RED)).child(if names.len() == 1 {
                    format!(
                        "Delete {} permanently from {}?",
                        names[0],
                        self.pane(*side).source.label()
                    )
                } else {
                    format!(
                        "Delete {} items permanently from {}?",
                        names.len(),
                        self.pane(*side).source.label()
                    )
                }))
                .child(div().flex_1())
                .child(
                    button("p-ok", "Delete")
                        .bg(c(RED))
                        .text_color(c(0xffffff))
                        .on_click(cx.listener(|t, _, w, cx| t.confirm_prompt(w, cx))),
                )
                .child(cancel)
                .into_any_element(),
            Prompt::Conflict {
                from,
                names,
                existing,
            } => {
                let (from, names, existing) = (*from, names.clone(), existing.clone());
                let (n2, e2) = (names.clone(), existing.clone());
                let preview: Vec<String> = existing.iter().take(3).cloned().collect();
                bar.child(div().child(format!(
                    "{} already exist{} in {}: {}{}",
                    existing.len(),
                    if existing.len() == 1 { "s" } else { "" },
                    self.pane(from.other()).source.label(),
                    preview.join(", "),
                    if existing.len() > 3 { ", ..." } else { "" }
                )))
                .child(div().flex_1())
                .child(primary_button("p-over", "Overwrite").on_click(cx.listener(
                    move |t, _, w, cx| {
                        let n = names.clone();
                        t.close_prompt(w, cx);
                        t.run_transfer(from, n, cx);
                    },
                )))
                .child(button("p-skip", "Skip existing").on_click(cx.listener(
                    move |t, _, w, cx| {
                        let n: Vec<String> =
                            n2.iter().filter(|x| !e2.contains(x)).cloned().collect();
                        t.close_prompt(w, cx);
                        t.run_transfer(from, n, cx);
                    },
                )))
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
            let files = tr.progress.files.load(Ordering::Relaxed);
            let total = tr.progress.total.load(Ordering::Relaxed);
            // Past this size rsync (delta, compression, resume) beats per-file SFTP.
            let prefs = crate::prefs::get();
            let big = files >= prefs.rsync_hint_files() as u64
                || total >= (prefs.rsync_hint_mb() as u64) << 20;
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
            let row = div()
                .h(px(24.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(icon("arrow-right", 13., ACCENT_TEXT))
                .child(
                    div()
                        .w(px(150.))
                        .flex_none()
                        .overflow_hidden()
                        .child(div().whitespace_nowrap().child(tr.label.clone())),
                )
                .child(
                    div()
                        .w(px(130.))
                        .flex_none()
                        .overflow_hidden()
                        .text_color(c(PLACEHOLDER))
                        .child(div().whitespace_nowrap().child(tr.route.clone())),
                )
                .child(
                    div().w(px(140.)).h(px(6.)).flex_none().bg(c(ACTIVE)).child(
                        div()
                            .h_full()
                            .w(gpui::relative(if matches!(tr.state, TState::Done) {
                                1.0
                            } else {
                                frac
                            }))
                            .bg(c(bar_color)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_color(c(status.1))
                        .child(div().whitespace_nowrap().child(status.0)),
                )
                .child(
                    icon_button(
                        ("t-x", id as usize),
                        "x",
                        if running { "Cancel" } else { "Dismiss" },
                        MUTED,
                    )
                    .on_click(cx.listener(move |t, _, _, cx| {
                        if let Some(tr) = t.transfers.iter().find(|x| x.id == id) {
                            if matches!(tr.state, TState::Running) {
                                tr.progress.cancel();
                            } else {
                                t.transfers.retain(|x| x.id != id);
                            }
                        }
                        cx.notify();
                    })),
                );
            let hint = (big && running).then(|| {
                let cmd = tr.rsync.clone();
                div()
                    .px_2()
                    .pb(px(5.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(c(YELLOW))
                    .child(format!(
                        "Large transfer ({} files, {}): rsync is faster (needs rsync on the server){}.",
                        files,
                        human_size(total),
                        if tr.rsync_prompts {
                            ". It will ask for the password"
                        } else {
                            ""
                        }
                    ))
                    .child(
                        button(("t-rsync", id as usize), "Copy rsync command").on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(cmd.clone()))
                            }),
                        ),
                    )
            });
            fade_in(
                div().flex().flex_col().child(row).children(hint),
                ("xfer-row", id as usize),
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
                icon_button(
                    "xfer-up",
                    "arrow-right",
                    "Copy selected to the right pane (F5)",
                    ACCENT_TEXT,
                )
                .on_click(cx.listener(|t, _, _, cx| t.begin_transfer(Side::Left, cx))),
            )
            .child(
                icon_button(
                    "xfer-down",
                    "arrow-left",
                    "Copy selected to the left pane (F5)",
                    ACCENT_TEXT,
                )
                .on_click(cx.listener(|t, _, _, cx| t.begin_transfer(Side::Right, cx))),
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
                    .child(self.pane_view(Side::Left, cx))
                    .child(mid)
                    .child(self.pane_view(Side::Right, cx)),
            )
            .children(self.prompt_bar(cx))
            .children(self.transfer_bar(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn refuses_same_folder_and_self_nesting() {
        assert!(self_copy_error(true, "/a/b", "/a/b", &n(&["f"])).is_some());
        assert!(self_copy_error(true, "/a/b/", "/a/b", &n(&["f"])).is_some());
        assert!(self_copy_error(true, "/a", "/a/dir", &n(&["dir"])).is_some());
        assert!(self_copy_error(true, "/a", "/a/dir/deeper", &n(&["dir"])).is_some());
    }

    #[test]
    fn allows_normal_copies() {
        assert!(self_copy_error(true, "/a", "/b", &n(&["x"])).is_none());
        // Prefix of the name but a different folder.
        assert!(self_copy_error(true, "/a", "/a/dirty", &n(&["dir"])).is_none());
        // Different file systems can use identical paths.
        assert!(self_copy_error(false, "/a", "/a", &n(&["x"])).is_none());
    }
}
