//! Two-pane SFTP browser: local on the left, remote on the right.

mod edit;
mod file_ops;
mod navigation;
mod panes;
#[cfg(test)]
mod tests;
mod transfers;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, Render, ScrollStrategy, SharedString,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use ssh_core::config::Host;
use ssh_core::sftp::{self, Entry, Progress, Sftp, local};

use crate::rsync;
use crate::runtime;
use crate::theme::*;
use crate::widgets::anim::{fade_in, pop_in};
use crate::widgets::input::{Hide, TextInput};
use crate::widgets::{button, icon, icon_button, keyboard, primary_button};

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
    /// The SSH connection behind `sftp`, for running rsync over it.
    #[cfg_attr(not(unix), expect(dead_code, reason = "only the rsync runner uses it"))]
    pub conn: Arc<ssh_core::session::Connection>,
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
                        ssh_core::config::AuthMethod::Password
                        | ssh_core::config::AuthMethod::Agent => None,
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
            // Reachable with Tab.
            focus: cx.focus_handle().tab_stop(true),
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
    /// Waiting for a free slot (see the "Parallel transfers" setting).
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

/// Everything needed to start a transfer later.
struct Job {
    to: Side,
    src_dir: String,
    dst_dir: String,
    names: Vec<String>,
    from_src: Source,
    to_src: Source,
}

struct Transfer {
    id: u64,
    job: Option<Job>,
    label: String,
    route: String,
    progress: Arc<Progress>,
    state: TState,
    finished: Option<Instant>,
    last_done: u64,
    speed: f64,
    /// `None` when the names can't be put in a command safely.
    rsync: Option<String>,
    /// Both ends are servers, so the command forwards the ssh agent to the source one.
    rsync_agent: bool,
    /// An endpoint uses password auth, so rsync will prompt.
    rsync_prompts: bool,
    /// Running with the built-in rsync instead of SFTP.
    via_rsync: Arc<AtomicBool>,
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
    edits: Vec<Edit>,
}

enum EState {
    Downloading,
    /// In sync with the server.
    Synced,
    /// Saved locally, waiting for an upload (auto-upload is off).
    Modified,
    Uploading,
    Failed(String),
}

/// A server file opened in a local editor and watched for saves.
struct Edit {
    id: u64,
    name: String,
    remote: RemoteRef,
    remote_path: String,
    local: PathBuf,
    /// Private temp folder holding `local`, removed when the edit stops.
    dir: PathBuf,
    /// The download finished, so `local` is the real file and safe to upload.
    downloaded: bool,
    /// Modification time last synced, to spot a save.
    mtime: Option<SystemTime>,
    state: EState,
    synced_at: Option<chrono::DateTime<chrono::Local>>,
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
            edits: vec![],
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
                        t.poll_edits(cx);
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
        // A new speed limit also applies to transfers already running.
        let limit = crate::prefs::get().transfer_limit_kbps() as u64 * 1024;
        for tr in &self.transfers {
            tr.progress.limit.store(limit, Ordering::Relaxed);
        }
        self.left.rebuild();
        self.right.rebuild();
        self.pump(cx);
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
}

/// Delete an edit's temp folder, but only ever something inside our own temp root.
fn remove_edit_dir(dir: &std::path::Path) {
    if dir.starts_with(std::env::temp_dir().join("wiz-edit")) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

impl Drop for SftpView {
    fn drop(&mut self) {
        for e in &self.edits {
            remove_edit_dir(&e.dir);
        }
    }
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
                    accent_text(),
                )
                .on_click(cx.listener(|t, _, _, cx| t.begin_transfer(Side::Left, cx))),
            )
            .child(
                icon_button(
                    "xfer-down",
                    "arrow-left",
                    "Copy selected to the left pane (F5)",
                    accent_text(),
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
            .children(self.edit_bar(cx))
            .children(self.transfer_bar(cx))
    }
}
