//! rsync for large transfers. Two ways:
//!
//! - Built in (`run`): the local rsync is started with `-e "<bastion> --rsync-rsh"`, so its
//!   remote shell is this app again in bridge mode (`bridge_main`). The bridge hands rsync's
//!   remote command and byte stream over a Unix socket to the running app, which runs it on
//!   the already open SSH connection. No second login, so password hosts work too.
//! - `command` builds a command line to copy, for when the built-in path can't be used.

#[derive(Clone, Debug)]
pub enum Where {
    Local,
    Remote {
        user: String,
        host: String,
        port: u16,
        key: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub at: Where,
    pub dir: String,
}

/// POSIX single-quote escaping.
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn spec(e: &Endpoint, name: Option<&str>) -> String {
    let path = match name {
        Some(n) => join(&e.dir, n),
        None => format!("{}/", e.dir.trim_end_matches('/')),
    };
    match &e.at {
        Where::Local => path,
        Where::Remote { user, host, .. } => format!("{user}@{host}:{path}"),
    }
}

/// `-e` argument carrying the port and, for key hosts, the identity file.
fn ssh_e(port: u16, key: Option<&str>) -> String {
    if port == 22 && key.is_none() {
        return String::new();
    }
    let mut inner = String::from("ssh");
    if port != 22 {
        inner.push_str(&format!(" -p {port}"));
    }
    if let Some(k) = key {
        inner.push_str(&format!(" -i {}", q(k)));
    }
    format!(" -e {}", q(&inner))
}

/// Up to this many names are passed as arguments; more go through `--files-from`.
const INLINE_MAX: usize = 8;

/// Command to copy `names` from `src` into `dst`'s directory.
pub fn command(src: &Endpoint, names: &[String], dst: &Endpoint) -> String {
    if names.len() > INLINE_MAX {
        return files_from(src, names, dst);
    }
    inline(src, names, dst)
}

/// Exact for any selection size: names are read from stdin, relative to the source folder.
/// `-r` is explicit because `--files-from` stops `-a` from implying it.
fn files_from(src: &Endpoint, names: &[String], dst: &Endpoint) -> String {
    let list = names.join("\n");
    let body = |cmd: String| format!("{cmd} <<'RSYNC_FILES_EOF'\n{list}\nRSYNC_FILES_EOF");
    match (&src.at, &dst.at) {
        (Where::Local, Where::Local) => body(format!(
            "rsync -avrP --files-from=- {} {}",
            q(&spec(src, None)),
            q(&spec(dst, None))
        )),
        (Where::Local, Where::Remote { port, key, .. })
        | (Where::Remote { port, key, .. }, Where::Local) => body(format!(
            "rsync -avrzP{} --files-from=- {} {}",
            ssh_e(*port, key.as_deref()),
            q(&spec(src, None)),
            q(&spec(dst, None))
        )),
        (
            Where::Remote {
                user, host, port, ..
            },
            Where::Remote { port: dp, .. },
        ) => {
            let dest = spec(dst, None);
            let inner = format!(
                "rsync -avrzP{} --files-from=- {} {}",
                ssh_e(*dp, None),
                q(&format!("{}/", src.dir.trim_end_matches('/'))),
                q(&dest)
            );
            let p = if *port == 22 {
                String::new()
            } else {
                format!(" -p {port}")
            };
            body(format!("ssh -A{p} {user}@{host} {}", q(&inner)))
        }
    }
}

fn inline(src: &Endpoint, names: &[String], dst: &Endpoint) -> String {
    let srcs: Vec<String> = names.iter().map(|n| spec(src, Some(n))).collect();
    match (&src.at, &dst.at) {
        (Where::Local, Where::Local) => {
            let list: Vec<String> = srcs.iter().map(|s| q(s)).collect();
            format!("rsync -avP {} {}", list.join(" "), q(&spec(dst, None)))
        }
        (Where::Local, Where::Remote { port, key, .. })
        | (Where::Remote { port, key, .. }, Where::Local) => {
            let list: Vec<String> = srcs.iter().map(|s| q(s)).collect();
            format!(
                "rsync -avzP{} {} {}",
                ssh_e(*port, key.as_deref()),
                list.join(" "),
                q(&spec(dst, None))
            )
        }
        (
            Where::Remote {
                user: su,
                host: sh,
                port: sp,
                ..
            },
            Where::Remote { port: dp, .. },
        ) => {
            // Run rsync on the source server, pushing to the destination. -A forwards your
            // local ssh-agent so the source server can authenticate to the destination.
            let list: Vec<String> = names.iter().map(|n| q(&join(&src.dir, n))).collect();
            let dest = match &dst.at {
                Where::Remote { user, host, .. } => {
                    format!("{user}@{host}:{}/", dst.dir.trim_end_matches('/'))
                }
                Where::Local => unreachable!(),
            };
            let inner = format!(
                "rsync -avzP{} {} {}",
                ssh_e(*dp, None),
                list.join(" "),
                q(&dest)
            );
            let port = if *sp == 22 {
                String::new()
            } else {
                format!(" -p {sp}")
            };
            format!("ssh -A{port} {su}@{sh} {}", q(&inner))
        }
    }
}

// ---------- built-in runner ----------

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ssh_core::session::Connection;
use ssh_core::sftp::Progress;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Argument that turns the app binary into the rsync remote-shell bridge.
pub const BRIDGE_ARG: &str = "--rsync-rsh";
const SOCK_ENV: &str = "BASTION_RSH_SOCK";
/// Per-transfer secret the bridge must present, so nothing else can use the socket.
const TOKEN_ENV: &str = "BASTION_RSH_TOKEN";
/// Set when the local rsync passes remote arguments unescaped (openrsync, rsync < 3.2.4):
/// the bridge then quotes them so paths with spaces survive the remote shell.
const QUOTE_ENV: &str = "BASTION_RSH_QUOTE";
/// Host name in rsync paths. Never resolved: the bridge ignores it.
const FAKE_HOST: &str = "bastion";

/// The local rsync, if it runs. `quote` says whether it leaves remote arguments unescaped.
#[derive(Clone, Copy, Debug)]
pub struct Local {
    pub quote: bool,
}

pub async fn local() -> Option<Local> {
    let out = tokio::process::Command::new("rsync")
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    Some(Local {
        quote: needs_quoting(&text),
    })
}

/// rsync 3.2.4 started escaping remote arguments itself. openrsync never does.
fn needs_quoting(version_text: &str) -> bool {
    if version_text.contains("openrsync") {
        return true;
    }
    let ver = version_text
        .split_whitespace()
        .skip_while(|w| *w != "version")
        .nth(1)
        .unwrap_or("");
    let nums: Vec<u32> = ver
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .filter_map(|p| p.parse().ok())
        .collect();
    nums < vec![3, 2, 4]
}

/// Copy `names` from `src_dir` to `dst_dir` with rsync over `conn`. `upload` is local to
/// remote. Progress counts the bytes that cross the connection.
pub async fn run(
    conn: Arc<Connection>,
    local: Local,
    upload: bool,
    src_dir: &str,
    names: &[String],
    dst_dir: &str,
    progress: Arc<Progress>,
) -> ssh_core::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sock = std::env::temp_dir().join(format!(
        "bastion-rsync-{}-{}.sock",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&sock);
    let listener = tokio::net::UnixListener::bind(&sock)?;
    let token = Arc::new(token()?);
    let moved = Arc::new(AtomicU64::new(0));
    let server = {
        let (conn, moved, token) = (conn.clone(), moved.clone(), token.clone());
        let progress = progress.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (conn, moved, token) = (conn.clone(), moved.clone(), token.clone());
                let progress = progress.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, &conn, &moved, &progress, upload, &token).await;
                });
            }
        })
    };
    let r = run_rsync(
        &sock, &token, local, upload, src_dir, names, dst_dir, &progress, &moved,
    )
    .await;
    server.abort();
    let _ = std::fs::remove_file(&sock);
    if r.is_ok() {
        let total = progress.total.load(Ordering::Relaxed);
        progress.done.store(total, Ordering::Relaxed);
    }
    r
}

