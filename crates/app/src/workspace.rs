//! The window's root view: tabs, pages and dialogs. Its types live here so every part can
//! reach their fields; the behavior is split by concern into the submodules.

mod connect;
mod credentials;
mod focus;
mod help;
mod home;
mod hosts;
mod latency;
mod overlays;
mod panes;
mod reconnect;
mod sessions;
mod snippets;
mod split;
mod tab_menu;
mod tabs;
#[cfg(test)]
mod tests;
mod windows;

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight,
    IntoElement, Pixels, Render, SharedString, Stateful, Window, actions, div, prelude::*, px,
};
use ssh_core::config::{AuthMethod, Config, Host};
use ssh_core::docker::{self, Backend};
use ssh_core::hostkey::{HostKeyVerifier, TrustPrompt};
use ssh_core::secrets::{self, Kind, Secret, Store};
use ssh_core::session::{AuthPrompt, Connection, Credentials, Options, Question};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::dialogs::code::{CodeDialog, CodeEvent};
use crate::dialogs::host_form::{FormEvent, HostForm, Start};
use crate::dialogs::quick_connect::{QuickConnect as QuickDialog, QuickEvent};
use crate::dialogs::snippets::SnippetPicker;
use crate::dialogs::unlock::{UnlockDialog, UnlockEvent, UnlockMode};
use crate::privacy;
use crate::runtime;
use crate::theme::*;
use crate::views::docker::{DockerEvent, DockerView};
use crate::views::logs::LogView;
use crate::views::settings::{SettingsEvent, SettingsView};
use crate::views::sftp::{RemoteRef, SftpEvent, SftpView, Side as SftpSide, Source as SftpSource};
use crate::views::terminal::{TerminalEvent, TerminalView};
use crate::widgets::anim::{Spring, fade_in, page_in, pop_in, stagger_in};
use crate::widgets::input::{Hide, InputEvent, TextInput};
use crate::widgets::{FocusNext, FocusPrev, button, icon, icon_button, keyboard, primary_button};

actions!(
    app,
    [
        Quit,
        CloseTab,
        NextTab,
        PrevTab,
        NewHost,
        OpenSettings,
        SplitPane,
        SplitMenu,
        TabMenu,
        QuickConnect,
        Snippets,
        FocusTabs,
        NextPane,
        PrevPane,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        ShowHelp
    ]
);

/// Select the tab at this index: Cmd+1 to Cmd+9 (Ctrl on Windows).
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = app, no_json)]
pub struct SelectTab(pub usize);

struct TrustRequest {
    /// `Pending::id` of the connect asking, so cancelling it closes this question.
    attempt: u64,
    host: String,
    port: u16,
    fingerprint: String,
    /// The fingerprint remembered before, when the host is known but presented a
    /// different key.
    previous: Option<String>,
    reply: std::sync::mpsc::SyncSender<bool>,
}

/// Called from the tokio side during the handshake. Blocks that worker until the
/// user answers in the UI. `prompting` is set meanwhile, so the connect timeout pauses
/// while the user reads the fingerprint.
struct GuiPrompt {
    attempt: u64,
    tx: UnboundedSender<TrustRequest>,
    prompting: Arc<AtomicBool>,
}

/// Questions a server asks during login (a one-time code), waiting for the user.
struct CodeRequest {
    /// `Pending::id` of the connect asking (it may be for a host behind the one asking).
    attempt: u64,
    /// Name of the saved host asking.
    host: String,
    text: String,
    questions: Vec<Question>,
    reply: std::sync::mpsc::SyncSender<Option<Vec<String>>>,
}

/// Asks the user for one-time codes, like `GuiPrompt` asks about host keys: blocks a
/// worker until answered, with the connect timeout paused meanwhile.
struct GuiAsk {
    attempt: u64,
    tx: UnboundedSender<CodeRequest>,
    prompting: Arc<AtomicBool>,
}

/// A connection attempt still in flight. Dropping `_cancel` aborts it.
struct Pending {
    id: u64,
    account: String,
    name: String,
    host: String,
    port: u16,
    /// More opens clicked while this connect was running (a second shell, files...).
    queued: Vec<Open>,
    _cancel: oneshot::Sender<()>,
}

