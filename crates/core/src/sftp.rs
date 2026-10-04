//! SFTP: remote and local directory listing plus recursive transfers with progress.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;

use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Error, Result};

fn e(err: impl std::fmt::Display) -> Error {
    Error::Sftp(err.to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

fn sort_entries(v: &mut [Entry]) {
    v.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Join POSIX paths without doubling slashes.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Parent of a POSIX path ("/" stays "/").
pub fn parent(path: &str) -> String {
    let t = path.trim_end_matches('/');
    match t.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => t[..i].into(),
    }
}

/// Shared between the UI and a running transfer.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    /// Files seen by the pre-scan, for the "use rsync" heuristic.
    pub files: AtomicU64,
    /// Totals were already counted for the whole job, so per-item scans must not add again.
    prescanned: AtomicBool,
    pub cancel: AtomicBool,
    pub current: std::sync::Mutex<String>,
    /// Speed limit in bytes per second, 0 for none. Can change while running.
    pub limit: AtomicU64,
    /// Start of the current one-second pacing window and bytes sent in it.
    window: std::sync::Mutex<Option<(std::time::Instant, u64)>>,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// Count `bytes` and `files` toward the job total (ignored once prescanned).
    fn add_totals(&self, bytes: u64, files: u64) {
        if !self.prescanned.load(Ordering::Relaxed) {
            self.total.fetch_add(bytes, Ordering::Relaxed);
            self.files.fetch_add(files, Ordering::Relaxed);
        }
    }

    /// Add to the totals up front and stop per-item scans from adding again.
    pub fn set_prescan(&self, bytes: u64, files: u64) {
        self.total.fetch_add(bytes, Ordering::Relaxed);
        self.files.fetch_add(files, Ordering::Relaxed);
        self.prescanned.store(true, Ordering::Relaxed);
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
    fn set_current(&self, s: &str) {
        *self.current.lock().unwrap() = s.to_string();
    }
    /// Count `n` bytes against the speed limit and return how long to wait before sending
    /// more. Paced over one-second windows, so a changed limit applies within a second.
    pub fn pace(&self, n: u64) -> std::time::Duration {
        use std::time::{Duration, Instant};
        let limit = self.limit.load(Ordering::Relaxed);
        if limit == 0 {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let mut w = self.window.lock().unwrap();
        let (start, sent) = match *w {
            Some((start, sent)) if now - start < Duration::from_secs(1) => (start, sent + n),
            _ => (now, n),
        };
        *w = Some((start, sent));
        Duration::from_secs_f64(sent as f64 / limit as f64).saturating_sub(now - start)
    }

    pub fn fraction(&self) -> f32 {
        let t = self.total.load(Ordering::Relaxed);
        if t == 0 {
            0.0
        } else {
            self.done.load(Ordering::Relaxed) as f32 / t as f32
        }
    }
}

const CHUNK: usize = 128 * 1024;

pub struct Sftp {
    inner: SftpSession,
}

impl Sftp {
    pub(crate) async fn new<S>(stream: S) -> Result<Self>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        Ok(Self {
            inner: SftpSession::new(stream).await.map_err(e)?,
        })
    }

    /// The server's idea of the home directory.
    pub async fn home(&self) -> Result<String> {
        self.inner.canonicalize(".").await.map_err(e)
    }

    /// Absolute form of `path` (resolves `~`-less relative paths, `..`, symlinks).
    pub async fn canonical(&self, path: &str) -> Result<String> {
        self.inner.canonicalize(path).await.map_err(e)
    }

    pub async fn is_dir(&self, path: &str) -> Result<bool> {
        Ok(self.inner.metadata(path).await.map_err(e)?.is_dir())
    }

    pub async fn exists(&self, path: &str) -> Result<bool> {
        self.inner.try_exists(path).await.map_err(e)
    }

    pub async fn list(&self, path: &str) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        for ent in self.inner.read_dir(path).await.map_err(e)? {
            let name = ent.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let md = ent.metadata();
            let is_symlink = md.is_symlink();
            // Follow symlinks so a link to a directory opens like a directory.
            let (is_dir, size) = if is_symlink {
                match self.inner.metadata(join(path, &name)).await {
                    Ok(t) => (t.is_dir(), t.len()),
                    Err(_) => (false, md.len()),
                }
            } else {
                (md.is_dir(), md.len())
            };
            out.push(Entry {
                name,
                is_dir,
                is_symlink,
                size,
                modified: md.modified().ok(),
            });
        }
        sort_entries(&mut out);
        Ok(out)
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        self.inner.create_dir(path).await.map_err(e)
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.inner.rename(from, to).await.map_err(e)
    }

    /// Delete a file, or a directory with everything in it.
    pub async fn remove(&self, path: &str) -> Result<()> {
        let md = self.inner.symlink_metadata(path).await.map_err(e)?;
        if md.is_dir() && !md.is_symlink() {
            let mut stack = vec![(path.to_string(), false)];
            // Depth-first: children are removed before their directory.
            while let Some((dir, visited)) = stack.pop() {
                if visited {
                    self.inner.remove_dir(&dir).await.map_err(e)?;
                    continue;
                }
                stack.push((dir.clone(), true));
                for ent in self.list(&dir).await? {
                    let p = join(&dir, &ent.name);
                    if ent.is_dir && !ent.is_symlink {
                        stack.push((p, false));
                    } else {
                        self.inner.remove_file(&p).await.map_err(e)?;
                    }
                }
            }
            Ok(())
        } else {
            self.inner.remove_file(path).await.map_err(e)
        }
    }

    /// (bytes, files) under a remote path.
    pub async fn scan(&self, path: &str) -> Result<(u64, u64)> {
        self.remote_size(path).await
    }

    async fn remote_size(&self, path: &str) -> Result<(u64, u64)> {
        let md = self.inner.metadata(path).await.map_err(e)?;
        if !md.is_dir() {
            return Ok((md.len(), 1));
        }
        let mut total = 0;
        let mut files = 0;
        let mut stack = vec![path.to_string()];
        while let Some(dir) = stack.pop() {
            for ent in self.list(&dir).await? {
                if ent.is_dir {
                    if !ent.is_symlink {
                        stack.push(join(&dir, &ent.name));
                    }
                } else {
                    total += ent.size;
                    files += 1;
                }
            }
        }
        Ok((total, files))
    }

    /// Download `remote` (file or directory) to the exact local path `dest`.
    pub async fn download(&self, remote: &str, dest: &Path, progress: &Progress) -> Result<()> {
        let (total, files) = self.remote_size(remote).await?;
        progress.add_totals(total, files);
        self.download_inner(remote, dest, progress).await
    }

    fn download_inner<'a>(
        &'a self,
        remote: &'a str,
        dest: &'a Path,
        progress: &'a Progress,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            progress.check()?;
            let md = self.inner.metadata(remote).await.map_err(e)?;
            if md.is_dir() {
                tokio::fs::create_dir_all(dest).await?;
                for ent in self.list(remote).await? {
                    if ent.is_dir && ent.is_symlink {
                        continue; // never follow directory links: they can loop
                    }
                    self.download_inner(&join(remote, &ent.name), &dest.join(&ent.name), progress)
                        .await?;
                }
                return Ok(());
            }
            progress.set_current(remote.rsplit('/').next().unwrap_or(remote));
            let mut src = self.inner.open(remote).await.map_err(e)?;
            let mut out = tokio::fs::File::create(dest).await?;
            let mut buf = vec![0u8; CHUNK];
            loop {
                if let Err(err) = progress.check() {
                    drop(out);
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(err);
                }
                let n = src.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).await?;
                progress.done.fetch_add(n as u64, Ordering::Relaxed);
                tokio::time::sleep(progress.pace(n as u64)).await;
            }
            out.flush().await?;
            Ok(())
        })
    }

    /// Copy `src` on this server to `dest` on another one, streaming chunk by chunk
    /// through this process (no temporary file). Works for files and directories.
    pub async fn copy_to(
        &self,
        src: &str,
        other: &Sftp,
        dest: &str,
        progress: &Progress,
    ) -> Result<()> {
        let (total, files) = self.remote_size(src).await?;
        progress.add_totals(total, files);
        self.copy_inner(src, other, dest, progress).await
    }

    fn copy_inner<'a>(
        &'a self,
        src: &'a str,
        other: &'a Sftp,
        dest: &'a str,
        progress: &'a Progress,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            progress.check()?;
            let md = self.inner.metadata(src).await.map_err(e)?;
            if md.is_dir() {
                if !other.exists(dest).await? {
                    other.mkdir(dest).await?;
                }
                for ent in self.list(src).await? {
                    if ent.is_dir && ent.is_symlink {
                        continue;
                    }
                    self.copy_inner(
                        &join(src, &ent.name),
                        other,
                        &join(dest, &ent.name),
                        progress,
                    )
                    .await?;
                }
                return Ok(());
            }
            progress.set_current(src.rsplit('/').next().unwrap_or(src));
            let mut input = self.inner.open(src).await.map_err(e)?;
            let mut out = other.inner.create(dest).await.map_err(e)?;
            let mut buf = vec![0u8; CHUNK];
            loop {
                if let Err(err) = progress.check() {
                    drop(out);
                    let _ = other.inner.remove_file(dest).await;
                    return Err(err);
                }
                let n = input.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).await?;
                progress.done.fetch_add(n as u64, Ordering::Relaxed);
                tokio::time::sleep(progress.pace(n as u64)).await;
            }
            out.shutdown().await?;
            Ok(())
        })
    }

    /// Upload `local` (file or directory) to the exact remote path `dest`.
    pub async fn upload(&self, local: &Path, dest: &str, progress: &Progress) -> Result<()> {
        let (total, files) = local_size(local)?;
        progress.add_totals(total, files);
        self.upload_inner(local, dest, progress).await
    }

    fn upload_inner<'a>(
        &'a self,
        local: &'a Path,
        dest: &'a str,
        progress: &'a Progress,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            progress.check()?;
            let md = tokio::fs::metadata(local).await?;
            if md.is_dir() {
                if !self.exists(dest).await? {
                    self.mkdir(dest).await?;
                }
                let mut rd = tokio::fs::read_dir(local).await?;
                while let Some(ent) = rd.next_entry().await? {
                    if is_dir_link(&ent.path()) {
                        continue;
                    }
                    let name = ent.file_name().to_string_lossy().into_owned();
                    self.upload_inner(&ent.path(), &join(dest, &name), progress)
                        .await?;
                }
                return Ok(());
            }
            progress.set_current(&local.file_name().unwrap_or_default().to_string_lossy());
            let mut src = tokio::fs::File::open(local).await?;
            let mut out = self.inner.create(dest).await.map_err(e)?;
            let mut buf = vec![0u8; CHUNK];
            loop {
                if let Err(err) = progress.check() {
                    drop(out);
                    let _ = self.inner.remove_file(dest).await;
                    return Err(err);
                }
                let n = src.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).await?;
                progress.done.fetch_add(n as u64, Ordering::Relaxed);
                tokio::time::sleep(progress.pace(n as u64)).await;
            }
            out.shutdown().await?;
            Ok(())
        })
    }
}

