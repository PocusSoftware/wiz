//! SSH connections: connect and authenticate, run commands, open shells and SFTP.

mod agent;
mod auth;

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use russh::client;
use russh::keys::PublicKey;
use russh::{ChannelMsg, Disconnect};
use tokio::sync::{mpsc, oneshot};

use crate::config::{AuthMethod, Host};
use crate::hostkey::HostKeyVerifier;
use crate::secrets::{self, Kind, Secret};
use crate::{Error, Result};
pub use agent::set_socket as set_agent_socket;
pub use auth::{AuthPrompt, Question};

#[derive(Clone)]
pub enum Credentials {
    Password(Secret),
    Key {
        path: std::path::PathBuf,
        passphrase: Option<Secret>,
    },
    /// Keys from the ssh-agent; nothing is stored.
    Agent,
}

impl Credentials {
    /// Load secrets for `host` from the configured store (Keychain or secrets file).
    pub fn from_store(host: &Host) -> Result<Self> {
        let account = host.account();
        match &host.auth {
            AuthMethod::Password => secrets::get(Kind::Password, &account)?
                .map(Credentials::Password)
                .ok_or_else(|| Error::NoCredentials(format!("no password for {account}"))),
            AuthMethod::Key { path } => Ok(Credentials::Key {
                path: path.clone(),
                passphrase: secrets::get(Kind::KeyPassphrase, &account)?,
            }),
            AuthMethod::Agent => Ok(Credentials::Agent),
        }
    }
}

struct Handler {
    host: String,
    port: u16,
    verifier: HostKeyVerifier,
    /// Takes the host keys the server announces after login (`hostkeys-00@openssh.com`).
    announced: Option<oneshot::Sender<Vec<PublicKey>>>,
}

impl client::Handler for Handler {
    type Error = Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool> {
        use russh::keys::PublicKeyOrCertificate as P;
        let pk = match key {
            P::PublicKey { key, .. } => key.clone(),
            P::Certificate(c) => c.public_key().clone().into(),
        };
        // The prompt may block for the user, so keep it off the async workers.
        let (verifier, host, port) = (self.verifier.clone(), self.host.clone(), self.port);
        tokio::task::spawn_blocking(move || verifier.verify(&host, port, &pk))
            .await
            .map_err(|e| Error::Remote(e.to_string()))??;
        Ok(true)
    }

    /// Hand the keys on: proving them needs the session handle, and must not be awaited here.
    async fn openssh_ext_host_keys_announced(
        &mut self,
        keys: Vec<PublicKey>,
        _: &mut client::Session,
    ) -> Result<()> {
        if let Some(tx) = self.announced.take() {
            let _ = tx.send(keys);
        }
        Ok(())
    }
}

/// Remember the other host keys the server proves it holds, so that when it later switches
/// to one of them (say from RSA to ed25519) that is not reported as a changed key. Only keys
/// of an algorithm not yet known for the host are added, never a replacement. Holds the
/// connection only while proving, so a closed tab is never kept open by this.
async fn learn_announced(
    handle: Weak<client::Handle<Handler>>,
    announced: oneshot::Receiver<Vec<PublicKey>>,
    verifier: HostKeyVerifier,
    host: String,
    port: u16,
) {
    let Ok(keys) = announced.await else {
        return;
    };
    let (v, h) = (verifier.clone(), host.clone());
    let Ok(new) = tokio::task::spawn_blocking(move || v.unknown(&h, port, keys)).await else {
        return;
    };
    if new.is_empty() {
        return;
    }
    let Some(handle) = handle.upgrade() else {
        return;
    };
    let proven = tokio::time::timeout(Duration::from_secs(10), handle.hostkeys_prove(new.clone()));
    if !matches!(proven.await, Ok(Ok(()))) {
        return;
    }
    drop(handle);
    let _ = tokio::task::spawn_blocking(move || verifier.learn_proven(&host, port, new)).await;
}

