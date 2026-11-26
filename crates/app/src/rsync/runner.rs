//! Running rsync over Wiz's own SSH connection: the local rsync starts the app binary as
//! its remote shell (`--rsync-rsh`), and that bridge talks to the running app over a unix
//! socket, which pipes the bytes through an SSH exec channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ssh_core::session::Connection;
use ssh_core::sftp::Progress;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::*;

/// Argument that turns the app binary into the rsync remote-shell bridge.
pub const BRIDGE_ARG: &str = "--rsync-rsh";
const SOCK_ENV: &str = "WIZ_RSH_SOCK";
/// Per-transfer secret the bridge must present, so nothing else can use the socket.
const TOKEN_ENV: &str = "WIZ_RSH_TOKEN";
/// Set when the local rsync passes remote arguments unescaped (openrsync, rsync < 3.2.4):
/// the bridge then quotes them so paths with spaces survive the remote shell.
const QUOTE_ENV: &str = "WIZ_RSH_QUOTE";
/// Host name in rsync paths. Never resolved: the bridge ignores it.
const FAKE_HOST: &str = "wiz";

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
        "wiz-rsync-{}-{}.sock",
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
    let exe = match std::env::var_os("WIZ_RSH_EXE") {
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

/// Bridge mode: rsync runs us as `<wiz> --rsync-rsh [ssh options] wiz <command...>`.
/// Plain blocking I/O, no GUI, no runtime.
pub fn bridge_main() -> ! {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let fail = |msg: &str| -> ! {
        eprintln!("wiz rsync bridge: {msg}");
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
        fail("not started by Wiz");
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
        fail("could not reach Wiz");
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
mod tests {
    //! Tests for the rsync runner and bridge.

    use super::*;

    #[tokio::test]
    async fn rsync_over_connection() {
        if std::env::var("SSHC_DOCKER_TESTS").is_err() {
            eprintln!("skipped (set SSHC_DOCKER_TESTS=1)");
            return;
        }
        use ssh_core::config::{AuthMethod, Host};
        use ssh_core::hostkey::{HostKeyVerifier, TrustPrompt};
        use ssh_core::secrets::Secret;
        use ssh_core::session::Credentials;
        struct TrustAll;
        impl TrustPrompt for TrustAll {
            fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
                true
            }
        }
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        // SAFETY: set once, before anything reads it.
        unsafe { std::env::set_var("WIZ_RSH_EXE", root.join("target/debug/wiz")) };
        let tmp = std::env::temp_dir().join(format!("wiz-rsync-test-{}", std::process::id()));
        let (up, down) = (tmp.join("up"), tmp.join("down"));
        std::fs::create_dir_all(up.join("dir with space")).unwrap();
        std::fs::create_dir_all(&down).unwrap();
        std::fs::write(up.join("dir with space/it's.txt"), "hello").unwrap();
        std::fs::write(up.join("big.bin"), vec![7u8; 300_000]).unwrap();

        let host = Host {
            name: "t".into(),
            host: "127.0.0.1".into(),
            port: 2222,
            user: "tester".into(),
            group: None,
            pinned: false,
            jump: None,
            os: None,
            logo: None,
            tags: Vec::new(),
            auth: AuthMethod::Password,
        };
        let verifier = HostKeyVerifier {
            known_hosts: tmp.join("known_hosts"),
            prompt: Arc::new(TrustAll),
        };
        let conn = Arc::new(
            Connection::connect(
                &host,
                &Credentials::Password(Secret::new("hunter2").unwrap()),
                verifier,
            )
            .await
            .unwrap(),
        );
        let local = local().await.expect("local rsync");
        assert!(conn.has_command("rsync").await);
        let remote_dir = format!("/tmp/wiz rsync {}", std::process::id());
        conn.exec(&format!("mkdir -p {}", q(&remote_dir)))
            .await
            .unwrap();
        let names = vec!["dir with space".to_string(), "big.bin".to_string()];
        let up_s = up.to_string_lossy().to_string();
        let down_s = down.to_string_lossy().to_string();

        let p = Progress::new();
        p.set_prescan(300_005, 2);
        run(
            conn.clone(),
            local,
            true,
            &up_s,
            &names,
            &remote_dir,
            p.clone(),
        )
        .await
        .unwrap();
        assert_eq!(p.done.load(Ordering::Relaxed), 300_005);
        let p = Progress::new();
        run(conn.clone(), local, false, &remote_dir, &names, &down_s, p)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(down.join("dir with space/it's.txt")).unwrap(),
            "hello"
        );
        assert_eq!(std::fs::read(down.join("big.bin")).unwrap().len(), 300_000);
        // The speed limit holds: 300 KB at 150 KB/s takes about two seconds.
        conn.exec(&format!("rm -rf {}/big.bin", q(&remote_dir)))
            .await
            .unwrap();
        let p = Progress::new();
        p.limit.store(150_000, Ordering::Relaxed);
        let started = std::time::Instant::now();
        run(
            conn.clone(),
            local,
            true,
            &up_s,
            &["big.bin".to_string()],
            &remote_dir,
            p,
        )
        .await
        .unwrap();
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );

        conn.exec(&format!("rm -rf {}", q(&remote_dir)))
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bridge_only_runs_rsync_servers() {
        assert!(allowed("rsync --server -logDtpre.iLsfxCIvu . /tmp/x/"));
        assert!(allowed("'rsync' '--server' '-g' '.' '/tmp/a b/'"));
        assert!(!allowed("rm -rf /"));
        assert!(!allowed("rsync --server ; rm -rf /"));
        assert!(allowed("rsync --server . /tmp/a\\$b/"));
        assert!(allowed("'rsync' '--server' '.' '/tmp/a;$b/'"));
        assert!(!allowed("rsync --daemon"));
    }

    #[test]
    fn quoting_follows_rsync_version() {
        assert!(needs_quoting(
            "openrsync: protocol version 29\nrsync version 2.6.9 compatible"
        ));
        assert!(needs_quoting("rsync  version 3.2.3  protocol version 31"));
        assert!(!needs_quoting("rsync  version 3.2.4  protocol version 31"));
        assert!(!needs_quoting("rsync  version v3.4.1  protocol version 32"));
    }
}
