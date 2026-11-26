//! Logging in with keys held by an ssh-agent: the system one, 1Password, Secretive, a
//! YubiKey. A key the server refuses or the agent declines (Touch ID cancelled) moves on to
//! the next way in instead of failing the whole login.

use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use russh::Signer;
use russh::client::{self, AuthResult};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKey, PublicKey};

use super::Handler;
use crate::{Error, Result};

#[cfg(unix)]
type Agent = AgentClient<tokio::net::UnixStream>;
#[cfg(windows)]
type Agent = AgentClient<tokio::net::windows::named_pipe::NamedPipeClient>;

/// Where the agent listens, when not the default (`SSH_AUTH_SOCK`, or the OpenSSH agent's
/// pipe on Windows). Apps started from the Dock only see the system agent, so 1Password or
/// Secretive need their socket set here.
static SOCKET: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Use the agent at `path` from now on, or the default with `None`.
pub fn set_socket(path: Option<PathBuf>) {
    *SOCKET.write().unwrap_or_else(PoisonError::into_inner) = path;
}

/// Connect to the agent and tie it to this session, so agents that limit keys to certain
/// hosts (OpenSSH destination constraints) know where the login goes.
pub(super) async fn open(handle: &client::Handle<Handler>) -> Result<Agent> {
    let socket = SOCKET
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let at = socket
        .as_deref()
        .map_or_else(|| DEFAULT.to_string(), |p| p.display().to_string());
    #[cfg(unix)]
    let agent = match socket {
        Some(p) => AgentClient::connect_uds(p).await,
        None => AgentClient::connect_env().await,
    };
    #[cfg(windows)]
    let agent =
        AgentClient::connect_named_pipe(socket.unwrap_or_else(|| PathBuf::from(DEFAULT))).await;
    let mut agent = agent.map_err(|e| agent_error(&e, &at))?;
    if let Some(binding) = handle.agent_session_binding() {
        // Older and third-party agents don't know this; they work without it.
        let _ = agent.bind_session(binding, false).await;
    }
    Ok(agent)
}

#[cfg(unix)]
const DEFAULT: &str = "SSH_AUTH_SOCK";
#[cfg(windows)]
const DEFAULT: &str = r"\\.\pipe\openssh-ssh-agent";

pub(super) fn agent_error(e: &russh::keys::Error, at: &str) -> Error {
    use russh::keys::Error as E;
    let missing = match e {
        E::IO(io) => io.kind() == std::io::ErrorKind::NotFound,
        E::EnvVar(_) | E::BadAuthSock => true,
        _ => false,
    };
    Error::NoCredentials(if missing {
        format!(
            "no SSH agent is running at {at}. For 1Password, turn on Settings, Developer, Use the SSH agent"
        )
    } else {
        format!("can't reach the SSH agent at {at}: {e}")
    })
}

/// The public keys the agent holds. Certificates are left out: logging in with them is
/// not supported.
pub(super) async fn keys(agent: &mut Agent) -> Result<Vec<PublicKey>> {
    Ok(agent
        .request_identities()
        .await?
        .into_iter()
        .filter_map(|id| match id {
            AgentIdentity::PublicKey { key, .. } => Some(key),
            AgentIdentity::Certificate { .. } => None,
        })
        .collect())
}

/// Offer `keys` one by one until the server takes one: logged in, or accepted with another
/// step to go (a one-time code).
pub(super) async fn login<S: Signer>(
    handle: &mut client::Handle<Handler>,
    user: &str,
    keys: Vec<PublicKey>,
    signer: &mut S,
) -> Result<AuthResult> {
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    for key in keys {
        // Servers hang up after too many tries (MaxAuthTries).
        if handle.is_closed() {
            break;
        }
        let hash = key.algorithm().is_rsa().then_some(hash).flatten();
        if let Ok(r) = handle
            .authenticate_publickey_with(user, key, hash, signer)
            .await
            && matches!(
                r,
                AuthResult::Success
                    | AuthResult::Failure {
                        partial_success: true,
                        ..
                    }
            )
        {
            return Ok(r);
        }
    }
    Ok(AuthResult::Failure {
        remaining_methods: russh::MethodSet::empty(),
        partial_success: false,
    })
}

/// Log in with the agent's copy of the key in `path`, if it holds one. Only that key is
/// offered, so the server doesn't see every key in the agent. `None` when that didn't get
/// anywhere: the caller then reads the file itself.
pub(super) async fn login_with_key_of(
    handle: &mut client::Handle<Handler>,
    user: &str,
    path: &Path,
) -> Option<AuthResult> {
    // The public half of an OpenSSH key file is readable without the passphrase.
    let public = PrivateKey::read_openssh_file(path)
        .ok()?
        .public_key()
        .clone();
    let mut agent = open(handle).await.ok()?;
    if !keys(&mut agent).await.ok()?.contains(&public) {
        return None;
    }
    login(handle, user, vec![public], &mut agent)
        .await
        .ok()
        .filter(|r| {
            !matches!(
                r,
                AuthResult::Failure {
                    partial_success: false,
                    ..
                }
            )
        })
}
