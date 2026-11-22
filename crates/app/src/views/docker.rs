//! Container list with start/stop/restart/remove, logs, and (remote only) a shell.

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::time::Duration;

use gpui::{
    ClickEvent, Context, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*,
    px,
};
use ssh_core::docker::{Action, Backend, Container};

use crate::runtime;
use crate::theme::*;
use crate::widgets::icon_button;

pub enum DockerEvent {
    Shell {
        id: String,
        name: String,
    },
    Logs {
        name: String,
        id: String,
        backend: Backend,
    },
}

impl EventEmitter<DockerEvent> for DockerView {}

pub struct DockerView {
    backend: Backend,
    can_shell: bool,
    items: Vec<Container>,
    busy: HashSet<String>,
    confirm_remove: Option<String>,
    error: Option<SharedString>,
    loaded: bool,
}

impl DockerView {
    /// True for a server's Docker (over SSH), false for the local one.
    pub fn is_remote(&self) -> bool {
        matches!(self.backend, Backend::Remote(_))
    }

    pub fn new(backend: Backend, can_shell: bool, cx: &mut Context<Self>) -> Self {
        let this = Self {
            backend,
            can_shell,
            items: vec![],
            busy: HashSet::new(),
            confirm_remove: None,
            error: None,
            loaded: false,
        };
        this.refresh(cx);
        cx.spawn(async move |this, cx| {
            loop {
                let secs = crate::prefs::get().docker_refresh_secs().max(1) as u64;
                cx.background_executor()
                    .timer(Duration::from_secs(secs))
                    .await;
                if this.update(cx, |t, cx| t.refresh(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn refresh(&self, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let r = runtime::spawn(async move { backend.list().await }).await;
            this.update(cx, |t, cx| {
                t.loaded = true;
                match r {
                    Ok(mut items) => {
                        items.sort_by(|a, b| {
                            b.is_running()
                                .cmp(&a.is_running())
                                .then(a.name.cmp(&b.name))
                        });
                        t.items = items;
                        t.error = None;
                    }
                    Err(e) => t.error = Some(e.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn act(&mut self, id: String, action: Action, cx: &mut Context<Self>) {
        self.busy.insert(id.clone());
        self.confirm_remove = None;
        let backend = self.backend.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let id2 = id.clone();
            let r = runtime::spawn(async move { backend.act(&id2, action).await }).await;
            this.update(cx, |t, cx| {
                t.busy.remove(&id);
                if let Err(e) = r {
                    t.error = Some(e.to_string().into());
                }
                t.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn logs(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        cx.emit(DockerEvent::Logs {
            name,
            id,
            backend: self.backend.clone(),
        });
    }
}

impl Render for DockerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.items.iter().enumerate().map(|(ix, ct)| {
            let busy = self.busy.contains(&ct.id);
            let running = ct.is_running();
            let id = ct.id.clone();
            let name = ct.name.clone();
            let confirming = self.confirm_remove.as_deref() == Some(ct.id.as_str());
            let (log_id, log_name) = (ct.id.clone(), ct.name.clone());

            let mut actions = div().flex().gap_0p5().items_center();
            if busy {
                actions = actions.child(
                    div()
                        .text_xs()
                        .text_color(c(palette().placeholder))
                        .child("working..."),
                );
            } else {
                if running {
                    let i = id.clone();
                    actions = actions
                        .child(
                            icon_button(("stop", ix), "stop", "Stop", palette().muted).on_click(
                                cx.listener(move |t, _, _, cx| t.act(i.clone(), Action::Stop, cx)),
                            ),
                        )
                        .child(
                            icon_button(("restart", ix), "refresh", "Restart", palette().muted)
                                .on_click({
                                    let i = id.clone();
                                    cx.listener(move |t, _, _, cx| {
                                        t.act(i.clone(), Action::Restart, cx)
                                    })
                                }),
                        );
                    if self.can_shell {
                        let (i, n) = (id.clone(), name.clone());
                        actions = actions.child(
                            icon_button(("shell", ix), "terminal", "Open shell", palette().muted)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(DockerEvent::Shell {
                                        id: i.clone(),
                                        name: n.clone(),
                                    })
                                })),
                        );
                    }
                } else {
                    let i = id.clone();
                    actions = actions.child(
                        icon_button(("start", ix), "play", "Start", palette().muted).on_click(
                            cx.listener(move |t, _, _, cx| t.act(i.clone(), Action::Start, cx)),
                        ),
                    );
                }
                let (i, n) = (id.clone(), name);
                actions = actions.child(
                    icon_button(("logs", ix), "logs", "Logs", palette().muted)
                        .on_click(cx.listener(move |t, _, _, cx| t.logs(i.clone(), n.clone(), cx))),
                );
                let i = id;
                let rm = icon_button(
                    ("rm", ix),
                    "trash",
                    if confirming {
                        "Click again to remove"
                    } else {
                        "Remove"
                    },
                    if confirming {
                        palette().red
                    } else {
                        palette().muted
                    },
                )
                .when(confirming, |b| b.bg(with_alpha(danger(), 0.15)))
                .on_click(cx.listener(move |t, _, _, cx| {
                    if t.confirm_remove.as_deref() == Some(i.as_str()) {
                        t.act(i.clone(), Action::Remove, cx);
                    } else {
                        t.confirm_remove = Some(i.clone());
                        cx.notify();
                    }
                }));
                actions = actions.child(rm);
            }

            div()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .h(px(26.))
                .border_b_1()
                .border_color(c(palette().border_variant))
                .hover(|s| s.bg(c(palette().hover)))
                .child(
                    div()
                        .size(px(6.))
                        .flex_none()
                        .rounded(px(1.))
                        .bg(c(dot_color(if running {
                            palette().green
                        } else {
                            palette().active
                        }))),
                )
                .child(
                    // Double click opens the logs. Only here, so quick clicks on the
                    // action buttons don't open them too.
                    div()
                        .id(("ct", ix))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .gap_2()
                        .on_click(cx.listener(move |t, ev: &ClickEvent, _, cx| {
                            if ev.click_count() >= 2 {
                                t.logs(log_id.clone(), log_name.clone(), cx);
                            }
                        }))
                        .child(div().whitespace_nowrap().child(ct.name.clone()))
                        .child(
                            div()
                                .text_color(c(palette().placeholder))
                                .whitespace_nowrap()
                                .child(summary(ct)),
                        ),
                )
                .child(actions)
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(c(palette().border_variant))
                    .child(
                        div()
                            .text_xs()
                            .text_color(c(palette().muted))
                            .child(format!("{} containers", self.items.len())),
                    )
                    .child(
                        icon_button("refresh", "refresh", "Refresh", palette().muted)
                            .on_click(cx.listener(|t, _, _, cx| t.refresh(cx))),
                    ),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(c(palette().red))
                        .child(crate::privacy::redact(&e).into_owned()),
                )
            })
            .when(
                self.loaded && self.items.is_empty() && self.error.is_none(),
                |d| {
                    d.child(
                        div()
                            .p_2()
                            .text_color(c(palette().placeholder))
                            .child("No containers."),
                    )
                },
            )
            .child(
                div()
                    .id("containers")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(rows),
            )
    }
}

/// "image · status · ports" with noise removed: short hashes, no IPv6 duplicates.
fn summary(ct: &Container) -> String {
    let image = match ct.image.strip_prefix("sha256:") {
        Some(h) => h.chars().take(12).collect(),
        None => ct.image.clone(),
    };
    let mut ports: Vec<String> = Vec::new();
    for p in ct.ports.split(", ") {
        if p.is_empty() || p.starts_with("[::]") {
            continue;
        }
        let p = p
            .trim_start_matches("0.0.0.0:")
            .trim_end_matches("/tcp")
            .to_string();
        if !ports.contains(&p) {
            ports.push(p);
        }
    }
    let mut out = format!("{image} · {}", ct.status);
    if !ports.is_empty() {
        out.push_str(" · ");
        let shown: Vec<String> = ports
            .iter()
            .map(|p| {
                p.split("->")
                    .map(port_mapping)
                    .collect::<Vec<_>>()
                    .join("\u{2192}")
            })
            .collect();
        out.push_str(&shown.join(", "));
    }
    crate::privacy::redact(&out).into_owned()
}

/// One side of a port mapping (`127.0.0.1:5432` or `80`) with the port masked when hidden.
fn port_mapping(side: &str) -> String {
    match side.rsplit_once(':') {
        Some((addr, port)) => format!("{addr}:{}", crate::privacy::port(port)),
        None => crate::privacy::port(side),
    }
}
