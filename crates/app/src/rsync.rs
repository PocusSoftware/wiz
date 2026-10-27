//! rsync for large transfers. Two ways:
//!
//! - Built in (`runner::run`, macOS only): the local rsync is started with `-e "<wiz> --rsync-rsh"`, so its
//!   remote shell is this app again in bridge mode (`runner::bridge_main`). The bridge hands rsync's
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
            // local ssh-agent so the source server can authenticate to the destination. -t gives
            // the inner ssh a terminal for the host key and password prompts.
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
            format!("ssh -t -A{port} {su}@{sh} {}", q(&inner))
        }
    }
}

// The built-in runner needs unix sockets and the bridge mode of the app binary.
#[cfg(unix)]
pub mod runner;

#[cfg(test)]
mod tests;