#[expect(
    clippy::too_many_arguments,
    reason = "each is a separate input, a struct would only be built to be taken apart"
)]
async fn run_rsync(
    sock: &std::path::Path,
    token: &str,
    local: Local,
    upload: bool,
    src_dir: &str,
    names: &[String],
    dst_dir: &str,
    progress: &Progress,
    moved: &AtomicU64,
) -> ssh_core::Result<()> {
    // Tests run as the test harness, so they point this at the real app binary.
    let exe = match std::env::var_os("BASTION_RSH_EXE") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_exe()?,
    };
    let rsh = format!("{} {BRIDGE_ARG}", q(&exe.to_string_lossy()));
    let remote = |p: String| format!("{FAKE_HOST}:{p}");
    let dst = format!("{}/", dst_dir.trim_end_matches('/'));
    let dst = if upload { remote(dst) } else { dst };
    // One run per item: openrsync can't take several remote sources at once.
    for name in names {
        let src = join(src_dir, name);
        let src = if upload { src } else { remote(src) };
        if let Ok(mut current) = progress.current.lock() {
            *current = name.clone();
        }
        let mut child = tokio::process::Command::new("rsync")
            .args(["-a", "--partial", "-e", &rsh, &src, &dst])
            .env(SOCK_ENV, sock)
            .env(TOKEN_ENV, token)
            .env(QUOTE_ENV, if local.quote { "1" } else { "0" })
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut stderr = child.stderr.take();
        let status = loop {
            tokio::select! {
                st = child.wait() => break st?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                    if progress.cancel.load(Ordering::Relaxed) {
                        let _ = child.kill().await;
                        return Err(ssh_core::Error::Cancelled);
                    }
                    let total = progress.total.load(Ordering::Relaxed);
                    let n = moved.load(Ordering::Relaxed);
                    progress.done.store(if total > 0 { n.min(total) } else { n }, Ordering::Relaxed);
                }
            }
        };
        if !status.success() {
            let mut err = String::new();
            if let Some(e) = stderr.as_mut() {
                let _ = e.read_to_string(&mut err).await;
            }
            let last = err
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            return Err(ssh_core::Error::Remote(format!(
                "rsync failed ({status}){}{last}",
                if last.is_empty() { "" } else { ": " }
            )));
        }
    }
    Ok(())
}

