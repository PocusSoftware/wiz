//! New folder, rename and delete: the prompt bar that asks, and running the operation.

use super::*;

enum FsOp {
    Mkdir(String),
    Rename(String, String),
    Delete(Vec<String>),
}

impl SftpView {
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
            window.focus(&self.prompt_input.focus_handle(cx), cx);
        }
        cx.notify();
    }

    fn close_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = None;
        window.focus(&self.pane(self.active).focus, cx);
        cx.notify();
    }

    pub(super) fn ask_new_folder(
        &mut self,
        side: Side,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_prompt(Prompt::NewFolder(side), "", true, window, cx);
    }

    pub(super) fn ask_rename(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let names = self.pane(side).selected_names();
        if let [name] = names.as_slice() {
            let name = name.clone();
            self.open_prompt(Prompt::Rename(side, name.clone()), &name, true, window, cx);
        }
    }

    pub(super) fn ask_delete(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
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
        window.focus(&self.pane(self.active).focus, cx);
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

    pub(super) fn prompt_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
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
            .border_color(c(palette().border))
            .bg(c(palette().elevated))
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
                .child(
                    div()
                        .text_color(c(palette().red))
                        .child(if names.len() == 1 {
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
                        }),
                )
                .child(div().flex_1())
                .child(
                    button("p-ok", "Delete")
                        .bg(c(palette().red))
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
}
