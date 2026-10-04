use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement,
    Render, SharedString, Window, actions, div, prelude::*, px,
};
use ssh_core::config::{AuthMethod, Config, Host};
use ssh_core::docker::{self, Backend};
use ssh_core::hostkey::{HostKeyVerifier, TrustPrompt};
use ssh_core::keychain::{self, Kind};
use ssh_core::session::{Connection, Credentials};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::anim::{fade_in, pop_in, stagger_in};
use crate::docker_view::{DockerEvent, DockerView};
use crate::host_form::{FormEvent, HostForm};
use crate::log_view::LogView;
use crate::privacy;
use crate::runtime;
use crate::settings_view::{SettingsEvent, SettingsView};
use crate::sftp_view::{RemoteRef, SftpEvent, SftpView, Side as SftpSide, Source as SftpSource};
use crate::terminal_view::TerminalView;
use crate::theme::*;
use crate::ui::{button, icon, icon_button, primary_button};

actions!(
    app,
    [Quit, CloseTab, NextTab, PrevTab, NewHost, OpenSettings]
);

struct TrustRequest {
    host: String,
    port: u16,
    fingerprint: String,
    reply: std::sync::mpsc::SyncSender<bool>,
}

/// Called from the tokio side during the handshake. Blocks that worker until the
/// user answers in the UI. `prompting` is set meanwhile, so the connect timeout pauses
/// while the user reads the fingerprint.
struct GuiPrompt {
    tx: UnboundedSender<TrustRequest>,
    prompting: Arc<AtomicBool>,
}

impl TrustPrompt for GuiPrompt {
    fn trust_new_host(&self, host: &str, port: u16, fingerprint: &str) -> bool {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = TrustRequest {
            host: host.into(),
            port,
            fingerprint: fingerprint.into(),
            reply: tx,
        };
        if self.tx.send(req).is_err() {
            return false;
        }
        self.prompting.store(true, Ordering::Relaxed);
        let ok = rx.recv_timeout(Duration::from_secs(120)).unwrap_or(false);
        self.prompting.store(false, Ordering::Relaxed);
        ok
    }
}

/// Run `work` until it finishes, the cancel sender is dropped, or `timeout_secs` (0 = never)
/// pass. Time spent in the host key prompt does not count. `None` means cancelled.
async fn race<T>(
    work: impl std::future::Future<Output = ssh_core::Result<T>>,
    cancel: &mut oneshot::Receiver<()>,
    timeout_secs: u32,
    prompting: &AtomicBool,
) -> ssh_core::Result<Option<T>> {
    const TICK: Duration = Duration::from_millis(250);
    let limit = Duration::from_secs(timeout_secs as u64);
    let mut waited = Duration::ZERO;
    tokio::pin!(work);
    loop {
        tokio::select! {
            r = &mut work => return r.map(Some),
            _ = &mut *cancel => return Ok(None),
            _ = tokio::time::sleep(TICK) => {
                if timeout_secs > 0 && !prompting.load(Ordering::Relaxed) {
                    waited += TICK;
                    if waited >= limit {
                        return Err(ssh_core::Error::Timeout(timeout_secs));
                    }
                }
            }
        }
    }
}

/// A connection attempt still in flight. Dropping `_cancel` aborts it.
struct Pending {
    id: u64,
    account: String,
    name: String,
    host: String,
    port: u16,
    _cancel: oneshot::Sender<()>,
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
    password: Option<String>,
}

enum Open {
    Shell,
    Docker,
    Sftp,
    /// Connect a host and attach it to one pane of an open SFTP tab.
    SftpSource(gpui::WeakEntity<SftpView>, SftpSide),
}

enum TabKind {
    Terminal(Entity<TerminalView>),
    Docker(Entity<DockerView>),
    Sftp(Entity<SftpView>),
    Logs(Entity<LogView>),
}

struct Tab {
    label: SharedString,
    kind: TabKind,
}

pub struct Root {
    cfg: Config,
    cfg_path: PathBuf,
    conns: HashMap<String, Live>,
    tabs: Vec<Tab>,
    active: usize,
    page: Page,
    /// Created on first visit, then kept so the page keeps its scroll position.
    settings: Option<Entity<SettingsView>>,
    pending: Vec<Pending>,
    next_pending: u64,
    /// Host groups folded shut on the hosts page.
    collapsed: HashSet<String>,
    /// Bumped whenever the visible page changes, to replay the page fade.
    epoch: u64,
    form: Option<Entity<HostForm>>,
    trust: Option<TrustRequest>,
    trust_tx: UnboundedSender<TrustRequest>,
    status: Option<(SharedString, bool)>,
    focus: FocusHandle,
}

