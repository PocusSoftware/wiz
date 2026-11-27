//! Looking for a newer Wiz in the background, and installing it when asked.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use gpui::App;
use wiz_core::Error;
use wiz_core::update::{self, Release};

use crate::platform;

static FOUND: Mutex<Option<Release>> = Mutex::new(None);
const FIRST_CHECK: Duration = Duration::from_secs(5);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

pub fn available() -> Option<Release> {
    FOUND.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub fn start(cx: &mut App) {
    let Some(target) = platform::update_target() else {
        return;
    };
    clean_up(&target);
    cx.spawn(async move |cx| {
        let mut wait = FIRST_CHECK;
        loop {
            cx.background_executor().timer(wait).await;
            wait = EVERY;
            if !crate::prefs::get().check_updates() {
                continue;
            }
            if let Ok(Some(r)) = cx
                .background_executor()
                .spawn(async { update::newer_release() })
                .await
            {
                *FOUND.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
                cx.update(App::refresh_windows);
            }
        }
    })
    .detach();
}

pub fn install(r: &Release) -> wiz_core::Result<()> {
    let target = platform::update_target()
        .ok_or_else(|| Error::Update("this build can't update itself".into()))?;
    let cant =
        |e: std::io::Error| Error::Update(format!("can't replace {}: {e}", target.display()));
    let stage = stage_dir(&target);
    remove_any(&stage);
    std::fs::create_dir_all(&stage).map_err(cant)?;
    let file = stage.join(&r.asset.name);
    update::download(&r.asset, &file)?;
    let new = platform::unpack_update(&file, &stage).map_err(|e| Error::Update(e.to_string()))?;
    swap(&target, &new).map_err(cant)?;
    remove_any(&stage);
    platform::relaunch(&target).map_err(|e| Error::Update(e.to_string()))
}

fn stage_dir(target: &Path) -> PathBuf {
    target.with_file_name(".wiz-update")
}

fn old_path(target: &Path) -> PathBuf {
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{name}.old"))
}

fn swap(target: &Path, new: &Path) -> std::io::Result<()> {
    let old = old_path(target);
    remove_any(&old);
    std::fs::rename(target, &old)?;
    if let Err(e) = std::fs::rename(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e);
    }
    remove_any(&old);
    Ok(())
}

fn remove_any(path: &Path) {
    let _ = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
}

fn clean_up(target: &Path) {
    remove_any(&old_path(target));
    remove_any(&stage_dir(target));
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteKind {
    Heading,
    Bullet,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteLine {
    pub kind: NoteKind,
    pub text: String,
    pub bold: Vec<std::ops::Range<usize>>,
    pub code: Vec<std::ops::Range<usize>>,
}

pub fn note_lines(markdown: &str) -> Vec<NoteLine> {
    let mut out = Vec::new();
    let mut skipping = false;
    for raw in markdown.lines() {
        let line = raw.trim_end();
        let (kind, rest) = if let Some(h) = line.trim_start().strip_prefix('#') {
            (NoteKind::Heading, h.trim_start_matches('#').trim())
        } else if let Some(b) = line
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| line.trim_start().strip_prefix("* "))
        {
            (NoteKind::Bullet, b.trim())
        } else {
            (NoteKind::Text, line.trim())
        };
        if kind == NoteKind::Heading {
            skipping = rest.eq_ignore_ascii_case("downloads");
        }
        if skipping || rest.is_empty() {
            continue;
        }
        out.push(inline(kind, rest));
    }
    out
}

fn inline(kind: NoteKind, src: &str) -> NoteLine {
    let mut text = String::new();
    let (mut bold, mut code) = (Vec::new(), Vec::new());
    let (mut bold_at, mut code_at) = (None, None);
    let mut rest = src;
    while let Some(ch) = rest.chars().next() {
        if code_at.is_none() && rest.starts_with("**") {
            match bold_at.take() {
                Some(start) => bold.push(start..text.len()),
                None => bold_at = Some(text.len()),
            }
            rest = &rest[2..];
        } else if ch == '`' {
            match code_at.take() {
                Some(start) => code.push(start..text.len()),
                None => code_at = Some(text.len()),
            }
            rest = &rest[1..];
        } else if ch == '['
            && code_at.is_none()
            && let Some((label, after)) = rest[1..].split_once("](")
            && let Some((_, tail)) = after.split_once(')')
        {
            text.push_str(label);
            rest = tail;
        } else {
            text.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    NoteLine {
        kind,
        text,
        bold,
        code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_puts_the_new_file_in_place_and_leaves_nothing_behind() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("wiz");
        let new = d.path().join("new");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        swap(&target, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(!new.exists() && !old_path(&target).exists());
    }

    #[test]
    fn a_failed_swap_restores_the_old_file() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("wiz");
        std::fs::write(&target, "old").unwrap();
        assert!(swap(&target, &d.path().join("missing")).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
    }

    #[test]
    fn whole_bundles_swap_too() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("Wiz.app");
        let new = d.path().join("stage/Wiz.app");
        std::fs::create_dir_all(target.join("Contents")).unwrap();
        std::fs::create_dir_all(new.join("Contents")).unwrap();
        std::fs::write(new.join("Contents/v2"), "").unwrap();
        swap(&target, &new).unwrap();
        assert!(target.join("Contents/v2").exists());
        assert!(!old_path(&target).exists());
    }

    #[test]
    fn notes_lose_their_markdown_and_the_downloads_list() {
        let md = "## Fixed\n- **Mouse in terminal programs**: clicks reach `btop`, see [the docs](https://x).\n\n## Downloads\n- macOS: `Wiz-macOS-arm64.zip`\n\nRunning 0.26.0? Update.\n## New\nPlain text";
        let lines = note_lines(md);
        let texts: Vec<_> = lines.iter().map(|l| (l.kind, l.text.as_str())).collect();
        assert_eq!(
            texts,
            [
                (NoteKind::Heading, "Fixed"),
                (
                    NoteKind::Bullet,
                    "Mouse in terminal programs: clicks reach btop, see the docs."
                ),
                (NoteKind::Heading, "New"),
                (NoteKind::Text, "Plain text"),
            ]
        );
        let b = &lines[1];
        assert_eq!(&b.text[b.bold[0].clone()], "Mouse in terminal programs");
        assert_eq!(&b.text[b.code[0].clone()], "btop");
    }

    #[test]
    fn unclosed_markers_are_kept_as_plain_text() {
        let l = &note_lines("a **b and `c")[0];
        assert_eq!(l.text, "a b and c");
        assert!(l.bold.is_empty() && l.code.is_empty());
    }

    #[test]
    fn dev_builds_never_update_themselves() {
        if cfg!(debug_assertions) {
            assert!(platform::update_target().is_none());
        }
    }
}