// ---------- local filesystem (blocking, call from spawn_blocking) ----------

pub mod local {
    use super::*;

    pub fn home() -> PathBuf {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    }

    pub fn list(path: &Path) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        for ent in std::fs::read_dir(path)? {
            let ent = ent?;
            let name = ent.file_name().to_string_lossy().into_owned();
            let lmd = ent.metadata()?;
            let is_symlink = lmd.is_symlink();
            let md = if is_symlink {
                std::fs::metadata(ent.path()).unwrap_or(lmd)
            } else {
                lmd
            };
            out.push(Entry {
                name,
                is_dir: md.is_dir(),
                is_symlink,
                size: md.len(),
                modified: md.modified().ok(),
            });
        }
        sort_entries(&mut out);
        Ok(out)
    }

    pub fn mkdir(path: &Path) -> Result<()> {
        Ok(std::fs::create_dir(path)?)
    }

    pub fn rename(from: &Path, to: &Path) -> Result<()> {
        Ok(std::fs::rename(from, to)?)
    }

    /// (bytes, files) under a local path, skipping directory links.
    pub fn scan(path: &Path) -> Result<(u64, u64)> {
        local_size(path)
    }

    /// Recursive copy with progress and cancel (blocking).
    pub fn copy(src: &Path, dest: &Path, progress: &Progress) -> Result<()> {
        let (total, files) = local_size(src)?;
        progress.add_totals(total, files);
        copy_inner(src, dest, progress)
    }

    fn copy_inner(src: &Path, dest: &Path, progress: &Progress) -> Result<()> {
        use std::io::{Read, Write};
        progress.check()?;
        if std::fs::metadata(src)?.is_dir() {
            std::fs::create_dir_all(dest)?;
            for ent in std::fs::read_dir(src)? {
                let ent = ent?;
                if is_dir_link(&ent.path()) {
                    continue;
                }
                copy_inner(&ent.path(), &dest.join(ent.file_name()), progress)?;
            }
            return Ok(());
        }
        progress.set_current(&src.file_name().unwrap_or_default().to_string_lossy());
        let mut input = std::fs::File::open(src)?;
        let mut out = std::fs::File::create(dest)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if let Err(err) = progress.check() {
                drop(out);
                let _ = std::fs::remove_file(dest);
                return Err(err);
            }
            let n = input.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            out.write_all(&buf[..n])?;
            progress.done.fetch_add(n as u64, Ordering::Relaxed);
            std::thread::sleep(progress.pace(n as u64));
        }
    }

    pub fn remove(path: &Path) -> Result<()> {
        let md = std::fs::symlink_metadata(path)?;
        if md.is_dir() {
            Ok(std::fs::remove_dir_all(path)?)
        } else {
            Ok(std::fs::remove_file(path)?)
        }
    }
}

