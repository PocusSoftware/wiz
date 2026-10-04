//! Container list with start/stop/restart/remove, logs, and (remote only) a shell.

use std::collections::HashSet;
use std::time::Duration;

use gpui::{Context, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*, px};
use ssh_core::docker::{Action, Backend, Container};

use crate::runtime;
use crate::theme::*;
use crate::ui::button;

pub enum DockerEvent {
    Shell { id: String, name: String },
    Logs { name: String, text: String },
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
                cx.background_executor().timer(Duration::from_secs(5)).await;
                this.update(cx, |t, cx| t.refresh(cx)).ok()?;
            }
            #[allow(unreachable_code)]
            None::<()>
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
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let r = runtime::spawn(async move { backend.logs(&id, 500).await }).await;
            this.update(cx, |t, cx| match r {
                Ok(text) => cx.emit(DockerEvent::Logs { name, text }),
                Err(e) => {
                    t.error = Some(e.to_string().into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
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

            let mut actions = div().flex().gap_1().items_center();
            if busy {
                actions = actions.child(div().text_xs().text_color(c(DIM)).child("working..."));
            } else {
                if running {
                    let i = id.clone();
                    actions = actions
                        .child(button(("stop", ix), "Stop").on_click(
                            cx.listener(move |t, _, _, cx| t.act(i.clone(), Action::Stop, cx)),
                        ))
                        .child(button(("restart", ix), "Restart").on_click({
                            let i = id.clone();
                            cx.listener(move |t, _, _, cx| t.act(i.clone(), Action::Restart, cx))
                        }));
                    if self.can_shell {
                        let (i, n) = (id.clone(), name.clone());
                        actions = actions.child(button(("shell", ix), "Shell").on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.emit(DockerEvent::Shell {
                                    id: i.clone(),
                                    name: n.clone(),
                                })
                            }),
                        ));
                    }
                } else {
                    let i = id.clone();
                    actions = actions.child(button(("start", ix), "Start").on_click(
                        cx.listener(move |t, _, _, cx| t.act(i.clone(), Action::Start, cx)),
                    ));
                }
                let (i, n) = (id.clone(), name.clone());
                actions = actions
                    .child(button(("logs", ix), "Logs").on_click(
                        cx.listener(move |t, _, _, cx| t.logs(i.clone(), n.clone(), cx)),
                    ));
                let i = id.clone();
                let rm = button(
                    ("rm", ix),
                    if confirming {
                        "Really remove?"
                    } else {
                        "Remove"
                    },
                )
                .text_color(c(RED))
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
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(c(SURFACE))
                .child(
                    div()
                        .size(px(8.))
                        .rounded_full()
                        .bg(c(if running { GREEN } else { DIM })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().child(ct.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(c(DIM))
                                .overflow_hidden()
                                .child(format!("{}  ·  {}", ct.image, ct.status)),
                        )
                        .when(!ct.ports.is_empty(), |d| {
                            d.child(div().text_xs().text_color(c(DIM)).child(ct.ports.clone()))
                        }),
                )
                .child(actions)
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(c(BORDER))
                    .child(
                        div()
                            .text_sm()
                            .child(format!("{} containers", self.items.len())),
                    )
                    .child(
                        button("refresh", "Refresh")
                            .on_click(cx.listener(|t, _, _, cx| t.refresh(cx))),
                    ),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(div().px_3().py_2().text_xs().text_color(c(RED)).child(e))
            })
            .when(
                self.loaded && self.items.is_empty() && self.error.is_none(),
                |d| {
                    d.child(
                        div()
                            .p_4()
                            .text_sm()
                            .text_color(c(DIM))
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