/// How to connect, beyond the host and its credentials.
#[derive(Clone)]
pub struct Options {
    /// Time between keep-alive probes; zero turns them off.
    pub keepalive: Duration,
    /// Reach the host through this connection (a jump host), like `ssh -J`.
    pub via: Option<Arc<Connection>>,
    /// Asks the user what the server wants beyond the credentials (one-time codes). Without
    /// one such a login fails with `Error::CodeRequired`.
    pub prompt: Option<Arc<dyn AuthPrompt>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            keepalive: Duration::from_secs(30),
            via: None,
            prompt: None,
        }
    }
}

pub struct Connection {
    handle: Arc<client::Handle<Handler>>,
    /// Address connected to, to look for FTP when the server has no SFTP.
    host: String,
    /// The jump host this connection runs through, kept open as long as this one.
    via: Option<Arc<Connection>>,
}

#[derive(Debug)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: Option<u32>,
}

impl ExecOutput {
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

impl Connection {
    pub async fn connect(
        host: &Host,
        creds: &Credentials,
        verifier: HostKeyVerifier,
    ) -> Result<Self> {
        Self::connect_with(host, creds, verifier, Options::default()).await
    }

    /// Like `connect`, with `options`.
    pub async fn connect_with(
        host: &Host,
        creds: &Credentials,
        verifier: HostKeyVerifier,
        options: Options,
    ) -> Result<Self> {
        let keepalive = options.keepalive;
        let config = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: (!keepalive.is_zero()).then_some(keepalive),
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        });
        let (announced, announced_rx) = oneshot::channel();
        let handler = Handler {
            host: host.host.clone(),
            port: host.port,
            verifier: verifier.clone(),
            announced: Some(announced),
        };
        let mut handle = match &options.via {
            None => client::connect(config, (host.host.as_str(), host.port), handler).await?,
            Some(via) => {
                // The name goes to the jump host as typed, so names only it knows work.
                let channel = via
                    .handle
                    .channel_open_direct_tcpip(host.host.as_str(), host.port.into(), "127.0.0.1", 0)
                    .await?;
                client::connect_stream(config, channel.into_stream(), handler).await?
            }
        };
        let ask = auth::Asker {
            host: &host.name,
            prompt: options.prompt.as_ref(),
        };
        auth::authenticate(&mut handle, &host.user, creds, &ask).await?;
        let handle = Arc::new(handle);
        tokio::spawn(learn_announced(
            Arc::downgrade(&handle),
            announced_rx,
            verifier,
            host.host.clone(),
            host.port,
        ));
        Ok(Self {
            handle,
            host: host.host.clone(),
            via: options.via,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Round trip to the server (through any jump hosts), or `None` when it doesn't answer
    /// within 5 s or the connection is gone.
    pub async fn ping(&self) -> Option<Duration> {
        let start = Instant::now();
        tokio::time::timeout(Duration::from_secs(5), self.handle.send_ping())
            .await
            .ok()?
            .ok()?;
        // A closed connection answers at once; that is no round trip.
        (!self.is_closed()).then(|| start.elapsed())
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await?;
        Ok(())
    }

    /// Run a command without a PTY and collect its output.
    pub async fn exec(&self, command: &str) -> Result<ExecOutput> {
        let mut ch = self.handle.channel_open_session().await?;
        ch.exec(true, command).await?;
        let mut out = ExecOutput {
            stdout: vec![],
            stderr: vec![],
            status: None,
        };
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } => out.stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => out.stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => out.status = Some(exit_status),
                _ => {}
            }
        }
        Ok(out)
    }

    /// Run a command without a PTY and hand back its stdin/stdout as one byte stream
    /// (stderr is dropped). Used to carry rsync's protocol over this connection.
    pub async fn exec_stream(
        &self,
        command: &str,
    ) -> Result<impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static> {
        let ch = self.handle.channel_open_session().await?;
        ch.exec(true, command).await?;
        Ok(ch.into_stream())
    }

    /// True when `name` is on the remote PATH.
    pub async fn has_command(&self, name: &str) -> bool {
        self.exec(&format!("command -v {name} >/dev/null 2>&1"))
            .await
            .is_ok_and(|o| o.status == Some(0))
    }

    /// Open an SFTP session on this connection. A server without SFTP refuses it; then the
    /// error says whether the server offers FTP instead.
    pub async fn open_sftp(&self) -> Result<crate::sftp::Sftp> {
        let mut ch = self.handle.channel_open_session().await?;
        ch.request_subsystem(true, "sftp").await?;
        let answer = async {
            loop {
                match ch.wait().await {
                    Some(ChannelMsg::Success) => return true,
                    Some(ChannelMsg::Failure) | None => return false,
                    Some(_) => {}
                }
            }
        };
        // Servers must answer; one that never does counts as refusing.
        if !tokio::time::timeout(Duration::from_secs(10), answer)
            .await
            .unwrap_or(false)
        {
            // Behind a jump host the address may be internal; never dial it from here.
            let ftp = self.via.is_none() && crate::sftp::ftp_offered(&self.host).await;
            return Err(Error::NoSftp { ftp });
        }
        crate::sftp::Sftp::new(ch.into_stream()).await
    }

    /// Open an interactive PTY. With `command` set it runs that instead of the login shell
    /// (used for `docker exec -it`).
    pub async fn open_shell(&self, cols: u16, rows: u16, command: Option<&str>) -> Result<Shell> {
        let ch = self.handle.channel_open_session().await?;
        ch.request_pty(true, "xterm-256color", cols.into(), rows.into(), 0, 0, &[])
            .await?;
        match command {
            Some(c) => ch.exec(true, c).await?,
            None => ch.request_shell(true).await?,
        }
        let (in_tx, mut in_rx) = mpsc::channel::<ShellInput>(256);
        let (ev_tx, ev_rx) = mpsc::channel::<ShellEvent>(256);
        let (reader, writer) = ch.split();
        let mut reader = reader;

        // Writer task: keystrokes and resizes to the remote. Raw bytes, no local echo,
        // so password prompts (sudo, passwd) work: the remote PTY controls echo.
        tokio::spawn(async move {
            while let Some(inp) = in_rx.recv().await {
                let r = match inp {
                    ShellInput::Data(d) => writer.data_bytes(d).await,
                    ShellInput::Resize { cols, rows } => {
                        writer.window_change(cols.into(), rows.into(), 0, 0).await
                    }
                    ShellInput::Close => break,
                };
                if r.is_err() {
                    break;
                }
            }
            // Also reached when the UI drops its sender (tab closed): end the remote shell.
            let _ = writer.eof().await;
            let _ = writer.close().await;
        });

        tokio::spawn(async move {
            let mut status = None;
            let mut eof = false;
            loop {
                // The exit status can arrive just after EOF, so after EOF wait a moment for it
                // (or for the close) before ending.
                let msg = if eof {
                    tokio::time::timeout(Duration::from_millis(500), reader.wait())
                        .await
                        .unwrap_or_default()
                } else {
                    reader.wait().await
                };
                let Some(msg) = msg else { break };
                let ev = match msg {
                    ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                        ShellEvent::Data(data.to_vec())
                    }
                    ChannelMsg::ExitStatus { exit_status } => {
                        status = Some(exit_status);
                        continue;
                    }
                    ChannelMsg::Eof => {
                        eof = true;
                        continue;
                    }
                    ChannelMsg::Close => break,
                    _ => continue,
                };
                if ev_tx.send(ev).await.is_err() {
                    return;
                }
            }
            let _ = ev_tx.send(ShellEvent::Exit(status)).await;
        });

        Ok(Shell {
            input: in_tx,
            events: ev_rx,
        })
    }
}

#[derive(Debug)]
pub enum ShellInput {
    Data(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

#[derive(Debug)]
pub enum ShellEvent {
    Data(Vec<u8>),
    /// Remote side ended, with exit status if the server sent one.
    Exit(Option<u32>),
}

pub struct Shell {
    pub input: mpsc::Sender<ShellInput>,
    pub events: mpsc::Receiver<ShellEvent>,
}

impl Shell {
    pub async fn send(&self, data: impl Into<Vec<u8>>) {
        let _ = self.input.send(ShellInput::Data(data.into())).await;
    }
    pub async fn resize(&self, cols: u16, rows: u16) {
        let _ = self.input.send(ShellInput::Resize { cols, rows }).await;
    }
}

// Its tests need a unix ssh-agent.
#[cfg(all(test, unix))]
mod tests;