/// A symlink that points at a directory. Walkers skip these so link loops terminate.
fn is_dir_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink())
        && std::fs::metadata(path).is_ok_and(|m| m.is_dir())
}

fn local_size(path: &Path) -> Result<(u64, u64)> {
    let md = std::fs::metadata(path)?;
    if !md.is_dir() {
        return Ok((md.len(), 1));
    }
    let (mut total, mut files) = (0, 0);
    for ent in std::fs::read_dir(path)? {
        let ent = ent?;
        if is_dir_link(&ent.path()) {
            continue;
        }
        let (b, f) = local_size(&ent.path())?;
        total += b;
        files += f;
    }
    Ok((total, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pace_waits_to_hold_the_limit() {
        let p = Progress::new();
        assert_eq!(p.pace(1_000_000), std::time::Duration::ZERO);
        p.limit.store(100_000, Ordering::Relaxed);
        // 50 KB at 100 KB/s is due after about half a second.
        let d = p.pace(50_000);
        assert!(d > std::time::Duration::from_millis(400), "{d:?}");
        assert!(d <= std::time::Duration::from_millis(500), "{d:?}");
    }

    #[test]
    fn path_helpers() {
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("/", "b"), "/b");
        assert_eq!(parent("/a/b"), "/a");
        assert_eq!(parent("/a"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(parent("/a/b/"), "/a");
    }

    #[test]
    fn dirs_sort_first_case_insensitive() {
        let f = |n: &str, d: bool| Entry {
            name: n.into(),
            is_dir: d,
            is_symlink: false,
            size: 0,
            modified: None,
        };
        let mut v = vec![
            f("b.txt", false),
            f("Z", true),
            f("a", true),
            f("A.txt", false),
        ];
        sort_entries(&mut v);
        let names: Vec<_> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["a", "Z", "A.txt", "b.txt"]);
    }
}
