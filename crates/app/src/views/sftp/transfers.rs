//! Copying between the two panes: the queue, SFTP or rsync jobs, progress and the transfer bar.

use super::*;

/// Why a copy must be refused: onto itself, or a folder into its own subtree.
pub(super) fn self_copy_error(
    same_fs: bool,
    src_dir: &str,
    dst_dir: &str,
    names: &[String],
) -> Option<String> {
    if !same_fs {
        return None;
    }
    let (s, d) = (std::path::Path::new(src_dir), std::path::Path::new(dst_dir));
    if s == d {
        return Some("Source and destination are the same folder.".into());
    }
    names
        .iter()
        .find(|n| d.starts_with(s.join(n)))
        .map(|n| format!("Cannot copy {n} into itself."))
}

pub(super) fn group_by_folder(paths: &[PathBuf]) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for p in paths {
        let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else {
            continue;
        };
        let (dir, name) = (
            dir.to_string_lossy().into_owned(),
            name.to_string_lossy().into_owned(),
        );
        match groups.iter_mut().find(|g| g.0 == dir) {
            Some(g) => g.1.push(name),
            None => groups.push((dir, vec![name])),
        }
    }
    groups
}

impl SftpView {
    /// Copy the selection of `from` into the other pane's directory.
    pub(super) fn begin_transfer(&mut self, from: Side, cx: &mut Context<Self>) {
        let names = self.pane(from).selected_names();
        self.begin_transfer_names(from, names, cx);
    }

    pub(super) fn begin_transfer_names(
        &mut self,
        from: Side,
        names: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let p = self.pane(from);
        let (source, path) = (p.source.clone(), p.path.clone());
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for name in names {
            let (sub, base) = name.rsplit_once('/').unwrap_or(("", &name));
            let dir = if sub.is_empty() {
                path.clone()
            } else {
                source.join(&path, sub)
            };
            match groups.iter_mut().find(|(d, _)| *d == dir) {
                Some((_, v)) => v.push(base.to_string()),
                None => groups.push((dir, vec![base.to_string()])),
            }
        }
        for (dir, names) in groups {
            let origin = Origin {
                source: source.clone(),
                dir,
            };
            self.begin_copy(origin, from.other(), names, cx);
        }
    }

    pub(super) fn drop_paths(&mut self, to: Side, paths: &[PathBuf], cx: &mut Context<Self>) {
        for (dir, names) in group_by_folder(paths) {
            let origin = Origin {
                source: Source::Local,
                dir,
            };
            self.begin_copy(origin, to, names, cx);
        }
    }

