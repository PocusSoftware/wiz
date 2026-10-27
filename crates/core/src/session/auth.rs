//! Logging in: password, key file or agent, then whatever the server asks on top with
//! keyboard-interactive (a one-time code, a second password).

use std::sync::Arc;

use russh::MethodKind;
use russh::client::{self, AuthResult, KeyboardInteractiveAuthResponse as Kbd};
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};

use super::{Credentials, Handler, agent};
use crate::secrets::Secret;
use crate::{Error, Result};

/// One question of a keyboard-interactive round, as the server sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub text: String,
    /// What is typed stays hidden (codes, passwords).
    pub hidden: bool,
}

/// Asks the user what a server wants beyond the saved credentials, like a one-time code.
pub trait AuthPrompt: Send + Sync {
    /// `host` names who asks, `instructions` is the server's text above the questions (may be
    /// empty). One answer per question, or `None` to cancel the login. Blocks until answered.
    fn answer(&self, host: &str, instructions: &str, questions: &[Question])
    -> Option<Vec<String>>;
}

/// Servers may chain methods (`AuthenticationMethods publickey,keyboard-interactive`); this
/// many steps after the first are followed.
const MAX_STEPS: usize = 3;
/// Rounds of questions within one keyboard-interactive login.
const MAX_ROUNDS: usize = 10;

/// Who asks and how to reach the user, for keyboard-interactive questions.
pub(super) struct Asker<'a> {
    pub host: &'a str,
    pub prompt: Option<&'a Arc<dyn AuthPrompt>>,
}

pub(super) async fn authenticate(
    handle: &mut client::Handle<Handler>,
    user: &str,
    creds: &Credentials,
    ask: &Asker<'_>,
) -> Result<()> {
    let mut result = match creds {
        Credentials::Password(pw) => {
            // russh takes owned strings and doesn't wipe them; keep our copy short-lived.
            let pw = pw.reveal();
            match handle.authenticate_password(user, pw.to_string()).await? {
                // Many servers expose password login only as keyboard-interactive.
                AuthResult::Failure {
                    partial_success: false,
                    ..
                } => keyboard_interactive(handle, user, Some(&pw), ask).await?,
                r => r,
            }
        }
        Credentials::Agent => {
            let mut agent = agent::open(handle).await?;
            let keys = agent::keys(&mut agent).await?;
            agent::login(handle, user, keys, &mut agent).await?
        }
        Credentials::Key { path, passphrase } => {
            // The agent may hold this key already (unlocked, or behind Touch ID). Use it then,
            // and fall back to the file when it isn't there or declines.
            match agent::login_with_key_of(handle, user, path).await {
                Some(r) => r,
                None => key_file(handle, user, path, passphrase.as_ref()).await?,
            }
        }
    };
    for _ in 0..MAX_STEPS {
        match result {
            AuthResult::Success => return Ok(()),
            AuthResult::Failure {
                partial_success: true,
                remaining_methods,
            } if remaining_methods.contains(&MethodKind::KeyboardInteractive) => {
                result = keyboard_interactive(handle, user, None, ask).await?;
            }
            AuthResult::Failure { .. } => break,
        }
    }
    Err(Error::AuthFailed)
}

async fn key_file(
    handle: &mut client::Handle<Handler>,
    user: &str,
    path: &std::path::Path,
    passphrase: Option<&Secret>,
) -> Result<AuthResult> {
    let passphrase = passphrase.map(Secret::reveal);
    let key = load_secret_key(path, passphrase.as_deref().map(String::as_str)).map_err(|e| {
        if passphrase.is_none() && e.to_string().to_lowercase().contains("encrypted") {
            Error::PassphraseRequired
        } else {
            Error::Key(e)
        }
    })?;
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    Ok(handle
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await?)
}

/// Answer the server's rounds of questions. The saved `password` answers the first hidden
/// question, once; everything else goes to the user.
async fn keyboard_interactive(
    handle: &mut client::Handle<Handler>,
    user: &str,
    mut password: Option<&str>,
    ask: &Asker<'_>,
) -> Result<AuthResult> {
    let mut resp = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await?;
    for _ in 0..MAX_ROUNDS {
        let (name, instructions, prompts) = match resp {
            Kbd::Success => return Ok(AuthResult::Success),
            Kbd::Failure {
                remaining_methods,
                partial_success,
            } => {
                return Ok(AuthResult::Failure {
                    remaining_methods,
                    partial_success,
                });
            }
            Kbd::InfoRequest {
                name,
                instructions,
                prompts,
            } => (name, instructions, prompts),
        };
        let (password_at, user_at) = split_round(&prompts, password.is_some());
        let mut answers = vec![String::new(); prompts.len()];
        if let (Some(i), Some(pw)) = (password_at, password.take()) {
            answers[i] = pw.to_string();
        }
        if !user_at.is_empty() {
            let questions = user_at
                .iter()
                .map(|&i| Question {
                    text: prompts[i].prompt.clone(),
                    hidden: !prompts[i].echo,
                })
                .collect();
            let text = [name, instructions]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            for (i, a) in user_at
                .into_iter()
                .zip(ask_user(ask, text, questions).await?)
            {
                answers[i] = a;
            }
        }
        resp = handle
            .authenticate_keyboard_interactive_respond(answers)
            .await?;
    }
    Ok(AuthResult::Failure {
        remaining_methods: russh::MethodSet::empty(),
        partial_success: false,
    })
}

/// Which question of a round gets the saved password (the first hidden one, when there is a
/// password left to give), and which go to the user.
fn split_round(prompts: &[client::Prompt], has_password: bool) -> (Option<usize>, Vec<usize>) {
    let password_at = prompts
        .iter()
        .position(|p| !p.echo)
        .filter(|_| has_password);
    let user_at = (0..prompts.len())
        .filter(|&i| Some(i) != password_at)
        .collect();
    (password_at, user_at)
}

async fn ask_user(ask: &Asker<'_>, text: String, questions: Vec<Question>) -> Result<Vec<String>> {
    // Without a way to ask (an automatic reconnect), a one-time code can't be made up.
    let Some(prompt) = ask.prompt.cloned() else {
        return Err(Error::CodeRequired);
    };
    let host = ask.host.to_string();
    let n = questions.len();
    let mut answers = tokio::task::spawn_blocking(move || prompt.answer(&host, &text, &questions))
        .await
        .map_err(|e| Error::Remote(e.to_string()))?
        .ok_or(Error::LoginCancelled)?;
    answers.resize(n, String::new());
    Ok(answers)
}

#[cfg(test)]
mod tests;