impl Root {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let cfg_path = Config::default_path().unwrap_or_else(|_| PathBuf::from("hosts.toml"));
        let mut status = None;
        let cfg = match Config::load(&cfg_path) {
            Ok(c) => c,
            Err(e) => {
                // Keep the unreadable file: the next save would otherwise overwrite it.
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
        refresh_privacy_tokens(&cfg);
        let (trust_tx, mut trust_rx) = unbounded_channel::<TrustRequest>();
        cx.spawn(async move |this, cx| {
            while let Some(req) = trust_rx.recv().await {
                this.update(cx, |t, cx| {
                    t.trust = Some(req);
                    cx.notify();
                })
                .ok()?;
            }
            None::<()>
        })
        .detach();
        Self {
            cfg,
            cfg_path,
            conns: HashMap::new(),
            tabs: vec![],
            active: 0,
            page: Page::Home,
            settings: None,
            pending: Vec::new(),
            next_pending: 0,
            collapsed: HashSet::new(),
            epoch: 0,
            form: None,
            trust: None,
            trust_tx,
            status,
            focus: cx.focus_handle(),
        }
    }

    fn set_status(&mut self, msg: impl Into<SharedString>, error: bool) {
        self.status = Some((msg.into(), error));
    }

    fn save_config(&mut self) {
        refresh_privacy_tokens(&self.cfg);
        if let Err(e) = self.cfg.save(&self.cfg_path) {
            self.set_status(format!("Could not save hosts: {e}"), true);
        }
    }

    // ---------- tabs ----------

    fn push_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.push(Tab { ..tab });
        self.select_tab(self.tabs.len() - 1, window, cx);
    }

