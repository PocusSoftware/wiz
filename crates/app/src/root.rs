use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement,
    Render, SharedString, Window, WindowControlArea, actions, div, prelude::*, px,
};
use ssh_core::config::{AuthMethod, Config, Host};
use ssh_core::docker::{self, Backend};
use ssh_core::hostkey::{HostKeyVerifier, TrustPrompt};
use ssh_core::keychain::{self, Kind};
use ssh_core::session::{Connection, Credentials};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::docker_view::{DockerEvent, DockerView};
use crate::host_form::{FormEvent, HostForm};
use crate::runtime;
use crate::terminal_view::TerminalView;
use crate::theme::*;
use crate::ui::{button, icon, icon_button, primary_button};

actions!(app, [Quit, CloseTab, NextTab, PrevTab, NewHost]);

struct TrustRequest {
    host: String,
    port: u16,
    fingerprint: String,
    reply: std::sync::mpsc::SyncSender<bool>,
}

/// Called from the tokio side during the handshake. Blocks that worker until the
/// user answers in the UI.
struct GuiPrompt(UnboundedSender<TrustRequest>);

impl TrustPrompt for GuiPrompt {
    fn trust_new_host(&self, host: &str, port: u16, fingerprint: &str) -> bool {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let req = TrustRequest {
            host: host.into(),
            port,
            fingerprint: fingerprint.into(),
            reply: tx,
        };
        if self.0.send(req).is_err() {
            return false;
        }
        rx.recv_timeout(Duration::from_secs(120)).unwrap_or(false)
    }
}

struct Live {
    conn: Arc<Connection>,
    password: Option<String>,
}

enum Open {
    Shell,
    Docker,
}

