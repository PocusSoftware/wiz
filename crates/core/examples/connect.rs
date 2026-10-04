//! Raw terminal passthrough harness: proves shell, resize and password prompts without a UI.
//! Password host:  cargo run -p ssh-core --example connect -- tester@127.0.0.1:2222 --password hunter2
//! Key host:       cargo run -p ssh-core --example connect -- tester@127.0.0.1:2223 --key docker/keys/ed25519
use std::io::{Read, Write};
use std::sync::Arc;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
use ssh_core::config::{AuthMethod, Host};
use ssh_core::hostkey::{HostKeyVerifier, TrustPrompt};
use ssh_core::session::{Connection, Credentials, ShellEvent, ShellInput};

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
            "--password" => creds = Some(Credentials::Password(args.next().ok_or("value")?)),
            "--key" => {
                creds = Some(Credentials::Key {
                    path: args.next().ok_or("value")?.into(),
                    passphrase: std::env::var("KEY_PASSPHRASE").ok(),
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
        auth: AuthMethod::Password,
    };
    let creds = match creds {
        Some(c) => c,
        None => Credentials::from_keychain(&host)?,
    };
    let verifier = HostKeyVerifier {
        known_hosts: HostKeyVerifier::default_path()?,
        prompt: Arc::new(Tofu),
    };
    let conn = Connection::connect(&host, &creds, verifier).await?;
    let (cols, rows) = size()?;
    let mut shell = conn.open_shell(cols, rows, None).await?;

    enable_raw_mode()?;
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
    let mut winch = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
    let mut out = std::io::stdout();
    loop {
        tokio::select! {
            ev = shell.events.recv() => match ev {
                Some(ShellEvent::Data(d)) => { out.write_all(&d)?; out.flush()?; }
                _ => break,
            },
            _ = winch.recv() => { let (c, r) = size()?; shell.resize(c, r).await; }
        }
    }
    disable_raw_mode()?;
    Ok(())
}
