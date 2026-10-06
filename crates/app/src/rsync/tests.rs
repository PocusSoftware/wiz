//! Tests for `rsync`.

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

/// End to end against the docker test server (`docker/`), with rsync installed there.
/// Run: cargo build && SSHC_DOCKER_TESTS=1 cargo test -p ssh-app rsync_over_connection

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