    fn begin_copy(&mut self, origin: Origin, to: Side, names: Vec<String>, cx: &mut Context<Self>) {
        if names.is_empty() {
            return;
        }
        let dest_dir = self.pane(to).path.clone();
        let dest = self.pane(to).source.clone();
        if let Some(msg) =
            self_copy_error(origin.source.same_fs(&dest), &origin.dir, &dest_dir, &names)
        {
            self.pane_mut(to).error = Some(msg.into());
            cx.notify();
            return;
        }
        let check = names.clone();
        cx.spawn(async move |this, cx| {
            // Which names already exist at the destination?
            let existing: Vec<String> = match dest {
                Source::Local => check
                    .iter()
                    .filter(|n| PathBuf::from(&dest_dir).join(n).exists())
                    .cloned()
                    .collect(),
                Source::Remote(r) => {
                    let d = dest_dir.clone();
                    let c = check.clone();
                    runtime::spawn(async move {
                        let mut v = vec![];
                        for n in c {
                            if r.sftp.exists(&sftp::join(&d, &n)).await.unwrap_or(false) {
                                v.push(n);
                            }
                        }
                        v
                    })
                    .await
                }
            };
            this.update(cx, |t, cx| {
                if existing.is_empty() {
                    t.run_transfer(origin, to, names, cx);
                } else {
                    t.prompt = Some(Prompt::Conflict {
                        origin,
                        to,
                        names,
                        existing,
                    });
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn run_transfer(
        &mut self,
        origin: Origin,
        to: Side,
        names: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if names.is_empty() {
            return;
        }
        let Origin {
            source: from_src,
            dir: src_dir,
        } = origin;
        let dst_dir = self.pane(to).path.clone();
        let to_src = self.pane(to).source.clone();
        // A copied command would need the jump hosts and their keys too: none then.
        let through_jump = [&from_src, &to_src]
            .iter()
            .any(|s| matches!(s, Source::Remote(r) if r.host.jump.is_some()));
        let rsync_cmd = if through_jump {
            None
        } else {
            rsync::command(
                &from_src.endpoint(&src_dir),
                &names,
                &to_src.endpoint(&dst_dir),
            )
        };
        let id = self.next_id;
        self.next_id += 1;
        let label = if names.len() == 1 {
            names[0].clone()
        } else {
            format!("{} items", names.len())
        };
        self.transfers.push(Transfer {
            id,
            job: Some(Job {
                to,
                src_dir,
                dst_dir,
                names,
                from_src: from_src.clone(),
                to_src: to_src.clone(),
            }),
            label,
            route: format!("{} to {}", from_src.label(), to_src.label()),
            progress: Progress::new(),
            state: TState::Queued,
            finished: None,
            last_done: 0,
            speed: 0.0,
            rsync: rsync_cmd,
            rsync_agent: matches!(
                (&from_src, &to_src),
                (Source::Remote(_), Source::Remote(_))
            ),
            rsync_prompts: [&from_src, &to_src].iter().any(|s| {
                matches!(s, Source::Remote(r) if matches!(r.host.auth, ssh_core::config::AuthMethod::Password))
            }),
            via_rsync: Arc::new(AtomicBool::new(false)),
        });
        self.pump(cx);
    }

    /// Start queued transfers while there are free slots.
    pub(super) fn pump(&mut self, cx: &mut Context<Self>) {
        let max = crate::prefs::get().max_transfers() as usize;
        loop {
            let running = self
                .transfers
                .iter()
                .filter(|t| matches!(t.state, TState::Running))
                .count();
            if running >= max {
                break;
            }
            let Some(id) = self
                .transfers
                .iter()
                .find(|t| matches!(t.state, TState::Queued))
                .map(|t| t.id)
            else {
                break;
            };
            self.start_transfer(id, cx);
        }
        cx.notify();
    }

    fn start_transfer(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(tr) = self.transfers.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let Some(Job {
            to,
            src_dir,
            dst_dir,
            names,
            from_src,
            to_src,
        }) = tr.job.take()
        else {
            return;
        };
        tr.state = TState::Running;
        let progress = tr.progress.clone();
        progress.limit.store(
            crate::prefs::get().transfer_limit_kbps() as u64 * 1024,
            Ordering::Relaxed,
        );
        #[cfg(unix)]
        let via = tr.via_rsync.clone();
        cx.spawn(async move |this, cx| {
            let r = runtime::spawn(async move {
                // Count the whole selection first so the percentage and the "use rsync"
                // hint are right from the start.
                let (mut bytes, mut files) = (0u64, 0u64);
                for n in &names {
                    let sp = from_src.join(&src_dir, n);
                    let (b, f) = match &from_src {
                        Source::Remote(r) => r.sftp.scan(&sp).await?,
                        Source::Local => {
                            let p = PathBuf::from(sp);
                            tokio::task::spawn_blocking(move || local::scan(&p))
                                .await
                                .map_err(|e| ssh_core::Error::Sftp(e.to_string()))??
                        }
                    };
                    bytes += b;
                    files += f;
                }
                progress.set_prescan(bytes, files);
                // Large and between this Mac and one server: rsync, if both sides have it.
                #[cfg(unix)]
                {
                    let prefs = crate::prefs::get();
                    let big = files >= prefs.rsync_hint_files() as u64
                        || bytes >= (prefs.rsync_hint_mb() as u64) << 20;
                    let job = match (&from_src, &to_src) {
                        (Source::Local, Source::Remote(r)) => Some((r.conn.clone(), true)),
                        (Source::Remote(r), Source::Local) => Some((r.conn.clone(), false)),
                        _ => None,
                    };
                    if prefs.use_rsync()
                        && big
                        && let Some((conn, upload)) = job
                        && let Some(local) = rsync::runner::local().await
                        && conn.has_command("rsync").await
                    {
                        via.store(true, Ordering::Relaxed);
                        let (src, dst) = (&src_dir, &dst_dir);
                        return rsync::runner::run(conn, local, upload, src, &names, dst, progress)
                            .await;
                    }
                }
                for n in names {
                    let (sp, dp) = (from_src.join(&src_dir, &n), to_src.join(&dst_dir, &n));
                    match (&from_src, &to_src) {
                        (Source::Local, Source::Remote(r)) => {
                            r.sftp.upload(&PathBuf::from(&sp), &dp, &progress).await?
                        }
                        (Source::Remote(r), Source::Local) => {
                            r.sftp.download(&sp, &PathBuf::from(&dp), &progress).await?
                        }
                        (Source::Remote(a), Source::Remote(b)) => {
                            a.sftp.copy_to(&sp, &b.sftp, &dp, &progress).await?
                        }
                        (Source::Local, Source::Local) => {
                            let p = progress.clone();
                            tokio::task::spawn_blocking(move || {
                                local::copy(&PathBuf::from(sp), &PathBuf::from(dp), &p)
                            })
                            .await
                            .map_err(|e| ssh_core::Error::Sftp(e.to_string()))??;
                        }
                    }
                }
                Ok::<_, ssh_core::Error>(())
            })
            .await;
            this.update(cx, |t, cx| {
                if let Some(tr) = t.transfers.iter_mut().find(|x| x.id == id) {
                    tr.finished = Some(Instant::now());
                    tr.state = match r {
                        Ok(()) => TState::Done,
                        Err(ssh_core::Error::Cancelled) => TState::Cancelled,
                        Err(e) => TState::Failed(e.to_string()),
                    };
                }
                t.refresh(to, cx);
                t.pump(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn transfer_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.transfers.is_empty() {
            return None;
        }
        let rows = self.transfers.iter().map(|tr| {
            let frac = tr.progress.fraction();
            let running = matches!(tr.state, TState::Running);
            let queued = matches!(tr.state, TState::Queued);
            let paused = running && tr.progress.is_paused();
            let id = tr.id;
            let files = tr.progress.files.load(Ordering::Relaxed);
            let total = tr.progress.total.load(Ordering::Relaxed);
            // Past this size rsync (delta, compression, resume) beats per-file SFTP.
            let prefs = crate::prefs::get();
            let big = files >= prefs.rsync_hint_files() as u64
                || total >= (prefs.rsync_hint_mb() as u64) << 20;
            let status: (String, u32) = match &tr.state {
                TState::Queued => ("Queued".into(), palette().muted),
                TState::Running if paused => (
                    format!(
                        "Paused  {:.0}%  {}/{}",
                        frac * 100.0,
                        human_size(tr.progress.done.load(Ordering::Relaxed)),
                        human_size(tr.progress.total.load(Ordering::Relaxed)),
                    ),
                    palette().yellow,
                ),
                TState::Running => (
                    format!(
                        "{:.0}%  {}/{}  {}/s",
                        frac * 100.0,
                        human_size(tr.progress.done.load(Ordering::Relaxed)),
                        human_size(tr.progress.total.load(Ordering::Relaxed)),
                        human_size(tr.speed as u64)
                    ),
                    palette().muted,
                ),
                TState::Done => ("Done".into(), palette().green),
                TState::Cancelled => ("Cancelled".into(), palette().yellow),
                TState::Failed(e) => (crate::privacy::redact(e).into_owned(), palette().red),
            };
            let bar_color = match tr.state {
                TState::Failed(_) => palette().red,
                TState::Done => palette().green,
                TState::Running if paused => palette().yellow,
                _ => accent(),
            };
            let row = div()
                .h(px(24.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(icon("arrow-right", 13., accent_text()))
                .child(
                    div()
                        .w(px(150.))
                        .flex_none()
                        .overflow_hidden()
                        .child(div().whitespace_nowrap().child(tr.label.clone())),
                )
                .child(
                    div()
                        .w(px(130.))
                        .flex_none()
                        .overflow_hidden()
                        .text_color(c(palette().placeholder))
                        .child(div().whitespace_nowrap().child(
                            if tr.via_rsync.load(Ordering::Relaxed) {
                                format!("{} · rsync", tr.route)
                            } else {
                                tr.route.clone()
                            },
                        )),
                )
                .child(
                    div().w(px(140.)).h(px(6.)).flex_none().bg(c(palette().active)).child(
                        div()
                            .h_full()
                            .w(relative(if matches!(tr.state, TState::Done) {
                                1.0
                            } else {
                                frac
                            }))
                            .bg(c(bar_color)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_color(c(status.1))
                        .child(div().whitespace_nowrap().child(status.0)),
                )
                .when(running, |d| {
                    d.child(
                        icon_button(
                            ("t-pause", id as usize),
                            if paused { "play" } else { "pause" },
                            if paused { "Resume" } else { "Pause" },
                            palette().muted,
                        )
                        .on_click(cx.listener(move |t, _, _, cx| {
                            if let Some(tr) = t.transfers.iter().find(|x| x.id == id) {
                                tr.progress.set_paused(!tr.progress.is_paused());
                            }
                            cx.notify();
                        })),
                    )
                })
                .child(
                    icon_button(
                        ("t-x", id as usize),
                        "x",
                        if running { "Cancel" } else if queued { "Remove from queue" } else { "Dismiss" },
                        palette().muted,
                    )
                    .on_click(cx.listener(move |t, _, _, cx| {
                        if let Some(tr) = t.transfers.iter().find(|x| x.id == id) {
                            if matches!(tr.state, TState::Running) {
                                tr.progress.cancel();
                            } else {
                                t.transfers.retain(|x| x.id != id);
                            }
                        }
                        cx.notify();
                    })),
                );
            let hint = tr
                .rsync
                .clone()
                .filter(|_| {
                    crate::platform::RSYNC
                        && big
                        && running
                        && !tr.via_rsync.load(Ordering::Relaxed)
                })
                .map(|cmd| {
                div()
                    .px_2()
                    .pb(px(5.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(c(palette().yellow))
                    .child(format!(
                        "Large transfer ({} files, {}): rsync is faster (needs rsync on the server){}{}.",
                        files,
                        human_size(total),
                        if tr.rsync_prompts {
                            ". It will ask for the password"
                        } else {
                            ""
                        },
                        if tr.rsync_agent {
                            ". It lets the source server use your ssh agent while it runs"
                        } else {
                            ""
                        }
                    ))
                    .child(
                        button(("t-rsync", id as usize), "Copy rsync command").on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(cmd.clone()))
                            }),
                        ),
                    )
            });
            fade_in(
                div().flex().flex_col().child(row).children(hint),
                ("xfer-row", id as usize),
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