/// One shell of a terminal tab, with the connection it runs on: the panes of a split may
/// be on different servers.
struct Pane {
    view: Entity<TerminalView>,
    conn: Arc<Connection>,
    /// Which host `conn` belongs to, so a lost terminal can reconnect.
    account: String,
    password: Option<Secret>,
}

enum PaneView {
    Shell(Pane),
    View(Box<Tab>),
}

/// A terminal tab: one shell, or several split side by side.
struct TermTab {
    panes: Vec<PaneView>,
    /// Pane that had focus last; gets it back when the tab is selected.
    focused: usize,
    /// Pane indices per column, left to right, each column top to bottom. Picked from the
    /// space available on every render (see `layout`).
    layout: Vec<Vec<usize>>,
    /// Width of each column as a share of the tab, summing to 1.
    cols: Vec<Spring>,
    /// Height of each pane in its column as a share of the column, summing to 1.
    rows: Vec<Vec<Spring>>,
    ticked: Option<Instant>,
}

/// A split divider being dragged, and where the mouse was last (x for a column divider,
/// y for a row divider).
struct PaneDrag {
    divider: Divider,
    last: Pixels,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Divider {
    /// Between column `n` and `n + 1`.
    Col(usize),
    /// Between rows `n` and `n + 1` of column `col`.
    Row { col: usize, n: usize },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Settings,
    /// `tabs[active]`.
    Tab,
}

struct Live {
    conn: Arc<Connection>,
    password: Option<Secret>,
}

enum Open {
    Shell,
    Docker,
    Sftp,
    /// Connect a host and attach it to one pane of an open SFTP tab.
    SftpSource(gpui::WeakEntity<SftpView>, SftpSide),
    /// Reconnect a terminal whose connection dropped (attempt number). True when the user
    /// asked for it; only then may login questions (a one-time code) be put to them.
    Reattach(gpui::WeakEntity<TerminalView>, u32, bool),
    /// Connect a host and open a shell for it as a new pane of the terminal tab with this id.
    SplitShell(u64),
}

enum TabKind {
    Terminal(TermTab),
    Docker(Entity<DockerView>),
    Sftp(Entity<SftpView>),
    Logs(Entity<LogView>),
}

struct Tab {
    /// Stable across reordering and closing, unlike the index.
    id: u64,
    label: SharedString,
    kind: TabKind,
    /// Pinned tabs come first and have no close button.
    pinned: bool,
    /// Name given with Rename in the tab menu, shown instead of the label and the
    /// terminal title. Kept only while the tab is open.
    name: Option<SharedString>,
    /// Keyboard focus on the tab strip, moved with the arrow keys.
    focus: FocusHandle,
    /// Account of the host a Docker or files tab was opened for, so the hosts page can
    /// list it. Terminals go by their panes instead.
    account: Option<String>,
}

/// Runs once the secrets file is unlocked.
type AfterUnlock = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

pub struct Workspace {
    cfg: Config,
    /// Hosts opened with quick connect (not saved), by account, with the credentials typed
    /// for them, so a dropped terminal can reconnect.
    quick: HashMap<String, (Host, Credentials)>,
    quick_form: Option<Entity<QuickDialog>>,
    snippet_picker: Option<Entity<SnippetPicker>>,
    /// Master password dialog, and what to do once it succeeds.
    unlock: Option<Entity<UnlockDialog>>,
    after_unlock: Option<AfterUnlock>,
    cfg_path: PathBuf,
    conns: HashMap<String, Live>,
    tabs: Vec<Tab>,
    active: usize,
    /// Right-click menu of a tab: its id, and where the mouse was.
    tab_menu: Option<(u64, gpui::Point<Pixels>)>,
    /// The menu of the split button, where the mouse was: pick the server for a new pane.
    split_menu: Option<gpui::Point<Pixels>>,
    /// The right-click menu of the host list, where the mouse was, and the account of the
    /// host it was opened on (none for the empty part of the list).
    hosts_menu: Option<(gpui::Point<Pixels>, Option<String>)>,
    /// A tab being renamed in place: its id, the name field, and the blur watch that
    /// saves when focus leaves it.
    renaming: Option<(u64, Entity<TextInput>, gpui::Subscription)>,
    page: Page,
    /// Created on first visit, then kept so the page keeps its scroll position.
    settings: Option<Entity<SettingsView>>,
    /// The page Settings was opened from, shown again when it closes.
    settings_from: Page,
    pending: Vec<Pending>,
    pane_drag: Option<PaneDrag>,
    /// Where the page content (below the tab bar) was drawn last frame.
    content_bounds: Rc<Cell<gpui::Bounds<Pixels>>>,
    /// A close waiting for "are you sure": tab id, and the pane for a split.
    confirm: Option<(u64, Option<usize>)>,
    /// Quit was refused because this many sessions are open.
    quit_notice: Option<usize>,
    help: bool,
    /// A host delete waiting for "are you sure": index into the host list.
    confirm_delete: Option<usize>,
    tab_scroll: gpui::ScrollHandle,
    next_pending: u64,
    next_tab: u64,
    /// Account of the host picked on the hosts page. By account, not index, so it
    /// survives hosts being deleted or reordered.
    selected: Option<String>,
    /// The search field on the hosts page.
    search: Entity<TextInput>,
    /// Folders on the hosts page showing only their title, by name.
    folded: HashSet<String>,
    /// Bumped whenever the visible page changes, to replay the page fade.
    epoch: u64,
    form: Option<Entity<HostForm>>,
    /// Host key prompts, one shown at a time. A queue, so connecting to several new
    /// servers at once asks about each of them in turn.
    trust: VecDeque<TrustRequest>,
    trust_tx: UnboundedSender<TrustRequest>,
    /// Login questions, one shown at a time in `code`.
    codes: VecDeque<CodeRequest>,
    code: Option<Entity<CodeDialog>>,
    code_tx: UnboundedSender<CodeRequest>,
    /// Round trip in ms to the server of the terminal in front (`latency`).
    latency: Option<u32>,
    status: Option<(SharedString, bool)>,
    /// Bumped by every `set_status`, so a hide timer only clears the message it was set for.
    status_seq: u64,
    /// The `status_seq` whose hide timer is running.
    status_timer: u64,
    focus: FocusHandle,
    /// The hosts tab on the tab strip.
    home_focus: FocusHandle,
    /// Around the dialog or menu on top, to keep Tab inside it.
    modal_focus: FocusHandle,
    window: gpui::AnyWindowHandle,
}

/// Width of the window buttons: three 40 px buttons and their border.
const CONTROLS_W: f32 = 3. * 40. + 1.;

/// How long a status line message stays up. Errors stay longer so there is time to read them.
const STATUS_SECS: u64 = 5;
const STATUS_ERROR_SECS: u64 = 10;

impl Workspace {
    /// Follow the system look from the start. Activation catches changes that send no
    /// appearance event, like high contrast on Windows.
    fn follow_system_look(window: &mut Window, cx: &mut Context<Self>) {
        read_system_look(window);
        cx.observe_window_appearance(window, |_, window, _| read_system_look(window))
            .detach();
        cx.observe_window_activation(window, |_, window, _| read_system_look(window))
            .detach();
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cfg_path = Config::default_path().unwrap_or_else(|_| PathBuf::from("hosts.toml"));
        let mut status = None;
        let cfg = match Config::load(&cfg_path) {
            Ok(c) => c,
            Err(e) => {
                // Keep the unreadable file: the next save would otherwise overwrite it.
                // Set without `set_status`, so this one stays until something replaces it.
                let backup = cfg_path.with_extension("toml.bak");
                let kept = std::fs::copy(&cfg_path, &backup).is_ok();
                status = Some((
                    format!(
                        "Could not read hosts: {e}{}",
                        if kept {
                            format!(" (copy kept as {})", backup.display())
                        } else {
                            String::new()
                        }
                    )
                    .into(),
                    true,
                ));
                Config::default()
            }
        };
        refresh_privacy_tokens(cfg.hosts.iter());
        let (trust_tx, mut trust_rx) = unbounded_channel::<TrustRequest>();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(req) = trust_rx.recv().await {
                this.update_in(cx, |t, window, cx| {
                    // It comes from the network, not from a key or click: take focus so
                    // Tab and Escape reach it, unless another dialog is in front.
                    if !t.modal_open() {
                        window.focus(&t.modal_focus, cx);
                    }
                    t.trust.push_back(req);
                    cx.notify();
                })
                .ok()?;
            }
            None::<()>
        })
        .detach();
        Self::watch_latency(window, cx);
        Self::follow_system_look(window, cx);
        let search = cx.new(|cx| {
            let mut t = TextInput::new("Search or user@host", false, cx);
            t.set_hide_mode(Hide::Redact);
            t
        });
        cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        let (code_tx, mut code_rx) = unbounded_channel::<CodeRequest>();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(req) = code_rx.recv().await {
                this.update(cx, |t, cx| {
                    // Shown by the next render.
                    t.codes.push_back(req);
                    cx.notify();
                })
                .ok()?;
            }
            None::<()>
        })
        .detach();
        Self {
            window: window.window_handle(),
            cfg,
            quick: HashMap::new(),
            quick_form: None,
            snippet_picker: None,
            unlock: None,
            after_unlock: None,
            cfg_path,
            conns: HashMap::new(),
            tabs: vec![],
            active: 0,
            tab_menu: None,
            split_menu: None,
            hosts_menu: None,
            renaming: None,
            page: Page::Home,
            settings: None,
            settings_from: Page::Home,
            pending: Vec::new(),
            next_pending: 0,
            next_tab: 0,
            pane_drag: None,
            content_bounds: Rc::new(Cell::new(gpui::Bounds::default())),
            confirm: None,
            confirm_delete: None,
            quit_notice: None,
            help: false,
            tab_scroll: gpui::ScrollHandle::new(),
            selected: None,
            search,
            folded: HashSet::new(),
            epoch: 0,
            form: None,
            trust: VecDeque::new(),
            trust_tx,
            codes: VecDeque::new(),
            code: None,
            code_tx,
            latency: None,
            status,
            status_seq: 0,
            status_timer: 0,
            focus: cx.focus_handle(),
            home_focus: cx.focus_handle().tab_stop(true),
            modal_focus: cx.focus_handle(),
        }
    }

    /// Show a message in the status line. It hides itself after a few seconds (see
    /// `hide_status_later`), or when clicked.
    fn set_status(&mut self, msg: impl Into<SharedString>, error: bool) {
        self.status = Some((msg.into(), error));
        self.status_seq += 1;
    }

    /// Start the hide timer for the current message, once per message. Runs from `render`,
    /// which every status change goes through, so callers don't need a context for it.
    fn hide_status_later(&mut self, cx: &mut Context<Self>) {
        let Some((_, error)) = &self.status else {
            return;
        };
        if self.status_timer == self.status_seq {
            return;
        }
        let seq = self.status_seq;
        self.status_timer = seq;
        let secs = if *error {
            STATUS_ERROR_SECS
        } else {
            STATUS_SECS
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(secs))
                .await;
            this.update(cx, |t, cx| {
                if t.status_seq == seq {
                    t.status = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn status_line(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (msg, err) = self.status.clone()?;
        Some(
            keyboard(div().id("status"))
                .px_2()
                .py(px(3.))
                .border_t_1()
                .border_color(c(palette().border_variant))
                .text_color(c(if err { palette().red } else { palette().muted }))
                .focus(|s| s.bg(c(palette().hover)))
                .cursor_pointer()
                .tooltip(crate::widgets::tip("Click to dismiss"))
                .on_click(cx.listener(|t, _, _, cx| {
                    t.status = None;
                    cx.notify();
                }))
                .child(privacy::redact(&msg).into_owned()),
        )
    }

    /// Hosts and users to mask: saved hosts and quick connect hosts. Views drawn from
    /// the last frame do not see the new list, so every window is drawn again.
    fn refresh_tokens(&self, cx: &mut App) {
        refresh_privacy_tokens(
            self.cfg
                .hosts
                .iter()
                .chain(self.quick.values().map(|(h, _)| h)),
        );
        cx.refresh_windows();
    }

    fn save_config(&mut self, cx: &mut App) {
        self.refresh_tokens(cx);
        if let Err(e) = self.cfg.save(&self.cfg_path) {
            self.set_status(format!("Could not save hosts: {e}"), true);
        }
    }

    /// Show the Settings page (a page like the hosts page, not a tab).
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_none() {
            let view = cx.new(SettingsView::new);
            cx.subscribe_in(&view, window, |this, _, ev: &SettingsEvent, window, cx| {
                match ev {
                    SettingsEvent::Changed => {}
                    SettingsEvent::Close => {
                        this.close_settings(window, cx);
                        return;
                    }
                    SettingsEvent::SwitchStore(to) => this.switch_store(*to, window, cx),
                    SettingsEvent::Unlock => this.show_unlock(UnlockMode::for_file(), window, cx),
                    SettingsEvent::Lock => {
                        secrets::lock();
                        this.set_status("Secrets file locked.", false);
                    }
                    SettingsEvent::ChangeMaster => this.show_unlock(UnlockMode::Change, window, cx),
                }
                this.settings_changed(cx)
            })
            .detach();
            self.settings = Some(view);
        }
        if self.page != Page::Settings {
            self.settings_from = self.page;
        }
        self.show_page(Page::Settings, window, cx);
    }

    /// Leave Settings for the page it was opened from.
    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.settings_from {
            Page::Tab if !self.tabs.is_empty() => {
                self.select_tab(self.active.min(self.tabs.len() - 1), window, cx)
            }
            _ => self.show_page(Page::Home, window, cx),
        }
    }

    /// The settings button and shortcut: open Settings, or close it when it shows.
    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Settings {
            self.close_settings(window, cx);
        } else {
            self.open_settings(window, cx);
        }
    }

    /// Settings changed somewhere: re-filter file lists and draw every window again, since
    /// views read the live settings (theme, privacy) outside of any entity.
    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        crate::shortcuts::apply(cx);
        self.refresh_file_lists(cx);
        for w in self.other_windows(cx) {
            w.update(cx, |ws, _, cx| ws.refresh_file_lists(cx)).ok();
        }
        cx.refresh_windows();
    }

    fn refresh_file_lists(&self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            let nested = match &tab.kind {
                TabKind::Terminal(t) => t.panes.iter().filter_map(PaneView::tab).collect(),
                _ => vec![tab],
            };
            for tab in nested {
                if let TabKind::Sftp(v) = &tab.kind {
                    v.update(cx, SftpView::settings_changed);
                }
            }
        }
    }

    /// Quick switch for the privacy master setting.
    fn toggle_hide(&mut self, cx: &mut Context<Self>) {
        let on = !crate::prefs::get().hide_sensitive();
        if let Err(e) = crate::prefs::update(|s| s.hide_sensitive = Some(on)) {
            self.set_status(format!("Could not save settings: {e}"), true);
        }
        self.settings_changed(cx);
    }

    fn zoom(&mut self, step: Option<f32>, cx: &mut Context<Self>) {
        let size = step.map(|d| zoomed(crate::prefs::get().terminal_font_size(), d));
        if let Err(e) = crate::prefs::update(|s| s.terminal_font_size = size) {
            self.set_status(format!("Could not save settings: {e}"), true);
        }
        self.settings_changed(cx);
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.hide_status_later(cx);
        if self.code.is_none() && !self.codes.is_empty() {
            self.show_code(window, cx);
        }
        // Remember which split pane has focus, for the tab title and for re-selecting the tab.
        if self.page == Page::Tab
            && let Some(Tab {
                kind: TabKind::Terminal(t),
                ..
            }) = self.tabs.get_mut(self.active)
        {
            t.focused = Self::focused_pane(t, window, cx);
        }
        self.arrange_splits(window);
        if self.page == Page::Tab
            && let Some(Tab {
                kind: TabKind::Terminal(t),
                ..
            }) = self.tabs.get_mut(self.active)
            && t.step()
        {
            window.request_animation_frame();
        }
        static FIRST: AtomicBool = AtomicBool::new(true);
        if FIRST.swap(false, Ordering::Relaxed) {
            crate::mark("first render");
        }
        let overlays = self.overlays(cx);
        let tab_menu = self.tab_menu(cx);

        div()
            .id("root")
            .track_focus(&self.focus)
            .key_context("Workspace")
            .on_key_down(cx.listener(Self::on_root_key))
            .on_action(cx.listener(|t, _: &SplitPane, w, cx| t.split(w, cx)))
            .on_action(cx.listener(|t, _: &ShowHelp, w, cx| t.toggle_help(w, cx)))
            .on_action(cx.listener(|t, _: &SplitMenu, w, cx| t.open_split_menu(None, w, cx)))
            .on_action(cx.listener(|t, _: &TabMenu, w, cx| t.open_active_tab_menu(w, cx)))
            .on_action(cx.listener(|t, _: &CloseTab, w, cx| match t.page {
                Page::Tab => t.close_current(w, cx),
                Page::Settings => t.close_settings(w, cx),
                Page::Home => {}
            }))
            .on_action(cx.listener(|t, _: &NextTab, w, cx| {
                if !t.tabs.is_empty() {
                    t.select_tab((t.active + 1) % t.tabs.len(), w, cx)
                }
            }))
            .on_action(cx.listener(|t, _: &PrevTab, w, cx| {
                if !t.tabs.is_empty() {
                    t.select_tab((t.active + t.tabs.len() - 1) % t.tabs.len(), w, cx)
                }
            }))
            .on_action(cx.listener(|t, a: &SelectTab, w, cx| t.select_tab(a.0, w, cx)))
            .on_action(cx.listener(|t, _: &NewHost, w, cx| t.open_form(None, w, cx)))
            .on_action(cx.listener(|t, _: &QuickConnect, w, cx| t.open_quick(w, cx)))
            .on_action(cx.listener(|t, _: &Snippets, w, cx| t.open_snippets(w, cx)))
            .on_action(cx.listener(|t, _: &OpenSettings, w, cx| t.toggle_settings(w, cx)))
            .on_action(cx.listener(|t, _: &FocusTabs, w, cx| t.focus_tabs(w, cx)))
            .on_action(cx.listener(|t, _: &FocusNext, w, cx| t.move_focus(false, w, cx)))
            .on_action(cx.listener(|t, _: &FocusPrev, w, cx| t.move_focus(true, w, cx)))
            .on_action(cx.listener(|t, _: &NextPane, w, cx| t.cycle_pane(false, w, cx)))
            .on_action(cx.listener(|t, _: &PrevPane, w, cx| t.cycle_pane(true, w, cx)))
            .on_action(cx.listener(|t, _: &ZoomIn, _, cx| t.zoom(Some(1.), cx)))
            .on_action(cx.listener(|t, _: &ZoomOut, _, cx| t.zoom(Some(-1.), cx)))
            .on_action(cx.listener(|t, _: &ResetZoom, _, cx| t.zoom(None, cx)))
            .size_full()
            .flex()
            .font_family(ui())
            .text_size(px(12.))
            .bg(c(bg()))
            .text_color(c(palette().text))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.tab_bar(window.viewport_size().width, cx))
                    .child({
                        let cell = self.content_bounds.clone();
                        div()
                            .flex_1()
                            .min_h_0()
                            .relative()
                            .child(page_in(
                                div()
                                    .size_full()
                                    .child(self.content(window.viewport_size().width, cx)),
                                ("page", self.epoch as usize),
                            ))
                            // Records the content size, for split layout and divider drags.
                            .child(
                                gpui::canvas(move |b, _, _| cell.set(b), |_, _, _, _| {})
                                    .absolute()
                                    .size_full(),
                            )
                    })
                    .children(self.pending_rows(cx))
                    .children(self.status_line(cx)),
            )
            .children(overlays)
            .child(Self::window_controls())
            // Menus open next to the window buttons, so they go above them.
            .children(tab_menu)
            .children(crate::platform::resize_handles(window))
    }
}

/// Hosts, host names and usernames (and the local user) that "hide sensitive information" masks.
/// Pick up the system's light or dark and contrast settings, for the `System` choices.
fn read_system_look(window: &mut Window) {
    let (dark, high) = crate::platform::system_look(window);
    if set_system(dark, high) {
        window.refresh();
    }
}

fn refresh_privacy_tokens<'a>(hosts: impl Iterator<Item = &'a Host> + Clone) {
    let mut users: Vec<String> = hosts.clone().map(|h| h.user.clone()).collect();
    let names = hosts
        .clone()
        .flat_map(|h| std::iter::once(h.name.clone()).chain(h.group.clone()))
        .collect();
    let hosts = hosts.map(|h| h.host.clone()).collect();
    users.extend(crate::platform::local_user());
    privacy::set_known(hosts, users, names);
}
