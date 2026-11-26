//! What opens on a connection: shells (tabs or split panes), files and Docker tabs.

use super::*;

impl Workspace {
    pub(super) fn open_shell(
        &mut self,
        label: String,
        conn: Arc<Connection>,
        password: Option<Secret>,
        command: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_shell(label, conn, password, command, None, window, cx);
    }

    /// Open a shell as a new tab, or as a new pane of tab `split_into` (a tab id).
    #[expect(
        clippy::too_many_arguments,
        reason = "each is a separate input, a struct would only be built to be taken apart"
    )]
    pub(super) fn spawn_shell(
        &mut self,
        label: String,
        conn: Arc<Connection>,
        password: Option<Secret>,
        command: Option<String>,
        split_into: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let account = self
            .conns
            .iter()
            .find(|(_, l)| Arc::ptr_eq(&l.conn, &conn))
            .map(|(k, _)| k.clone())
            .unwrap_or_default();
        let saved_command = command.clone();
        cx.spawn_in(window, async move |this, cx| {
            let c = conn.clone();
            let r = runtime::spawn(async move { c.open_shell(100, 30, command.as_deref()).await })
                .await;
            this.update_in(cx, |this, window, cx| match r {
                Ok(shell) => {
                    let view = cx.new(|cx| {
                        let mut v = TerminalView::new(shell, password.clone(), cx);
                        v.command = saved_command;
                        v
                    });
                    this.watch_terminal(&view, window, cx);
                    let pane = Pane {
                        view,
                        conn,
                        account,
                        password,
                    };
                    let target =
                        split_into.and_then(|id| this.tabs.iter().position(|t| t.id == id));
                    // A tab that filled up while connecting gets a tab of its own instead.
                    if let Some(ix) = target
                        && let TabKind::Terminal(t) = &mut this.tabs[ix].kind
                        && t.panes.len() < split::max_panes()
                    {
                        t.panes.push(PaneView::Shell(pane));
                        t.focused = t.panes.len() - 1;
                        this.select_tab(ix, window, cx);
                        return;
                    }
                    this.push_tab(
                        label,
                        TabKind::Terminal(TermTab::new(vec![PaneView::Shell(pane)])),
                        None,
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

    /// A Docker tab for the host.
    pub(super) fn open_docker(
        &mut self,
        host: Host,
        conn: Arc<Connection>,
        password: Option<Secret>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = format!("{} · docker", host.name);
        let account = host.account();
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
        self.push_tab(label, TabKind::Docker(view), Some(account), window, cx);
    }

    /// A files tab: this Mac on the left, the host on the right.
    pub(super) fn open_sftp(
        &mut self,
        host: Host,
        conn: Arc<Connection>,
        split_into: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                    let remote = SftpSource::Remote(Box::new(RemoteRef {
                        label: host.name.clone(),
                        host: host.clone(),
                        sftp: Arc::new(sftp),
                        conn: conn.clone(),
                    }));
                    let view = cx.new(|cx| SftpView::new(SftpSource::Local, remote, names, cx));
                    cx.subscribe_in(&view, window, |this, view, ev: &SftpEvent, window, cx| {
                        this.on_sftp_event(view.downgrade(), ev, window, cx)
                    })
                    .detach();
                    view.update(cx, SftpView::load);
                    let kind = TabKind::Sftp(view);
                    let target =
                        split_into.and_then(|id| this.tabs.iter().position(|t| t.id == id));
                    if let Some(ix) = target
                        && let TabKind::Terminal(t) = &this.tabs[ix].kind
                        && t.panes.len() < split::max_panes()
                    {
                        let pane = this.new_tab(label, kind, Some(host.account()), cx);
                        if let TabKind::Terminal(t) = &mut this.tabs[ix].kind {
                            t.panes.push(PaneView::View(Box::new(pane)));
                            t.focused = t.panes.len() - 1;
                        }
                        this.select_tab(ix, window, cx);
                        return;
                    }
                    this.push_tab(label, kind, Some(host.account()), window, cx);
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

    /// Show the host in one side of an open files tab.
    pub(super) fn attach_sftp(
        &mut self,
        host: Host,
        conn: Arc<Connection>,
        view: gpui::WeakEntity<SftpView>,
        side: SftpSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let c = conn.clone();
            let r = runtime::spawn(async move { c.open_sftp().await }).await;
            this.update_in(cx, |this, _, cx| match r {
                Ok(sftp) => {
                    let src = SftpSource::Remote(Box::new(RemoteRef {
                        label: host.name.clone(),
                        host: host.clone(),
                        sftp: Arc::new(sftp),
                        conn: conn.clone(),
                    }));
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

    fn on_docker_event(
        &mut self,
        ev: &DockerEvent,
        remote: Option<(String, Arc<Connection>, Option<Secret>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            DockerEvent::Shell { id, name } => {
                let Some((host, conn, pw)) = remote else {
                    return;
                };
                match docker::exec_shell_command(id) {
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
                    format!("{name} · logs"),
                    TabKind::Logs(view),
                    None,
                    window,
                    cx,
                );
            }
        }
    }

    pub(super) fn open_local_docker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| DockerView::new(Backend::Local, false, cx));
        cx.subscribe_in(&view, window, |this, _, ev: &DockerEvent, window, cx| {
            this.on_docker_event(ev, None, window, cx)
        })
        .detach();
        self.push_tab(
            "Local Docker".into(),
            TabKind::Docker(view),
            None,
            window,
            cx,
        );
    }
}