    fn select_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // Re-selecting the current page only refocuses it; the enter animation is for changes.
        if self.page != Page::Tab || ix != self.active {
            self.epoch += 1;
        }
        self.active = ix;
        self.page = Page::Tab;
        match &self.tabs[ix].kind {
            TabKind::Terminal(t) => t.read(cx).focus(window),
            TabKind::Sftp(v) => {
                let h = v.read(cx).focus_handle(cx);
                window.focus(&h)
            }
            TabKind::Logs(v) => {
                let h = v.read(cx).focus_handle(cx);
                window.focus(&h)
            }
            _ => window.focus(&self.focus),
        }
        cx.notify();
    }

    /// Show the hosts or settings page.
    fn show_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            self.epoch += 1;
        }
        match (page, &self.settings) {
            (Page::Settings, Some(v)) => window.focus(&v.read(cx).focus_handle(cx)),
            _ => window.focus(&self.focus),
        }
        cx.notify();
    }

    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // Dropping the view drops the shell input sender, which closes the channel.
        self.tabs.remove(ix);
        if self.page != Page::Tab {
            // Closed from the tab strip while another page shows: stay there.
            if ix < self.active || self.active >= self.tabs.len() {
                self.active = self.active.saturating_sub(1);
            }
            cx.notify();
        } else if self.tabs.is_empty() {
            self.active = 0;
            self.show_page(Page::Home, window, cx);
        } else {
            let next = if ix < self.active {
                self.active - 1
            } else {
                self.active
            };
            self.select_tab(next.min(self.tabs.len() - 1), window, cx);
        }
    }

    // ---------- connecting ----------

    fn open_host(&mut self, ix: usize, what: Open, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.cfg.hosts.get(ix).cloned() else {
            return;
        };
        let account = host.account();
        if let Some(live) = self.conns.get(&account) {
            if !live.conn.is_closed() {
                let (conn, pw) = (live.conn.clone(), live.password.clone());
                self.finish_open(host, conn, pw, what, window, cx);
                return;
            }
            self.conns.remove(&account);
        }
        if self.pending.iter().any(|p| p.account == account) {
            // Already connecting: a second click must not open a second session.
            return;
        }
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        let id = self.next_pending;
        self.next_pending += 1;
        self.pending.push(Pending {
            id,
            account: account.clone(),
            name: host.name.clone(),
            host: host.host.clone(),
            port: host.port,
            _cancel: cancel_tx,
        });
        self.status = None;
        cx.notify();
        let prompting = Arc::new(AtomicBool::new(false));
        let verifier = HostKeyVerifier {
            known_hosts: HostKeyVerifier::default_path()
                .unwrap_or_else(|_| PathBuf::from("known_hosts")),
            prompt: Arc::new(GuiPrompt {
                tx: self.trust_tx.clone(),
                prompting: prompting.clone(),
            }),
        };
        let prefs = crate::prefs::get();
        let (keepalive_secs, timeout_secs) = (prefs.keepalive_secs(), prefs.connect_timeout_secs());
        cx.spawn_in(window, async move |this, cx| {
            let h = host.clone();
            let r = runtime::spawn(async move {
                // Keychain access can block on a system prompt, keep it off the async workers.
                // It is not timed: macOS may be waiting for the user to allow access.
                let hk = h.clone();
                let creds = tokio::select! {
                    r = tokio::task::spawn_blocking(move || Credentials::from_keychain(&hk)) => {
                        r.map_err(|e| ssh_core::Error::Remote(e.to_string()))??
                    }
                    _ = &mut cancel_rx => return Ok(None),
                };
                let pw = match &creds {
                    Credentials::Password(p) => Some(p.clone()),
                    _ => None,
                };
                let keepalive = std::time::Duration::from_secs(keepalive_secs as u64);
                let conn = race(
                    Connection::connect_with(&h, &creds, verifier, keepalive),
                    &mut cancel_rx,
                    timeout_secs,
                    &prompting,
                )
                .await?;
                Ok::<_, ssh_core::Error>(conn.map(|c| (c, pw)))
            })
            .await;
            this.update_in(cx, |this, window, cx| {
                // Cancelled (X, host deleted) while connecting: drop whatever came back.
                let Some(pos) = this.pending.iter().position(|p| p.id == id) else {
                    return;
                };
                this.pending.remove(pos);
                let exists = this.cfg.hosts.iter().any(|h| h.account() == account);
                match r {
                    Ok(Some((conn, pw))) if exists => {
                        let conn = Arc::new(conn);
                        this.conns.insert(
                            account,
                            Live {
                                conn: conn.clone(),
                                password: pw.clone(),
                            },
                        );
                        this.finish_open(host, conn, pw, what, window, cx);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        this.dismiss_trust(&host.host, host.port);
                        this.set_status(format!("{}: {e}", host.name), true);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stop connecting to `account`. The task notices its cancel sender is gone.
    fn cancel_connect(&mut self, account: &str, cx: &mut Context<Self>) {
        if let Some(pos) = self.pending.iter().position(|p| p.account == account) {
            let p = self.pending.remove(pos);
            self.dismiss_trust(&p.host, p.port);
            cx.notify();
        }
    }

    /// Answer "no" to an open host key prompt for this address.
    fn dismiss_trust(&mut self, host: &str, port: u16) {
        if self
            .trust
            .as_ref()
            .is_some_and(|t| t.host == host && t.port == port)
            && let Some(t) = self.trust.take()
        {
            let _ = t.reply.send(false);
        }
    }

    fn finish_open(
        &mut self,
        host: Host,
        conn: Arc<Connection>,
        password: Option<String>,
        what: Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match what {
            Open::Shell => self.open_shell(host.name.clone(), conn, password, None, window, cx),
            Open::Docker => {
                let label = format!("{} · docker", host.name);
                let view = cx.new(|cx| DockerView::new(Backend::Remote(conn.clone()), true, cx));
                cx.subscribe_in(
                    &view,
                    window,
                    move |this, _, ev: &DockerEvent, window, cx| {
                        this.on_docker_event(
                            ev,
                            Some((host.name.clone(), conn.clone(), password.clone())),
                            window,
                            cx,
                        )
                    },
                )
                .detach();
                self.push_tab(
                    Tab {
                        label: label.into(),
                        kind: TabKind::Docker(view),
                    },
                    window,
                    cx,
                );
            }
            Open::Sftp => {
                let label = format!("{} · files", host.name);
                let names: Vec<(String, String)> = self
                    .cfg
                    .hosts
                    .iter()
                    .map(|h| (h.account(), h.name.clone()))
                    .collect();
                cx.spawn_in(window, async move |this, cx| {
                    let c = conn.clone();
                    let r = runtime::spawn(async move { c.open_sftp().await }).await;
                    this.update_in(cx, |this, window, cx| match r {
                        Ok(sftp) => {
                            let remote = SftpSource::Remote(RemoteRef {
                                label: host.name.clone(),
                                host: host.clone(),
                                sftp: Arc::new(sftp),
                            });
                            let view =
                                cx.new(|cx| SftpView::new(SftpSource::Local, remote, names, cx));
                            cx.subscribe_in(
                                &view,
                                window,
                                |this, view, ev: &SftpEvent, window, cx| {
                                    this.on_sftp_event(view.downgrade(), ev, window, cx)
                                },
                            )
                            .detach();
                            view.update(cx, |v, cx| v.load(cx));
                            this.push_tab(
                                Tab {
                                    label: label.into(),
                                    kind: TabKind::Sftp(view),
                                },
                                window,
                                cx,
                            );
                        }
                        Err(e) => {
                            this.set_status(format!("{}: {e}", host.name), true);
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Open::SftpSource(view, side) => {
                cx.spawn_in(window, async move |this, cx| {
                    let c = conn.clone();
                    let r = runtime::spawn(async move { c.open_sftp().await }).await;
                    this.update_in(cx, |this, _, cx| match r {
                        Ok(sftp) => {
                            let src = SftpSource::Remote(RemoteRef {
                                label: host.name.clone(),
                                host: host.clone(),
                                sftp: Arc::new(sftp),
                            });
                            view.update(cx, |v, cx| v.set_source(side, src, cx)).ok();
                        }
                        Err(e) => {
                            this.set_status(format!("{}: {e}", host.name), true);
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    fn on_sftp_event(
        &mut self,
        view: gpui::WeakEntity<SftpView>,
        ev: &SftpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            SftpEvent::PickHost {
                side,
                account: None,
            } => {
                view.update(cx, |v, cx| v.set_source(*side, SftpSource::Local, cx))
                    .ok();
            }
            SftpEvent::PickHost {
                side,
                account: Some(acct),
            } => {
                // Look the host up by account: the list may have changed since the tab opened.
                match self.cfg.hosts.iter().position(|h| &h.account() == acct) {
                    Some(ix) => self.open_host(ix, Open::SftpSource(view, *side), window, cx),
                    None => {
                        self.set_status("That host no longer exists.", true);
                        cx.notify();
                    }
                }
            }
        }
    }

    fn open_shell(
        &mut self,
        label: String,
        conn: Arc<Connection>,
        password: Option<String>,
        command: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let r =
                runtime::spawn(async move { conn.open_shell(100, 30, command.as_deref()).await })
                    .await;
            this.update_in(cx, |this, window, cx| match r {
                Ok(shell) => {
                    let view = cx.new(|cx| TerminalView::new(shell, password, cx));
                    cx.observe(&view, |_, _, cx| cx.notify()).detach();
                    this.push_tab(
                        Tab {
                            label: label.into(),
                            kind: TabKind::Terminal(view),
                        },
                        window,
                        cx,
                    );
                }
                Err(e) => {
                    this.set_status(format!("{label}: {e}"), true);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn on_docker_event(
        &mut self,
        ev: &DockerEvent,
        remote: Option<(String, Arc<Connection>, Option<String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            DockerEvent::Shell { id, name } => {
                let Some((host, conn, pw)) = remote else {
                    return;
                };
                match docker::Remote::exec_shell_command(id) {
                    Ok(cmd) => {
                        self.open_shell(format!("{host} · {name}"), conn, pw, Some(cmd), window, cx)
                    }
                    Err(e) => {
                        self.set_status(e.to_string(), true);
                        cx.notify();
                    }
                }
            }
            DockerEvent::Logs { name, id, backend } => {
                let (id, backend) = (id.clone(), backend.clone());
                let view = cx.new(|cx| LogView::new(backend, id, cx));
                self.push_tab(
                    Tab {
                        label: format!("{name} · logs").into(),
                        kind: TabKind::Logs(view),
                    },
                    window,
                    cx,
                );
            }
        }
    }

    fn open_local_docker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let r = runtime::spawn(async { docker::Local::new() }).await;
            this.update_in(cx, |this, window, cx| match r {
                Ok(local) => {
                    let view =
                        cx.new(|cx| DockerView::new(Backend::Local(Arc::new(local)), false, cx));
                    cx.subscribe_in(&view, window, |this, _, ev: &DockerEvent, window, cx| {
                        this.on_docker_event(ev, None, window, cx)
                    })
                    .detach();
                    this.push_tab(
                        Tab {
                            label: "Local Docker".into(),
                            kind: TabKind::Docker(view),
                        },
                        window,
                        cx,
                    );
                }
                Err(e) => {
                    this.set_status(format!("Docker: {e}"), true);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ---------- hosts ----------

    fn open_form(&mut self, existing: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let ex = existing.and_then(|i| self.cfg.hosts.get(i).cloned().map(|h| (i, h)));
        let taken = self
            .cfg
            .hosts
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != existing)
            .map(|(_, h)| h.account())
            .collect();
        let form = cx.new(|cx| HostForm::new(ex, taken, cx));
        cx.subscribe_in(&form, window, |this, _, ev: &FormEvent, window, cx| {
            match ev {
                FormEvent::Saved { host, index } => {
                    match index {
                        Some(i) if *i < this.cfg.hosts.len() => this.cfg.hosts[*i] = host.clone(),
                        _ => this.cfg.hosts.push(host.clone()),
                    }
                    this.save_config();
                }
                FormEvent::Cancelled => {}
            }
            this.form = None;
            window.focus(&this.focus);
            cx.notify();
        })
        .detach();
        form.read(cx).focus_first(window, cx);
        self.form = Some(form);
        cx.notify();
    }

    fn delete_host(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.cfg.hosts.len() {
            return;
        }
        let h = self.cfg.hosts.remove(ix);
        let acct = h.account();
        self.cancel_connect(&acct, cx);
        for kind in [Kind::Password, Kind::KeyPassphrase] {
            if let Err(e) = keychain::delete(kind, &acct) {
                self.set_status(format!("Keychain: {e}"), true);
            }
        }
        self.conns.remove(&acct);
        self.save_config();
        cx.notify();
    }

    fn toggle_pin(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(h) = self.cfg.hosts.get_mut(ix) {
            h.pinned = !h.pinned;
            self.save_config();
            cx.notify();
        }
    }

    // ---------- settings ----------

    /// Show the Settings page (a page like the hosts page, not a tab).
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_none() {
            let view = cx.new(SettingsView::new);
            cx.subscribe_in(&view, window, |this, _, _: &SettingsEvent, _, cx| {
                this.settings_changed(cx)
            })
            .detach();
            self.settings = Some(view);
        }
        self.show_page(Page::Settings, window, cx);
    }

    /// Settings changed somewhere: repaint every page (they read the live settings) and
    /// re-filter file lists.
    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            match &tab.kind {
                TabKind::Terminal(v) => v.update(cx, |_, cx| cx.notify()),
                TabKind::Docker(v) => v.update(cx, |_, cx| cx.notify()),
                TabKind::Sftp(v) => v.update(cx, |v, cx| v.settings_changed(cx)),
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

    // ---------- render ----------

    /// One host as a big square card. `ix` is the index in `cfg.hosts`, whatever section the
    /// card is shown in.
    fn host_card(&self, ix: usize, h: &Host, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let account = h.account();
        let live = self
            .conns
            .get(&account)
            .is_some_and(|l| !l.conn.is_closed());
        let connecting = self.pending.iter().any(|p| p.account == account);
        let kind_icon = match h.auth {
            AuthMethod::Password => "server",
            AuthMethod::Key { .. } => "key",
        };
        let dot = if live {
            GREEN
        } else if connecting {
            YELLOW
        } else {
            ACTIVE
        };
        div()
            .id(("host", ix))
            .w(px(210.))
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(2.))
            .border_2()
            .border_color(c(BORDER))
            .bg(c(PANEL))
            .cursor_pointer()
            .hover(|s| s.bg(c(ACTIVE)).border_color(c(BORDER_FOCUSED)))
            .on_click(
                cx.listener(move |t, _: &ClickEvent, w, cx| t.open_host(ix, Open::Shell, w, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon(kind_icon, 16., if live { GREEN } else { ACCENT_TEXT }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_nowrap()
                            .child(h.name.clone()),
                    )
                    .child(div().size(px(7.)).bg(c(dot))),
            )
            .child(
                div().text_color(c(MUTED)).whitespace_nowrap().child(
                    privacy::redact(&format!("{}@{}:{}", h.user, h.host, privacy::port(h.port)))
                        .into_owned(),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        icon_button(("h-shell", ix), "terminal", "Open shell", ACCENT_TEXT)
                            .on_click(cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.open_host(ix, Open::Shell, w, cx)
                            })),
                    )
                    .child(
                        icon_button(("h-docker", ix), "container", "Docker containers", MUTED)
                            .on_click(cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.open_host(ix, Open::Docker, w, cx)
                            })),
                    )
                    .child(
                        icon_button(("h-sftp", ix), "folder", "Files (SFTP)", MUTED).on_click(
                            cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.open_host(ix, Open::Sftp, w, cx)
                            }),
                        ),
                    )
                    .child(div().flex_1())
                    .child(
                        icon_button(
                            ("h-pin", ix),
                            "pin",
                            if h.pinned { "Unpin" } else { "Pin to top" },
                            if h.pinned { ACCENT_TEXT } else { MUTED },
                        )
                        .on_click(cx.listener(move |t, _, _, cx| {
                            cx.stop_propagation();
                            t.toggle_pin(ix, cx)
                        })),
                    )
                    .child(
                        icon_button(("h-edit", ix), "pencil", "Edit host", MUTED).on_click(
                            cx.listener(move |t, _, w, cx| {
                                cx.stop_propagation();
                                t.open_form(Some(ix), w, cx)
                            }),
                        ),
                    )
                    .child(
                        icon_button(("h-del", ix), "trash", "Delete host", RED).on_click(
                            cx.listener(move |t, _, _, cx| {
                                cx.stop_propagation();
                                t.delete_host(ix, cx)
                            }),
                        ),
                    ),
            )
    }

    /// Hosts as big square cards: pinned ones first, then one section per group, then the
    /// ungrouped ones. With a handful of servers this beats a list.
    fn home_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let hosts: Vec<(usize, &Host)> = self.cfg.hosts.iter().enumerate().collect();
        let pinned: Vec<usize> = hosts
            .iter()
            .filter(|(_, h)| h.pinned)
            .map(|(i, _)| *i)
            .collect();
        let mut groups: Vec<String> = hosts
            .iter()
            .filter(|(_, h)| !h.pinned)
            .filter_map(|(_, h)| h.group.clone())
            .collect();
        groups.sort_by_key(|g| g.to_lowercase());
        groups.dedup();
        let ungrouped: Vec<usize> = hosts
            .iter()
            .filter(|(_, h)| !h.pinned && h.group.is_none())
            .map(|(i, _)| *i)
            .collect();
        let sectioned = !pinned.is_empty() || !groups.is_empty();

        // Cascade only on launch. Coming back home later just fades the page,
        // so it shows up as fast as any other page.
        let mut n = 0;
        let mut cards = |ixs: &[usize], cx: &mut Context<Self>| -> Vec<AnyElement> {
            ixs.iter()
                .map(|&ix| {
                    let card = self.host_card(ix, &self.cfg.hosts[ix], cx);
                    n += 1;
                    if self.epoch == 0 {
                        stagger_in(card, ("card", ix), n - 1).into_any_element()
                    } else {
                        card.into_any_element()
                    }
                })
                .collect()
        };
        let grid = || div().flex().flex_wrap().gap_2();
        let header = |text: String| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c(MUTED))
                .child(text)
        };

        let mut page = div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_3();

        if !pinned.is_empty() {
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(header("PINNED".into()).child(icon("pin", 11., MUTED)))
                    .child(grid().children(cards(&pinned, cx))),
            );
        }
        for (gi, g) in groups.iter().enumerate() {
            let members: Vec<usize> = hosts
                .iter()
                .filter(|(_, h)| !h.pinned && h.group.as_ref() == Some(g))
                .map(|(i, _)| *i)
                .collect();
            let open = !self.collapsed.contains(g);
            let key = g.clone();
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        header(String::new())
                            .id(("group", gi))
                            .cursor_pointer()
                            .hover(|s| s.text_color(c(TEXT)))
                            .on_click(cx.listener(move |t, _, _, cx| {
                                if !t.collapsed.remove(&key) {
                                    t.collapsed.insert(key.clone());
                                }
                                cx.notify();
                            }))
                            .child(icon(
                                if open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                12.,
                                MUTED,
                            ))
                            .child(icon("folder", 12., MUTED))
                            .child(privacy::redact(&g.to_uppercase()).into_owned())
                            .child(
                                div()
                                    .text_color(c(PLACEHOLDER))
                                    .font_weight(FontWeight::NORMAL)
                                    .child(members.len().to_string()),
                            ),
                    )
                    .when(open, |d| d.child(grid().children(cards(&members, cx)))),
            );
        }

        let small_card = |id: &'static str, ic: &'static str, text: &'static str| {
            div()
                .id(id)
                .w(px(210.))
                .p_2()
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .rounded(px(2.))
                .border_2()
                .border_color(c(BORDER_VARIANT))
                .text_color(c(MUTED))
                .cursor_pointer()
                .hover(|s| s.bg(c(PANEL)).text_color(c(TEXT)).border_color(c(BORDER)))
                .child(icon(ic, 14., MUTED))
                .child(text)
        };
        page.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .when(sectioned && !ungrouped.is_empty(), |d| {
                    d.child(header("HOSTS".into()))
                })
                .child(
                    grid()
                        .children(cards(&ungrouped, cx))
                        .child(
                            small_card("add-host", "plus", "Add host")
                                .on_click(cx.listener(|t, _, w, cx| t.open_form(None, w, cx))),
                        )
                        .child(
                            small_card("local-docker", "container", "Local Docker")
                                .on_click(cx.listener(|t, _, w, cx| t.open_local_docker(w, cx))),
                        ),
                ),
        )
        .into_any_element()
    }

    /// The tab strip doubles as the title bar: empty space drags the window.
    fn tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let active = self.page == Page::Tab && ix == self.active;
            let mut text = tab.label.to_string();
            if let TabKind::Terminal(t) = &tab.kind
                && let Some(title) = t.read(cx).title()
                && !title.is_empty()
            {
                text = format!("{text} · {}", privacy::redact(&title));
            }
            let tab_icon = match tab.kind {
                TabKind::Terminal(_) => "terminal",
                TabKind::Docker(_) => "container",
                TabKind::Sftp(_) => "folder",
                TabKind::Logs(_) => "logs",
            };
            div()
                .id(("tab", ix))
                .group(format!("tab-{ix}"))
                .h_full()
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .max_w(px(220.))
                .border_r_1()
                .border_color(c(BORDER_VARIANT))
                .relative()
                .when(active, |d| {
                    d.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(px(2.))
                            .bg(c(ACCENT)),
                    )
                })
                .bg(c(BG))
                .text_color(c(if active { TEXT } else { MUTED }))
                .hover(|s| s.text_color(c(TEXT)))
                .on_click(cx.listener(move |t, _, w, cx| t.select_tab(ix, w, cx)))
                .child(icon(
                    tab_icon,
                    13.,
                    if active { ACCENT_TEXT } else { MUTED },
                ))
                .child(div().overflow_hidden().whitespace_nowrap().child(text))
                .child(
                    icon_button(("tab-close", ix), "x", "Close (⌘W)", MUTED)
                        .size(px(16.))
                        .on_click(cx.listener(move |t, _, w, cx| {
                            cx.stop_propagation();
                            t.close_tab(ix, w, cx)
                        })),
                )
        });
        div()
            .h(px(TITLEBAR_H))
            .flex_none()
            .flex()
            .bg(c(BG))
            .border_b_1()
            .border_color(c(BORDER_VARIANT))
            .child(
                div()
                    .id("home-tab")
                    .h_full()
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .border_x_1()
                    .border_color(c(BORDER_VARIANT))
                    .relative()
                    .when(self.page == Page::Home, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .h(px(2.))
                                .bg(c(ACCENT)),
                        )
                    })
                    .hover(|s| s.bg(c(PANEL)))
                    .tooltip(crate::ui::tip("Hosts"))
                    .on_click(cx.listener(|t, _, w, cx| t.show_page(Page::Home, w, cx)))
                    .child(icon(
                        "castle",
                        16.,
                        if self.page == Page::Home {
                            ACCENT_TEXT
                        } else {
                            MUTED
                        },
                    )),
            )
            .child(div().flex().h_full().children(tabs))
            .child(crate::window_drag::title_bar(
                div().id("title-drag").flex_1().h_full(),
            ))
            .child(
                div()
                    .h_full()
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .child(
                        icon_button(
                            "hide-btn",
                            if privacy::hide() { "eye-off" } else { "eye" },
                            "Hide sensitive information",
                            if privacy::hide() { ACCENT_TEXT } else { MUTED },
                        )
                        .on_click(cx.listener(|t, _, _, cx| t.toggle_hide(cx))),
                    )
                    .child(
                        icon_button(
                            "settings-btn",
                            "sliders",
                            "Settings (⌘,)",
                            if self.page == Page::Settings {
                                ACCENT_TEXT
                            } else {
                                MUTED
                            },
                        )
                        .on_click(cx.listener(|t, _, w, cx| t.open_settings(w, cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .h_full()
                    .border_l_1()
                    .border_color(c(BORDER_VARIANT))
                    .child(win_button("win-min", "minus", ACTIVE, |w| {
                        w.minimize_window()
                    }))
                    .child(win_button("win-max", "maximize", ACTIVE, |w| {
                        w.zoom_window()
                    }))
                    .child(win_button("win-close", "x", RED, |w| w.remove_window())),
            )
    }

    fn content(&self, cx: &mut Context<Self>) -> AnyElement {
        match (self.page, &self.settings) {
            (Page::Settings, Some(v)) => return v.clone().into_any_element(),
            (Page::Tab, _) if !self.tabs.is_empty() => {}
            _ => return self.home_view(cx),
        }
        match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Terminal(v)) => v.clone().into_any_element(),
            Some(TabKind::Docker(v)) => v.clone().into_any_element(),
            Some(TabKind::Sftp(v)) => v.clone().into_any_element(),
            Some(TabKind::Logs(v)) => v.clone().into_any_element(),
            None => div()
                .size_full()
                .flex()
                .flex_col()
                .gap_2()
                .items_center()
                .justify_center()
                .text_color(c(PLACEHOLDER))
                .child(icon("terminal", 28., ACTIVE))
                .child("Pick a host to open a shell")
                .into_any_element(),
        }
    }
}

impl Focusable for Root {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        static FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
        if FIRST.swap(false, std::sync::atomic::Ordering::Relaxed) {
            crate::mark("first render");
        }
        let overlay = |id: &'static str, child: AnyElement| {
            fade_in(
                div()
                    .absolute()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::hsla(0., 0., 0., 0.6))
                    .child(pop_in(div().child(child), (id, 1usize))),
                (id, 0usize),
            )
        };
        let trust_dialog = self.trust.as_ref().map(|req| {
            overlay(
                "trust",
                div()
                    .w(px(380.))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded(px(3.))
                    .bg(c(ELEVATED))
                    .border_1()
                    .border_color(c(BORDER))
                    .child(div().text_sm().child("Trust this host?"))
                    .child(
                        div().child(
                            privacy::redact(&format!(
                                "{}:{} has not been seen before.",
                                req.host,
                                privacy::port(req.port)
                            ))
                            .into_owned(),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(c(MUTED))
                            .child("Fingerprint (SHA256)"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_family(mono())
                            .child(req.fingerprint.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(button("trust-no", "Cancel").on_click(cx.listener(
                                |t, _, _, cx| {
                                    if let Some(r) = t.trust.take() {
                                        let _ = r.reply.send(false);
                                    }
                                    cx.notify();
                                },
                            )))
                            .child(primary_button("trust-yes", "Trust and connect").on_click(
                                cx.listener(|t, _, _, cx| {
                                    if let Some(r) = t.trust.take() {
                                        let _ = r.reply.send(true);
                                    }
                                    cx.notify();
                                }),
                            )),
                    )
                    .into_any_element(),
            )
        });
        let form = self
            .form
            .clone()
            .map(|f| overlay("form", f.into_any_element()));

        div()
            .id("root")
            .track_focus(&self.focus)
            .key_context("Root")
            .on_action(cx.listener(|t, _: &CloseTab, w, cx| match t.page {
                Page::Tab => t.close_tab(t.active, w, cx),
                Page::Settings => t.show_page(Page::Home, w, cx),
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
            .on_action(cx.listener(|t, _: &NewHost, w, cx| t.open_form(None, w, cx)))
            .on_action(cx.listener(|t, _: &OpenSettings, w, cx| t.open_settings(w, cx)))
            .size_full()
            .flex()
            .font_family(ui())
            .text_size(px(12.))
            .bg(c(BG))
            .text_color(c(TEXT))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.tab_bar(cx))
                    .child(div().flex_1().min_h_0().child(fade_in(
                        div().size_full().child(self.content(cx)),
                        ("page", self.epoch as usize),
                    )))
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
                            div()
                                .px_2()
                                .py(px(3.))
                                .border_t_1()
                                .border_color(c(BORDER_VARIANT))
                                .text_color(c(if err { RED } else { MUTED }))
                                .child(privacy::redact(&msg).into_owned()),
                        )
                    }),
            )
            .children(trust_dialog)
            .children(form)
    }
}

/// Square, full-height window control for the custom title bar.
fn win_button(
    id: &'static str,
    name: &str,
    hover: u32,
    action: impl Fn(&mut Window) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w(px(40.))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(c(hover)))
        .on_click(move |_, w, _| action(w))
        .child(icon(name, 14., TEXT))
}

/// Hosts and usernames (and the local user) that "hide sensitive information" masks.
fn refresh_privacy_tokens(cfg: &Config) {
    let hosts = cfg.hosts.iter().map(|h| h.host.clone()).collect();
    let mut users: Vec<String> = cfg.hosts.iter().map(|h| h.user.clone()).collect();
    if let Some(u) = std::env::var_os("USER") {
        users.push(u.to_string_lossy().into_owned());
    }
    privacy::set_known(hosts, users);
}
