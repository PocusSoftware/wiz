//! Builds `rsync` command lines for the "this transfer is big, use rsync" hint.

#[derive(Clone, Debug)]
pub enum Where {
    Local,
    Remote {
        user: String,
        host: String,
        port: u16,
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

fn ssh_e(port: u16) -> String {
    if port == 22 {
        String::new()
    } else {
        format!(" -e 'ssh -p {port}'")
    }
}

/// Command to copy `names` from `src` into `dst`'s directory.
pub fn command(src: &Endpoint, names: &[String], dst: &Endpoint) -> String {
    let srcs: Vec<String> = names.iter().map(|n| spec(src, Some(n))).collect();
    match (&src.at, &dst.at) {
        (Where::Local, Where::Local) => {
            let list: Vec<String> = srcs.iter().map(|s| q(s)).collect();
            format!("rsync -avP {} {}", list.join(" "), q(&spec(dst, None)))
        }
        (Where::Local, Where::Remote { port, .. }) | (Where::Remote { port, .. }, Where::Local) => {
            let list: Vec<String> = srcs.iter().map(|s| q(s)).collect();
            format!(
                "rsync -avzP{} {} {}",
                ssh_e(*port),
                list.join(" "),
                q(&spec(dst, None))
            )
        }
        (
            Where::Remote {
                user: su,
                host: sh,
                port: sp,
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
            let inner = format!("rsync -avzP{} {} {}", ssh_e(*dp), list.join(" "), q(&dest));
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
            },
            dir: "/b".into(),
        };
        let c = command(&src, &["x".into()], &dst);
        assert!(c.starts_with("ssh -A -p 2223 u@h 'rsync -avzP "), "{c}");
        assert!(c.contains("d@dh:/b/"), "{c}");
    }
}