enum TabKind {
    Terminal(Entity<TerminalView>),
    Docker(Entity<DockerView>),
    Logs(Vec<SharedString>),
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
    /// Showing the hosts page instead of a session tab.
    home: bool,
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
                status = Some((format!("Could not read hosts: {e}").into(), true));
                Config::default()
            }
        };
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
            home: true,
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
        self.active = ix;
        self.home = false;
        match &self.tabs[ix].kind {
            TabKind::Terminal(t) => t.read(cx).focus(window),
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
        if self.tabs.is_empty() {
            self.active = 0;
            self.home = true;
            window.focus(&self.focus);
            cx.notify();
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
        self.set_status(format!("Connecting to {}...", host.name), false);
        cx.notify();
        let verifier = HostKeyVerifier {
            known_hosts: HostKeyVerifier::default_path()
                .unwrap_or_else(|_| PathBuf::from("known_hosts")),
            prompt: Arc::new(GuiPrompt(self.trust_tx.clone())),
        };
        cx.spawn_in(window, async move |this, cx| {
            let h = host.clone();
            let r = runtime::spawn(async move {
                // Keychain access can block on a system prompt, keep it off the async workers.
                let hk = h.clone();
                let creds = tokio::task::spawn_blocking(move || Credentials::from_keychain(&hk))
                    .await
                    .map_err(|e| ssh_core::Error::Remote(e.to_string()))??;
                let pw = match &creds {
                    Credentials::Password(p) => Some(p.clone()),
                    _ => None,
                };
                let conn = Connection::connect(&h, &creds, verifier).await?;
                Ok::<_, ssh_core::Error>((conn, pw))
            })
            .await;
            this.update_in(cx, |this, window, cx| match r {
                Ok((conn, pw)) => {
                    let conn = Arc::new(conn);
                    this.conns.insert(
                        host.account(),
                        Live {
                            conn: conn.clone(),
                            password: pw.clone(),
                        },
                    );
                    this.status = None;
                    this.finish_open(host, conn, pw, what, window, cx);
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
            DockerEvent::Logs { name, text } => {
                let lines: Vec<SharedString> = text
                    .lines()
                    .map(|l| l.replace('\t', "    ").into())
                    .collect();
                self.push_tab(
                    Tab {
                        label: format!("{name} · logs").into(),
                        kind: TabKind::Logs(lines),
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
        let form = cx.new(|cx| HostForm::new(ex, cx));
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
        for kind in [Kind::Password, Kind::KeyPassphrase] {
            if let Err(e) = keychain::delete(kind, &acct) {
                self.set_status(format!("Keychain: {e}"), true);
            }
        }
        self.conns.remove(&acct);
        self.save_config();
        cx.notify();
    }

    // ---------- render ----------

    /// Hosts as big square cards. With a handful of servers this beats a list.
    fn home_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let cards = self.cfg.hosts.iter().enumerate().map(|(ix, h)| {
            let live = self
                .conns
                .get(&h.account())
                .is_some_and(|l| !l.conn.is_closed());
            let kind_icon = match h.auth {
                AuthMethod::Password => "server",
                AuthMethod::Key { .. } => "key",
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
                    cx.listener(move |t, _: &ClickEvent, w, cx| {
                        t.open_host(ix, Open::Shell, w, cx)
                    }),
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
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .whitespace_nowrap()
                                .child(h.name.clone()),
                        )
                        .child(div().size(px(7.)).bg(c(if live { GREEN } else { ACTIVE }))),
                )
                .child(
                    div()
                        .text_color(c(MUTED))
                        .whitespace_nowrap()
                        .child(format!("{}@{}:{}", h.user, h.host, h.port)),
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
                        .child(div().flex_1())
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
        });

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

        div()
            .size_full()
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(cards)
                    .child(
                        small_card("add-host", "plus", "Add host")
                            .on_click(cx.listener(|t, _, w, cx| t.open_form(None, w, cx))),
                    )
                    .child(
                        small_card("local-docker", "container", "Local Docker")
                            .on_click(cx.listener(|t, _, w, cx| t.open_local_docker(w, cx))),
                    ),
            )
            .into_any_element()
    }

    /// The tab strip doubles as the title bar: empty space drags the window.
    fn tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let active = !self.home && ix == self.active;
            let mut text = tab.label.to_string();
            if let TabKind::Terminal(t) = &tab.kind
                && let Some(title) = t.read(cx).title()
                && !title.is_empty()
            {
                text = format!("{text} · {title}");
            }
            let tab_icon = match tab.kind {
                TabKind::Terminal(_) => "terminal",
                TabKind::Docker(_) => "container",
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
                .when(active, |d| d.border_b_2().border_color(c(ACCENT)))
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
                    .w(px(TRAFFIC_LIGHTS_W))
                    .h_full()
                    .flex_none()
                    .window_control_area(WindowControlArea::Drag),
            )
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
                    .when(self.home, |d| d.border_b_2().border_color(c(ACCENT)))
                    .hover(|s| s.bg(c(PANEL)))
                    .tooltip(crate::ui::tip("Hosts"))
                    .on_click(cx.listener(|t, _, w, cx| {
                        t.home = true;
                        w.focus(&t.focus);
                        cx.notify();
                    }))
                    .child(icon(
                        "server",
                        14.,
                        if self.home { ACCENT_TEXT } else { MUTED },
                    )),
            )
            .child(div().flex().h_full().children(tabs))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
    }

    fn content(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.home || self.tabs.is_empty() {
            return self.home_view(cx);
        }
        match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Terminal(v)) => v.clone().into_any_element(),
            Some(TabKind::Docker(v)) => v.clone().into_any_element(),
            Some(TabKind::Logs(lines)) => div()
                .id("logs")
                .size_full()
                .overflow_scroll()
                .p_3()
                .font_family(mono())
                .text_xs()
                .children(
                    lines
                        .iter()
                        .map(|l| div().whitespace_nowrap().child(l.clone())),
                )
                .into_any_element(),
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
        let overlay = |child: AnyElement| {
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::hsla(0., 0., 0., 0.6))
                .child(child)
        };
        let trust_dialog = self.trust.as_ref().map(|req| {
            overlay(
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
                    .child(div().child(format!(
                        "{}:{} has not been seen before.",
                        req.host, req.port
                    )))
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
        let form = self.form.clone().map(|f| overlay(f.into_any_element()));

        div()
            .id("root")
            .track_focus(&self.focus)
            .key_context("Root")
            .on_action(cx.listener(|t, _: &CloseTab, w, cx| t.close_tab(t.active, w, cx)))
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
            .size_full()
            .flex()
            .font_family(FONT_UI)
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
                    .child(div().flex_1().min_h_0().child(self.content(cx)))
                    .when_some(self.status.clone(), |d, (msg, err)| {
                        d.child(
                            div()
                                .px_2()
                                .py(px(3.))
                                .border_t_1()
                                .border_color(c(BORDER_VARIANT))
                                .text_color(c(if err { RED } else { MUTED }))
                                .child(msg),
                        )
                    }),
            )
            .children(trust_dialog)
            .children(form)
    }
}
