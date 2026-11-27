//! Looking for a newer Wiz in the background, and installing it when asked.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use gpui::App;
use ssh_core::Error;
use ssh_core::update::{self, Release};

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

pub fn install(r: &Release) -> ssh_core::Result<()> {
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
    fn dev_builds_never_update_themselves() {
        if cfg!(debug_assertions) {
            assert!(platform::update_target().is_none());
        }
    }
}