/// 32 hex chars from the system's random source.
fn token() -> std::io::Result<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// One line from the bridge, without the newline.
async fn read_line(sock: &mut tokio::net::UnixStream) -> std::io::Result<String> {
    let mut line = Vec::new();
    loop {
        let b = sock.read_u8().await?;
        if b == b'\n' || line.len() > 64 * 1024 {
            break;
        }
        line.push(b);
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// The bridge may only start an rsync server, nothing else, on the connection. In quoting
/// mode every argument is single-quoted by us, so file names can hold any character. Otherwise
/// rsync escapes them itself, and an unescaped shell operator means something else is going on.
fn allowed(command: &str) -> bool {
    if command.contains('\n') {
        return false;
    }
    if command.starts_with("'rsync' '--server' ") {
        return true;
    }
    let chars: Vec<char> = command.chars().collect();
    command.starts_with("rsync --server ")
        && !chars
            .iter()
            .enumerate()
            .any(|(i, c)| ";&|`$<>(){}".contains(*c) && (i == 0 || chars[i - 1] != '\\'))
}

/// One bridge connection: the token line, the remote command line, then raw bytes both ways.
async fn serve(
    mut sock: tokio::net::UnixStream,
    conn: &Connection,
    moved: &AtomicU64,
    progress: &Progress,
    upload: bool,
    token: &str,
) -> ssh_core::Result<()> {
    if read_line(&mut sock).await? != token {
        return Err(ssh_core::Error::Remote("rsync bridge: bad token".into()));
    }
    let command = read_line(&mut sock).await?;
    if !allowed(&command) {
        return Err(ssh_core::Error::Remote(
            "rsync bridge: not an rsync command".into(),
        ));
    }
    let chan = conn.exec_stream(&command).await?;
    let (mut sr, mut sw) = sock.into_split();
    let (mut cr, mut cw) = tokio::io::split(chan);
    // Count and pace only the direction that carries the files.
    let payload = Some((moved, progress));
    let (up, down) = if upload {
        (payload, None)
    } else {
        (None, payload)
    };
    let _ = tokio::join!(pump(&mut sr, &mut cw, up), pump(&mut cr, &mut sw, down));
    Ok(())
}

/// Copy until EOF, then close the write side so the other end sees EOF too.
async fn pump(
    from: &mut (impl AsyncRead + Unpin),
    to: &mut (impl AsyncWrite + Unpin),
    payload: Option<(&AtomicU64, &Progress)>,
) -> std::io::Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = from.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if let Some((_, progress)) = payload {
            progress.wait_resume().await;
        }
        to.write_all(&buf[..n]).await?;
        if let Some((moved, progress)) = payload {
            moved.fetch_add(n as u64, Ordering::Relaxed);
            tokio::time::sleep(progress.pace(n as u64)).await;
        }
    }
    to.shutdown().await
}

/// Bridge mode: rsync runs us as `<bastion> --rsync-rsh [ssh options] bastion <command...>`.
/// Plain blocking I/O, no GUI, no runtime.
pub fn bridge_main() -> ! {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let fail = |msg: &str| -> ! {
        eprintln!("bastion rsync bridge: {msg}");
        std::process::exit(255)
    };
    let args: Vec<String> = std::env::args().skip(2).collect();
    // Skip any ssh-style options (-l user, -p port...) up to the host, then the rest is the
    // remote command.
    let Some(host_at) = args.iter().position(|a| a == FAKE_HOST) else {
        fail("no host argument");
    };
    let quote = std::env::var(QUOTE_ENV).is_ok_and(|v| v == "1");
    let command = args[host_at + 1..]
        .iter()
        .map(|a| if quote { q(a) } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ");
    let Some(path) = std::env::var_os(SOCK_ENV) else {
        fail("not started by Bastion");
    };
    // openrsync hands us non-blocking pipes; plain blocking copies need blocking ones.
    for fd in [0, 1] {
        // SAFETY: fcntl on our own standard descriptors, flags only.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
            }
        }
    }
    let mut sock = UnixStream::connect(path).unwrap_or_else(|e| fail(&e.to_string()));
    let token = std::env::var(TOKEN_ENV).unwrap_or_default();
    if sock
        .write_all(format!("{token}\n{command}\n").as_bytes())
        .is_err()
    {
        fail("could not reach Bastion");
    }
    let mut to_app = sock.try_clone().unwrap_or_else(|e| fail(&e.to_string()));
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut to_app);
        let _ = to_app.shutdown(std::net::Shutdown::Write);
    });
    let mut out = std::io::stdout().lock();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match sock.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.write_all(&buf[..n]).and_then(|_| out.flush()).is_err() {
                    break;
                }
            }
        }
    }
    std::process::exit(0)
}

#[cfg(test)]
mod tests;
