//! SSH connections: connect and authenticate, run commands, open shells and SFTP.

mod agent;

use std::sync::{Arc, Weak};
use std::time::Duration;

use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKeyWithHashAlg, PublicKey, load_secret_key};
use russh::{ChannelMsg, Disconnect};
use tokio::sync::{mpsc, oneshot};

use crate::config::{AuthMethod, Host};
use crate::hostkey::HostKeyVerifier;
use crate::secrets::{self, Kind, Secret};
use crate::{Error, Result};
pub use agent::set_socket as set_agent_socket;

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

pub struct Connection {
    handle: Arc<client::Handle<Handler>>,
    /// Address connected to, to look for FTP when the server has no SFTP.
    host: String,
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
        Self::connect_with(host, creds, verifier, Duration::from_secs(30)).await
    }

    /// Like `connect`, with a custom keep-alive interval (zero disables it).
    pub async fn connect_with(
        host: &Host,
        creds: &Credentials,
        verifier: HostKeyVerifier,
        keepalive: Duration,
    ) -> Result<Self> {
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
        let mut handle = client::connect(config, (host.host.as_str(), host.port), handler).await?;
        authenticate(&mut handle, &host.user, creds).await?;
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
        })
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
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
            let ftp = crate::sftp::ftp_offered(&self.host).await;
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

async fn authenticate(
    handle: &mut client::Handle<Handler>,
    user: &str,
    creds: &Credentials,
) -> Result<()> {
    match creds {
        Credentials::Password(pw) => {
            // russh takes owned strings and doesn't wipe them; keep our copy short-lived.
            let pw = pw.reveal();
            if handle
                .authenticate_password(user, pw.to_string())
                .await?
                .success()
            {
                return Ok(());
            }
            // Many servers expose password login only as keyboard-interactive.
            match handle
                .authenticate_keyboard_interactive_start(user, None)
                .await?
            {
                KeyboardInteractiveAuthResponse::Success => Ok(()),
                KeyboardInteractiveAuthResponse::Failure { .. } => Err(Error::AuthFailed),
                KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    let mut resp = handle
                        .authenticate_keyboard_interactive_respond(kbd_answers(&prompts, &pw))
                        .await?;
                    // Allow one extra empty round (some PAM stacks send a trailing request).
                    if let KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } = resp {
                        resp = handle
                            .authenticate_keyboard_interactive_respond(
                                prompts.iter().map(|_| String::new()).collect(),
                            )
                            .await?;
                    }
                    match resp {
                        KeyboardInteractiveAuthResponse::Success => Ok(()),
                        _ => Err(Error::AuthFailed),
                    }
                }
            }
        }
        Credentials::Agent => {
            let mut agent = agent::open(handle).await?;
            let keys = agent::keys(&mut agent).await?;
            if agent::login(handle, user, keys, &mut agent).await? {
                Ok(())
            } else {
                Err(Error::AuthFailed)
            }
        }
        Credentials::Key { path, passphrase } => {
            // The agent may hold this key already (unlocked, or behind Touch ID). Use it then,
            // and fall back to the file when it isn't there or declines.
            if agent::login_with_key_of(handle, user, path).await {
                return Ok(());
            }
            let passphrase = passphrase.as_ref().map(Secret::reveal);
            let key =
                load_secret_key(path, passphrase.as_deref().map(String::as_str)).map_err(|e| {
                    if passphrase.is_none() && e.to_string().to_lowercase().contains("encrypted") {
                        Error::PassphraseRequired
                    } else {
                        Error::Key(e)
                    }
                })?;
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            let ok = handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await?
                .success();
            if ok { Ok(()) } else { Err(Error::AuthFailed) }
        }
    }
}

/// Answers to a keyboard-interactive round: the password goes to the first hidden prompt
/// only, the rest get nothing. A server asking for something else as well (a one-time code)
/// then fails the login instead of receiving the password a second time.
fn kbd_answers(prompts: &[client::Prompt], password: &str) -> Vec<String> {
    let first_hidden = prompts.iter().position(|p| !p.echo);
    (0..prompts.len())
        .map(|i| {
            if Some(i) == first_hidden {
                password.to_string()
            } else {
                String::new()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
