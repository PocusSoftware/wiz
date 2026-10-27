//! The window's root view: tabs, pages and dialogs. Its types live here so every part can
//! reach their fields; the behavior is split by concern into the submodules.

mod connect;
mod credentials;
mod focus;
mod hosts;
mod latency;
mod overlays;
mod reconnect;
mod sessions;
mod split;
mod tab_menu;
mod tabs;
#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement,
    Pixels, Render, SharedString, Window, actions, div, prelude::*, px,
};
use ssh_core::config::{AuthMethod, Config, Host};
use ssh_core::docker::{self, Backend};
use ssh_core::hostkey::{HostKeyVerifier, TrustPrompt};
use ssh_core::secrets::{self, Kind, Secret, Store};
use ssh_core::session::{AuthPrompt, Connection, Credentials, Options, Question};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::dialogs::code::{CodeDialog, CodeEvent};
use crate::dialogs::host_form::{FormEvent, HostForm};
use crate::dialogs::quick_connect::{QuickConnect as QuickDialog, QuickEvent};
use crate::dialogs::unlock::{UnlockDialog, UnlockEvent, UnlockMode};
use crate::privacy;
use crate::runtime;
use crate::theme::*;
use crate::views::docker::{DockerEvent, DockerView};
use crate::views::logs::LogView;
use crate::views::settings::{SettingsEvent, SettingsView};
use crate::views::sftp::{RemoteRef, SftpEvent, SftpView, Side as SftpSide, Source as SftpSource};
use crate::views::terminal::{TerminalEvent, TerminalView};
use crate::widgets::anim::{fade_in, pop_in, stagger_in};
use crate::widgets::input::TextInput;
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
        FocusTabs,
        NextPane,
        PrevPane
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

/// A terminal tab: one shell, or several split side by side.
struct TermTab {
    panes: Vec<Pane>,
    /// Pane that had focus last; gets it back when the tab is selected.
    focused: usize,
    /// Pane indices per column, left to right, each column top to bottom. Picked from the
    /// space available on every render (see `layout`).
    layout: Vec<Vec<usize>>,
    /// Width of each column as a share of the tab, summing to 1.
    cols: Vec<f32>,
    /// Height of each pane in its column as a share of the column, summing to 1.
    rows: Vec<Vec<f32>>,
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
}

/// Runs once the secrets file is unlocked.
type AfterUnlock = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

pub struct Workspace {
    cfg: Config,
    /// Hosts opened with quick connect (not saved), by account, with the credentials typed
    /// for them, so a dropped terminal can reconnect.
    quick: HashMap<String, (Host, Credentials)>,
    quick_form: Option<Entity<QuickDialog>>,
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
    /// A host delete waiting for "are you sure": index into the host list.
    confirm_delete: Option<usize>,
    tab_scroll: gpui::ScrollHandle,
    next_pending: u64,
    next_tab: u64,
    /// Host groups folded shut on the hosts page.
    collapsed: HashSet<String>,
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
}

/// Width of the window buttons: three 40 px buttons and their border.
const CONTROLS_W: f32 = 3. * 40. + 1.;

/// How long a status line message stays up. Errors stay longer so there is time to read them.
const STATUS_SECS: u64 = 5;
const STATUS_ERROR_SECS: u64 = 10;

impl Workspace {
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
                        window.focus(&t.modal_focus);
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
            cfg,
            quick: HashMap::new(),
            quick_form: None,
            unlock: None,
            after_unlock: None,
            cfg_path,
            conns: HashMap::new(),
            tabs: vec![],
            active: 0,
            tab_menu: None,
            split_menu: None,
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
            tab_scroll: gpui::ScrollHandle::new(),
            collapsed: HashSet::new(),
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

    /// Hosts and users to mask: saved hosts and quick connect hosts.
    fn refresh_tokens(&self) {
        refresh_privacy_tokens(
            self.cfg
                .hosts
                .iter()
                .chain(self.quick.values().map(|(h, _)| h)),
        );
    }

