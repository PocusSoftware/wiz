//! Raw terminal passthrough harness: proves shell, resize and password prompts without a UI.
//! Password host:  cargo run -p wiz-core --example connect -- tester@127.0.0.1:2222 --password hunter2
//! Key host:       cargo run -p wiz-core --example connect -- tester@127.0.0.1:2223 --key docker/keys/ed25519
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;

use wiz_core::config::{AuthMethod, Host};
use wiz_core::hostkey::{HostKeyVerifier, TrustPrompt};
use wiz_core::secrets::Secret;
use wiz_core::session::{Connection, Credentials, ShellEvent, ShellInput};

fn stty(args: &[&str]) -> Option<String> {
    let out = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn size() -> (u16, u16) {
    stty(&["size"])
        .and_then(|s| {
            let (rows, cols) = s.split_once(' ')?;
            Some((cols.parse().ok()?, rows.parse().ok()?))
        })
        .unwrap_or((80, 24))
}

struct Tofu;
impl TrustPrompt for Tofu {
    fn trust_new_host(&self, host: &str, port: u16, fp: &str) -> bool {
        eprint!("Trust {host}:{port} {fp}? [y/N] ");
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).ok();
        s.trim().eq_ignore_ascii_case("y")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let target = args
        .next()
        .ok_or("usage: connect user@host[:port] --password P | --key PATH")?;
    let (user, rest) = target.split_once('@').ok_or("need user@host")?;
    let (hostname, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, p.parse()?),
        None => (rest, 22),
    };
    let mut creds = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--password" => {
                let pw = args.next().ok_or("value")?;
                creds = Some(Credentials::Password(Secret::new(&pw)?));
            }
            "--key" => {
                creds = Some(Credentials::Key {
                    path: args.next().ok_or("value")?.into(),
                    passphrase: std::env::var("KEY_PASSPHRASE")
                        .ok()
                        .map(|p| Secret::new(&p))
                        .transpose()?,
                })
            }
            _ => return Err(format!("unknown arg {a}").into()),
        }
    }
    let host = Host {
        name: "cli".into(),
        host: hostname.into(),
        port,
        user: user.into(),
        group: None,
        pinned: false,
        jump: None,
        os: None,
        logo: None,
        tags: Vec::new(),
        auth: AuthMethod::Password,
    };
    let creds = match creds {
        Some(c) => c,
        None => Credentials::from_store(&host)?,
    };
    let verifier = HostKeyVerifier {
        known_hosts: HostKeyVerifier::default_path()?,
        prompt: Arc::new(Tofu),
    };
    let conn = Connection::connect(&host, &creds, verifier).await?;
    let (cols, rows) = size();
    let mut shell = conn.open_shell(cols, rows, None).await?;

    let saved = stty(&["-g"]);
    stty(&["raw", "-echo"]);
    let input = shell.input.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = std::io::stdin().read(&mut buf) {
            if n == 0
                || input
                    .blocking_send(ShellInput::Data(buf[..n].to_vec()))
                    .is_err()
            {
                break;
            }
        }
    });
    let mut out = std::io::stdout();
    #[cfg(unix)]
    {
        let mut winch =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
        loop {
            tokio::select! {
                ev = shell.events.recv() => match ev {
                    Some(ShellEvent::Data(d)) => { out.write_all(&d)?; out.flush()?; }
                    _ => break,
                },
                _ = winch.recv() => { let (c, r) = size(); shell.resize(c, r).await; }
            }
        }
    }
    // No resize signal on Windows: just pass output through.
    #[cfg(not(unix))]
    while let Some(ShellEvent::Data(d)) = shell.events.recv().await {
        out.write_all(&d)?;
        out.flush()?;
    }
    if let Some(saved) = saved {
        stty(&[&saved]);
    }
    Ok(())
}
