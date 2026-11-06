//! Connecting to hosts: credentials, the host key prompt, the connect timeout, and handing the
//! connection to whatever asked for it.

use super::*;

impl GuiPrompt {
    /// Block until the user answers the host key question in the UI.
    fn ask(&self, host: &str, port: u16, fingerprint: &str, previous: Option<&str>) -> bool {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = TrustRequest {
            attempt: self.attempt,
            host: host.into(),
            port,
            fingerprint: fingerprint.into(),
            previous: previous.map(Into::into),
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

impl TrustPrompt for GuiPrompt {
    fn trust_new_host(&self, host: &str, port: u16, fingerprint: &str) -> bool {
        self.ask(host, port, fingerprint, None)
    }

    fn replace_changed_host(
        &self,
        host: &str,
        port: u16,
        previous: &str,
        fingerprint: &str,
    ) -> bool {
        self.ask(host, port, fingerprint, Some(previous))
    }
}

impl AuthPrompt for GuiAsk {
    fn answer(&self, host: &str, text: &str, questions: &[Question]) -> Option<Vec<String>> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = CodeRequest {
            attempt: self.attempt,
            host: host.into(),
            text: text.into(),
            questions: questions.to_vec(),
            reply: tx,
        };
        self.tx.send(req).ok()?;
        // Time to open an authenticator app doesn't count against the connect timeout.
        self.prompting.store(true, Ordering::Relaxed);
        let answers = rx.recv_timeout(Duration::from_secs(300)).ok().flatten();
        self.prompting.store(false, Ordering::Relaxed);
        answers
    }
}

/// Run `work` until it finishes, the cancel sender is dropped, or `timeout_secs` (0 = never)
/// pass. Time spent in the host key prompt does not count. `None` means cancelled.
async fn race<T>(
    work: impl Future<Output = ssh_core::Result<T>>,
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

impl Workspace {
    pub(super) fn open_host(
        &mut self,
        ix: usize,
        what: Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.cfg.hosts.get(ix).cloned() else {
            return;
        };
        self.connect(host, None, what, window, cx);
    }

    /// Connect `host` (or reuse its live connection) and open `what` on it. `creds` are used
    /// as given (quick connect); `None` loads them from the secret store, asking for the
    /// master password first when the secrets file is locked.
    pub(super) fn connect(
        &mut self,
        host: Host,
        creds: Option<Credentials>,
        what: Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let account = host.account();
        if let Some(live) = self.conns.get(&account) {
            if !live.conn.is_closed() {
                let (conn, pw) = (live.conn.clone(), live.password.clone());
                self.finish_open(host, conn, pw, what, window, cx);
                return;
            }
            self.conns.remove(&account);
        }
        if let Some(p) = self.pending.iter_mut().find(|p| p.account == account) {
            // Already connecting: open this one too once the connection is up.
            p.queued.push(what);
            return;
        }
        let chain = match self.cfg.jump_chain(&host) {
            Ok(c) => c,
            Err(e) => {
                match &what {
                    Open::Reattach(v, ..) => {
                        v.update(cx, |v, cx| v.set_notice(Some(e), true, cx)).ok();
                    }
                    _ => self.set_status(format!("{}: {e}", host.name), true),
                }
                return;
            }
        };
        // Jump hosts already open are used as they are.
        let hops: Vec<(Host, Option<Arc<Connection>>)> = chain
            .into_iter()
            .map(|h| {
                let live = self
                    .conns
                    .get(&h.account())
                    .filter(|l| !l.conn.is_closed())
                    .map(|l| l.conn.clone());
                (h, live)
            })
            .collect();
        // Agent hosts keep nothing in the secret store, so a locked file doesn't matter.
        let needs_secrets = (creds.is_none() && host.auth != AuthMethod::Agent)
            || hops
                .iter()
                .any(|(h, live)| live.is_none() && h.auth != AuthMethod::Agent);
        if needs_secrets && !secrets::ready() {
            if let Open::Reattach(v, ..) = &what {
                // Don't pop a dialog up on its own during an automatic reconnect.
                v.update(cx, |v, cx| {
                    v.set_notice(
                        Some("Connection lost. The secrets file is locked. Press Enter to unlock and reconnect.".into()),
                        true,
                        cx,
                    )
                })
                .ok();
                return;
            }
            self.with_secrets(
                move |this, window, cx| this.connect(host, None, what, window, cx),
                window,
                cx,
            );
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
            queued: Vec::new(),
            _cancel: cancel_tx,
        });
        self.status = None;
        cx.notify();
        let prompting = Arc::new(AtomicBool::new(false));
        let verifier = HostKeyVerifier {
            known_hosts: HostKeyVerifier::default_path()
                .unwrap_or_else(|_| PathBuf::from("known_hosts")),
            prompt: Arc::new(GuiPrompt {
                attempt: id,
                tx: self.trust_tx.clone(),
                prompting: prompting.clone(),
            }),
        };
        // An automatic reconnect asks nothing: a one-time code can't be reused anyway.
        let prompt = (!matches!(what, Open::Reattach(_, _, false))).then(|| {
            Arc::new(GuiAsk {
                attempt: id,
                tx: self.code_tx.clone(),
                prompting: prompting.clone(),
            }) as Arc<dyn AuthPrompt>
        });
        let prefs = crate::prefs::get();
        let (keepalive_secs, timeout_secs) = (prefs.keepalive_secs(), prefs.connect_timeout_secs());
        cx.spawn_in(window, async move |this, cx| {
            let h = host.clone();
            let r = runtime::spawn(async move {
                // Keychain access can block on a system prompt, keep it off the async workers.
                // It is not timed: macOS may be waiting for the user to allow access.
                let load = |host: &Host| {
                    let host = host.clone();
                    tokio::task::spawn_blocking(move || Credentials::from_store(&host))
                };
                let creds = match creds {
                    Some(c) => c,
                    None => tokio::select! {
                        r = load(&h) => r.map_err(|e| ssh_core::Error::Remote(e.to_string()))??,
                        _ = &mut cancel_rx => return Ok(None),
                    },
                };
                // Only the hops after the nearest open one are dialed.
                let skip = hops.iter().rposition(|(_, live)| live.is_some()).unwrap_or(0);
                let mut chain = Vec::new();
                for (hop, live) in hops.into_iter().skip(skip) {
                    let way = match live {
                        Some(conn) => Ok(conn),
                        None => tokio::select! {
                            r = load(&hop) => Err(r.map_err(|e| ssh_core::Error::Remote(e.to_string()))??),
                            _ = &mut cancel_rx => return Ok(None),
                        },
                    };
                    chain.push((hop, way));
                }
                let pw = match &creds {
                    Credentials::Password(p) => Some(p.clone()),
                    _ => None,
                };
                let options = |via| Options {
                    keepalive: Duration::from_secs(keepalive_secs as u64),
                    via,
                    prompt: prompt.clone(),
                };
                let work = async {
                    let mut via = None;
                    for (hop, way) in chain {
                        via = Some(match way {
                            Ok(conn) => conn,
                            Err(hop_creds) => Arc::new(
                                Connection::connect_with(
                                    &hop,
                                    &hop_creds,
                                    verifier.clone(),
                                    options(via.take()),
                                )
                                .await?,
                            ),
                        });
                    }
                    Connection::connect_with(&h, &creds, verifier, options(via)).await
                };
                let conn = race(work, &mut cancel_rx, timeout_secs, &prompting).await?;
                Ok::<_, ssh_core::Error>(conn.map(|c| (c, pw)))
            })
            .await;
            this.update_in(cx, |this, window, cx| {
                // Cancelled (X, host deleted) while connecting: drop whatever came back.
                let Some(pos) = this.pending.iter().position(|p| p.id == id) else {
                    return;
                };
                let queued = std::mem::take(&mut this.pending.remove(pos).queued);
                let exists = this.cfg.hosts.iter().any(|h| h.account() == account)
                    || this.quick.contains_key(&account);
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
                        for what in std::iter::once(what).chain(queued) {
                            this.finish_open(
                                host.clone(),
                                conn.clone(),
                                pw.clone(),
                                what,
                                window,
                                cx,
                            );
                        }
                    }
                    Ok(_) => {
                        // Cancelled: a terminal waiting to reconnect lets the user retry by hand.
                        for w in std::iter::once(&what).chain(&queued) {
                            if let Open::Reattach(v, ..) = w {
                                v.update(cx, |v, cx| {
                                    v.set_notice(
                                        Some("Reconnect cancelled. Press Enter to retry.".into()),
                                        true,
                                        cx,
                                    )
                                })
                                .ok();
                            }
                        }
                    }
                    Err(e) => {
                        this.dismiss_trust(id);
                        this.dismiss_codes(id);
                        use ssh_core::Error as E;
                        // Wrong password or a changed host key will not fix itself: stop.
                        let fatal = matches!(
                            e,
                            E::AuthFailed
                                | E::CodeRequired
                                | E::LoginCancelled
                                | E::HostKeyChanged { .. }
                                | E::HostKeyRejected { .. }
                                | E::PassphraseRequired
                                | E::NoCredentials(_)
                                | E::Locked
                                | E::WrongMasterPassword
                                | E::Vault(_)
                        );
                        let msg = e.to_string();
                        let mut other = false;
                        for w in std::iter::once(&what).chain(&queued) {
                            match w {
                                Open::Reattach(v, ..) if fatal => {
                                    v.update(cx, |v, cx| {
                                        v.set_notice(
                                            Some(format!(
                                                "Reconnect failed: {msg}. Press Enter to retry."
                                            )),
                                            true,
                                            cx,
                                        )
                                    })
                                    .ok();
                                }
                                Open::Reattach(v, n, _) => this.start_reconnect(
                                    v.clone(),
                                    n + 1,
                                    Some(msg.clone()),
                                    false,
                                    window,
                                    cx,
                                ),
                                _ => other = true,
                            }
                        }
                        if other {
                            this.set_status(format!("{}: {e}", host.name), true);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stop connecting to `account`. The task notices its cancel sender is gone.
    pub(super) fn cancel_connect(&mut self, account: &str, cx: &mut Context<Self>) {
        if let Some(pos) = self.pending.iter().position(|p| p.account == account) {
            let p = self.pending.remove(pos);
            self.dismiss_trust(p.id);
            self.dismiss_codes(p.id);
            cx.notify();
        }
    }

    /// Answer "no" to the host key questions of connect `attempt` (cancelled or failed).
    fn dismiss_trust(&mut self, attempt: u64) {
        self.trust.retain(|t| {
            let hit = t.attempt == attempt;
            if hit {
                let _ = t.reply.send(false);
            }
            !hit
        });
    }

    fn finish_open(
        &mut self,
        host: Host,
        conn: Arc<Connection>,
        password: Option<Secret>,
        what: Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match what {
            Open::Shell => self.open_shell(host.name, conn, password, None, window, cx),
            Open::SplitShell(tab) => {
                self.spawn_shell(host.name, conn, password, None, Some(tab), window, cx)
            }
            Open::Docker => self.open_docker(host, conn, password, window, cx),
            Open::Sftp => self.open_sftp(host, conn, window, cx),
            Open::SftpSource(view, side) => self.attach_sftp(host, conn, view, side, window, cx),
            Open::Reattach(view, attempt, _) => {
                self.reattach(view, attempt, conn, password, window, cx)
            }
        }
    }

    /// Show the login question in front of the queue.
    pub(super) fn show_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(req) = self.codes.front() else {
            return;
        };
        let dialog = cx.new(|cx| CodeDialog::new(&req.host, &req.text, &req.questions, cx));
        cx.subscribe_in(&dialog, window, |this, _, ev: &CodeEvent, window, cx| {
            let answers = match ev {
                CodeEvent::Answered(a) => Some(a.clone()),
                CodeEvent::Cancelled => None,
            };
            if let Some(req) = this.codes.pop_front() {
                let _ = req.reply.send(answers);
            }
            this.code = None;
            window.focus(&this.focus, cx);
            cx.notify();
        })
        .detach();
        dialog.update(cx, |d, cx| d.focus_first(window, cx));
        self.code = Some(dialog);
        cx.notify();
    }

    /// Cancel the login questions of connect `attempt` (cancelled or failed).
    pub(super) fn dismiss_codes(&mut self, attempt: u64) {
        if self.codes.front().is_some_and(|r| r.attempt == attempt) {
            self.code = None;
        }
        self.codes.retain(|r| {
            let hit = r.attempt == attempt;
            if hit {
                let _ = r.reply.send(None);
            }
            !hit
        });
    }
}
