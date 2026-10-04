//! Builds `rsync` command lines for the "this transfer is big, use rsync" hint.

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

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(port: u16) -> Where {
        Where::Remote {
            user: "u".into(),
            host: "h".into(),
            port,
            key: None,
        }
    }

    #[test]
    fn upload_with_port_and_quotes() {
        let src = Endpoint {
            at: Where::Local,
            dir: "/tmp/a".into(),
        };
        let dst = Endpoint {
            at: remote(2222),
            dir: "/srv".into(),
        };
        let c = command(&src, &["it's".into(), "b".into()], &dst);
        assert_eq!(
            c,
            "rsync -avzP -e 'ssh -p 2222' '/tmp/a/it'\\''s' '/tmp/a/b' 'u@h:/srv/'"
        );
    }

    #[test]
    fn key_host_passes_identity_file_quoted() {
        let src = Endpoint {
            at: Where::Local,
            dir: "/x".into(),
        };
        let dst = Endpoint {
            at: Where::Remote {
                user: "u".into(),
                host: "h".into(),
                port: 2223,
                key: Some("/Users/me/my keys/id".into()),
            },
            dir: "/srv".into(),
        };
        let c = command(&src, &["f".into()], &dst);
        assert_eq!(
            c,
            "rsync -avzP -e 'ssh -p 2223 -i '\\''/Users/me/my keys/id'\\''' '/x/f' 'u@h:/srv/'"
        );
    }

    #[test]
    fn many_files_use_files_from_with_heredoc() {
        let src = Endpoint {
            at: Where::Local,
            dir: "/data".into(),
        };
        let dst = Endpoint {
            at: remote(2222),
            dir: "/srv".into(),
        };
        let names: Vec<String> = (1..=20).map(|i| format!("f{i}")).collect();
        let c = command(&src, &names, &dst);
        let (first, rest) = c.split_once('\n').unwrap();
        assert_eq!(
            first,
            "rsync -avrzP -e 'ssh -p 2222' --files-from=- '/data/' 'u@h:/srv/' <<'RSYNC_FILES_EOF'"
        );
        assert!(rest.starts_with("f1\nf2\n"), "{rest}");
        assert!(rest.ends_with("f20\nRSYNC_FILES_EOF"), "{rest}");
        assert_eq!(rest.lines().count(), 21);
    }

    #[test]
    fn download_default_port() {
        let src = Endpoint {
            at: remote(22),
            dir: "/var/log".into(),
        };
        let dst = Endpoint {
            at: Where::Local,
            dir: "/Users/x".into(),
        };
        assert_eq!(
            command(&src, &["app".into()], &dst),
            "rsync -avzP 'u@h:/var/log/app' '/Users/x/'"
        );
    }

    #[test]
    fn server_to_server_runs_on_source() {
        let src = Endpoint {
            at: remote(2223),
            dir: "/a".into(),
        };
        let dst = Endpoint {
            at: Where::Remote {
                user: "d".into(),
                host: "dh".into(),
                port: 22,
                key: None,
            },
            dir: "/b".into(),
        };
        let c = command(&src, &["x".into()], &dst);
        assert!(c.starts_with("ssh -A -p 2223 u@h 'rsync -avzP "), "{c}");
        assert!(c.contains("d@dh:/b/"), "{c}");
    }
}