    fn save_config(&mut self) {
        self.refresh_tokens();
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
                    SettingsEvent::Close => this.close_settings(window, cx),
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

    /// Settings changed somewhere: repaint every page (they read the live settings) and
    /// re-filter file lists.
    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        crate::shortcuts::apply(cx);
        for tab in &self.tabs {
            match &tab.kind {
                TabKind::Terminal(t) => {
                    for p in &t.panes {
                        p.view.update(cx, |_, cx| cx.notify());
                    }
                }
                TabKind::Docker(v) => v.update(cx, |_, cx| cx.notify()),
                TabKind::Sftp(v) => v.update(cx, SftpView::settings_changed),
                TabKind::Logs(v) => v.update(cx, |_, cx| cx.notify()),
            }
        }
        if let Some(v) = &self.settings {
            v.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    /// Quick switch for the privacy master setting.
    fn toggle_hide(&mut self, cx: &mut Context<Self>) {
        let on = !crate::prefs::get().hide_sensitive();
        if let Err(e) = crate::prefs::update(|s| s.hide_sensitive = Some(on)) {
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
        static FIRST: AtomicBool = AtomicBool::new(true);
        if FIRST.swap(false, Ordering::Relaxed) {
            crate::mark("first render");
        }
        let overlays = self.overlays(cx);
        let tab_menu = self.tab_menu(window, cx);

        div()
            .id("root")
            .track_focus(&self.focus)
            .key_context("Workspace")
            .on_key_down(cx.listener(|t, ev: &gpui::KeyDownEvent, w, cx| {
                // Reaches here only when nothing focused took it: the shell and the file
                // panes keep Tab.
                let m = &ev.keystroke.modifiers;
                if ev.keystroke.key == "tab" && !(m.control || m.platform || m.alt) {
                    t.move_focus(m.shift, w, cx);
                    cx.stop_propagation();
                    return;
                }
                if t.renaming.is_some() {
                    match ev.keystroke.key.as_str() {
                        "enter" => t.finish_rename(true, w, cx),
                        "escape" => t.finish_rename(false, w, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                    return;
                }
                if t.menu_open() {
                    match ev.keystroke.key.as_str() {
                        "escape" => t.close_tab_menu(w, cx),
                        "up" => t.move_focus(true, w, cx),
                        "down" => t.move_focus(false, w, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                    return;
                }
                if t.quit_notice.is_some() {
                    if matches!(ev.keystroke.key.as_str(), "enter" | "escape") {
                        t.dismiss_quit_notice(w, cx);
                        cx.stop_propagation();
                    }
                    return;
                }
                if t.confirm_delete.is_some() {
                    match ev.keystroke.key.as_str() {
                        "enter" => t.answer_delete_host(true, w, cx),
                        "escape" => t.answer_delete_host(false, w, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                    return;
                }
                if t.confirm.is_some() {
                    match ev.keystroke.key.as_str() {
                        "enter" => t.answer_close(true, w, cx),
                        "escape" => t.answer_close(false, w, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                    return;
                }
                // No Enter here: trusting a new key takes a deliberate press of the button.
                if !t.trust.is_empty() && ev.keystroke.key == "escape" {
                    t.answer_trust(false, w, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|t, _: &SplitPane, w, cx| t.split(w, cx)))
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
            .on_action(cx.listener(|t, _: &OpenSettings, w, cx| t.toggle_settings(w, cx)))
            .on_action(cx.listener(|t, _: &FocusTabs, w, cx| t.focus_tabs(w, cx)))
            .on_action(cx.listener(|t, _: &FocusNext, w, cx| t.move_focus(false, w, cx)))
            .on_action(cx.listener(|t, _: &FocusPrev, w, cx| t.move_focus(true, w, cx)))
            .on_action(cx.listener(|t, _: &NextPane, w, cx| t.cycle_pane(false, w, cx)))
            .on_action(cx.listener(|t, _: &PrevPane, w, cx| t.cycle_pane(true, w, cx)))
            .size_full()
            .flex()
            .font_family(ui())
            .text_size(px(12.))
            .bg(c(bg()))
            .text_color(c(TEXT))
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
                            .child(fade_in(
                                div().size_full().child(self.content(cx)),
                                ("page", self.epoch as usize),
                            ))
                            // Records the content size, for split layout and divider drags.
                            .child(
                                gpui::canvas(move |b, _, _| cell.set(b), |_, _, _, _| {})
                                    .absolute()
                                    .size_full(),
                            )
                    })
                    .children(self.pending.iter().map(|p| {
                        let account = p.account.clone();
                        div()
                            .px_2()
                            .py(px(1.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .border_t_1()
                            .border_color(c(BORDER_VARIANT))
                            .text_color(c(MUTED))
                            .child(div().size(px(7.)).bg(c(YELLOW)))
                            .child(
                                div().flex_1().child(
                                    privacy::redact(&format!("Connecting to {}...", p.name))
                                        .into_owned(),
                                ),
                            )
                            .child(
                                icon_button(
                                    ("cancel-connect", p.id as usize),
                                    "x",
                                    "Cancel",
                                    MUTED,
                                )
                                .size(px(18.))
                                .on_click(
                                    cx.listener(move |t, _, _, cx| t.cancel_connect(&account, cx)),
                                ),
                            )
                    }))
                    .when_some(self.status.clone(), |d, (msg, err)| {
                        d.child(
                            keyboard(div().id("status"))
                                .px_2()
                                .py(px(3.))
                                .border_t_1()
                                .border_color(c(BORDER_VARIANT))
                                .text_color(c(if err { RED } else { MUTED }))
                                .focus(|s| s.bg(c(HOVER)))
                                .cursor_pointer()
                                .tooltip(crate::widgets::tip("Click to dismiss"))
                                .on_click(cx.listener(|t, _, _, cx| {
                                    t.status = None;
                                    cx.notify();
                                }))
                                .child(privacy::redact(&msg).into_owned()),
                        )
                    }),
            )
            .children(overlays)
            .children(tab_menu)
            .child(Self::window_controls())
    }
}

/// Hosts and usernames (and the local user) that "hide sensitive information" masks.
fn refresh_privacy_tokens<'a>(hosts: impl Iterator<Item = &'a Host> + Clone) {
    let mut users: Vec<String> = hosts.clone().map(|h| h.user.clone()).collect();
    let hosts = hosts.map(|h| h.host.clone()).collect();
    users.extend(crate::platform::local_user());
    privacy::set_known(hosts, users);
}
