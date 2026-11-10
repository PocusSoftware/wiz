//! Editing a server file in a local app: download, watch for saves, upload again.

use super::*;

/// Largest file opened for editing; bigger ones are better downloaded.
const EDIT_MAX: u64 = 64 << 20;

impl SftpView {
    /// Open `path` in the configured editor; a failure (no such program) shows on `side`.
    fn launch_editor(&mut self, side: Side, path: &std::path::Path, cx: &mut Context<Self>) {
        let app = crate::prefs::get().editor_app();
        if let Err(e) = crate::platform::open_in_editor(path, &app) {
            self.pane_mut(side).error = Some(format!("Could not open the editor: {e}").into());
            cx.notify();
        }
    }

    /// Open a file in the local editor. Server files are downloaded to a temp folder first
    /// and uploaded again when saved.
    pub(super) fn open_file(
        &mut self,
        side: Side,
        name: String,
        path: String,
        size: u64,
        cx: &mut Context<Self>,
    ) {
        let remote = match &self.pane(side).source {
            Source::Local => {
                self.launch_editor(side, std::path::Path::new(&path), cx);
                return;
            }
            Source::Remote(r) => (**r).clone(),
        };
        if size > EDIT_MAX {
            self.pane_mut(side).error =
                Some(format!("{name} is over {} MB, download it instead.", EDIT_MAX >> 20).into());
            cx.notify();
            return;
        }
        // Already open: just bring the editor forward.
        if let Some(local) = self
            .edits
            .iter()
            .find(|e| e.remote_path == path && e.remote.host.account() == remote.host.account())
            .map(|e| e.local.clone())
        {
            self.launch_editor(side, &local, cx);
            return;
        }
        // The name comes from the server: never let it point outside the temp folder.
        if !sftp::is_safe_name(&name) {
            self.pane_mut(side).error = Some("That file name can't be opened safely.".into());
            cx.notify();
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        let dir = std::env::temp_dir()
            .join("wiz-edit")
            .join(format!("{}-{id}", std::process::id()));
        let local = dir.join(&name);
        let task_dir = dir.clone();
        self.edits.push(Edit {
            id,
            name,
            remote: remote.clone(),
            remote_path: path.clone(),
            local: local.clone(),
            dir,
            downloaded: false,
            mtime: None,
            state: EState::Downloading,
            synced_at: None,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = runtime::spawn(async move {
                tokio::fs::create_dir_all(&task_dir).await?;
                let r = remote.sftp.download(&path, &local, &Progress::new()).await;
                if r.is_err() {
                    // Never leave a partial file around that could be uploaded.
                    let _ = tokio::fs::remove_dir_all(&task_dir).await;
                }
                r
            })
            .await;
            this.update(cx, |t, cx| {
                let Some(e) = t.edits.iter_mut().find(|e| e.id == id) else {
                    return;
                };
                match r {
                    Ok(()) => {
                        e.mtime = std::fs::metadata(&e.local).and_then(|m| m.modified()).ok();
                        e.downloaded = true;
                        e.state = EState::Synced;
                        e.synced_at = Some(chrono::Local::now());
                        let local = e.local.clone();
                        t.launch_editor(side, &local, cx);
                    }
                    Err(err) => e.state = EState::Failed(err.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Spot saves of watched files (called from the half-second ticker).
    pub(super) fn poll_edits(&mut self, cx: &mut Context<Self>) {
        let auto = crate::prefs::get().auto_upload_edits();
        let mut upload = vec![];
        for e in self.edits.iter_mut() {
            if !e.downloaded || matches!(e.state, EState::Downloading | EState::Uploading) {
                continue;
            }
            let now = std::fs::metadata(&e.local).and_then(|m| m.modified()).ok();
            if now.is_some() && now != e.mtime {
                if auto {
                    upload.push(e.id);
                } else if !matches!(e.state, EState::Modified) {
                    e.state = EState::Modified;
                    cx.notify();
                }
            }
        }
        for id in upload {
            self.upload_edit(id, cx);
        }
    }

    fn upload_edit(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(e) = self.edits.iter_mut().find(|e| e.id == id && e.downloaded) else {
            return;
        };
        let mtime = std::fs::metadata(&e.local).and_then(|m| m.modified()).ok();
        e.state = EState::Uploading;
        let (remote, path, local) = (e.remote.clone(), e.remote_path.clone(), e.local.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r =
                runtime::spawn(
                    async move { remote.sftp.upload(&local, &path, &Progress::new()).await },
                )
                .await;
            this.update(cx, |t, cx| {
                if let Some(e) = t.edits.iter_mut().find(|e| e.id == id) {
                    match r {
                        Ok(()) => {
                            e.mtime = mtime;
                            e.state = EState::Synced;
                            e.synced_at = Some(chrono::Local::now());
                        }
                        Err(err) => {
                            // Remember the mtime so a failure isn't retried every tick.
                            e.mtime = mtime;
                            e.state = EState::Failed(err.to_string());
                        }
                    }
                }
                // The server file changed: refresh a pane that shows its folder.
                for side in [Side::Left, Side::Right] {
                    t.refresh(side, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn stop_edit(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(ix) = self.edits.iter().position(|e| e.id == id) {
            remove_edit_dir(&self.edits.remove(ix).dir);
        }
        cx.notify();
    }

    pub(super) fn edit_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.edits.is_empty() {
            return None;
        }
        let rows = self.edits.iter().map(|e| {
            let id = e.id;
            let (status, color): (String, u32) = match &e.state {
                EState::Downloading => ("Downloading...".into(), palette().muted),
                EState::Synced => (
                    match e.synced_at {
                        Some(t) => format!("In sync, {}", t.format("%H:%M:%S")),
                        None => "In sync".into(),
                    },
                    palette().green,
                ),
                EState::Modified => ("Saved locally, not uploaded".into(), palette().yellow),
                EState::Uploading => ("Uploading...".into(), palette().muted),
                EState::Failed(m) => (crate::privacy::redact(m).into_owned(), palette().red),
            };
            let can_upload =
                e.downloaded && matches!(e.state, EState::Modified | EState::Failed(_));
            div()
                .h(px(24.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(icon("external-link", 13., accent_text()))
                .child(
                    div()
                        .w(px(150.))
                        .flex_none()
                        .overflow_hidden()
                        .child(div().whitespace_nowrap().child(e.name.clone())),
                )
                .child(
                    div()
                        .w(px(130.))
                        .flex_none()
                        .overflow_hidden()
                        .text_color(c(palette().placeholder))
                        .child(div().whitespace_nowrap().child(e.remote.label.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_color(c(color))
                        .child(div().whitespace_nowrap().child(status)),
                )
                .when(can_upload, |d| {
                    d.child(
                        icon_button(("e-up", id as usize), "upload", "Upload now", accent_text())
                            .on_click(cx.listener(move |t, _, _, cx| t.upload_edit(id, cx))),
                    )
                })
                .child(
                    icon_button(("e-x", id as usize), "x", "Stop watching", palette().muted)
                        .on_click(cx.listener(move |t, _, _, cx| t.stop_edit(id, cx))),
                )
        });
        Some(
            div()
                .border_t_1()
                .border_color(c(palette().border))
                .bg(c(palette().panel))
                .children(rows)
                .into_any_element(),
        )
    }
}
