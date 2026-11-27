//! Logins where the server asks for more than the saved credentials (a one-time code), against
//! an in-process russh server. Uses the throwaway keys from `docker/gen-keys.sh`.
//! Run with: WIZ_DOCKER_TESTS=1 cargo test -p wiz-core --test second_factor

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::keys::load_secret_key;
use russh::server::{self, Auth, Response};
use russh::{MethodKind, MethodSet};
use wiz_core::Error;
use wiz_core::config::{AuthMethod, Host};
use wiz_core::hostkey::{HostKeyVerifier, TrustPrompt};
use wiz_core::secrets::Secret;
use wiz_core::session::{AuthPrompt, Connection, Credentials, Options, Question};

const CODE: &str = "123456";

fn keys() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docker/keys")
}

struct TrustAll;
impl TrustPrompt for TrustAll {
    fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
        true
    }
}

/// Wants the password `hunter2` (or any public key), then a one-time code, both through
/// keyboard-interactive after the first step. Records every answer it gets.
#[derive(Clone)]
struct TwoFactor {
    answers: Arc<Mutex<Vec<String>>>,
    /// The first factor is done and the code question was sent.
    asked_code: bool,
}

fn only_kbd(partial: bool) -> Auth {
    Auth::Reject {
        proceed_with_methods: Some(MethodSet::from(&[MethodKind::KeyboardInteractive][..])),
        partial_success: partial,
    }
}

fn question(text: &'static str) -> Auth {
    Auth::Partial {
        name: Cow::Borrowed(""),
        instructions: Cow::Borrowed("Open your authenticator app."),
        prompts: Cow::Owned(vec![(Cow::Borrowed(text), false)]),
    }
}

impl server::Handler for TwoFactor {
    type Error = russh::Error;

    async fn auth_password(&mut self, _: &str, password: &str) -> Result<Auth, Self::Error> {
        self.answers.lock().unwrap().push(password.into());
        // Like a server that only offers keyboard-interactive for passwords.
        Ok(only_kbd(false))
    }

    async fn auth_publickey(
        &mut self,
        _: &str,
        _: server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        // `AuthenticationMethods publickey,keyboard-interactive`
        self.asked_code = true;
        Ok(only_kbd(true))
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(response) = response else {
            return Ok(if self.asked_code {
                question("Verification code: ")
            } else {
                question("Password: ")
            });
        };
        let got: Vec<String> = response
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .collect();
        self.answers.lock().unwrap().extend(got.iter().cloned());
        let answer = got.first().map(String::as_str).unwrap_or_default();
        Ok(match (self.asked_code, answer) {
            (false, "hunter2") => {
                self.asked_code = true;
                question("Verification code: ")
            }
            (true, CODE) => Auth::Accept,
            _ => Auth::reject(),
        })
    }
}

/// Serve one connection on a free local port.
async fn serve(handler: TwoFactor) -> u16 {
    let config = Arc::new(server::Config {
        keys: vec![load_secret_key(keys().join("ed25519"), None).unwrap()],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let session = server::run_stream(config, socket, handler).await.unwrap();
        let _ = session.await;
    });
    port
}

/// Answers every question with `CODE`, or cancels; records what it was shown.
struct Codes {
    cancel: bool,
    shown: Mutex<Vec<(String, String, Vec<Question>)>>,
}

impl AuthPrompt for Codes {
    fn answer(&self, host: &str, text: &str, questions: &[Question]) -> Option<Vec<String>> {
        self.shown
            .lock()
            .unwrap()
            .push((host.into(), text.into(), questions.to_vec()));
        (!self.cancel).then(|| vec![CODE.to_string(); questions.len()])
    }
}

async fn login(
    creds: Credentials,
    prompt: Option<Arc<Codes>>,
) -> (Result<Connection, Error>, Vec<String>) {
    let server = TwoFactor {
        answers: Arc::default(),
        asked_code: false,
    };
    let answers = server.answers.clone();
    let port = serve(server).await;
    let dir = tempfile::tempdir().unwrap();
    let host = Host {
        name: "two-factor".into(),
        host: "127.0.0.1".into(),
        port,
        user: "tester".into(),
        group: None,
        pinned: false,
        jump: None,
        os: None,
        logo: None,
        tags: Vec::new(),
        startup: None,
        auth: AuthMethod::Password,
    };
    let verifier = HostKeyVerifier {
        known_hosts: dir.path().join("known_hosts"),
        prompt: Arc::new(TrustAll),
    };
    let options = Options {
        prompt: prompt.map(|p| p as Arc<dyn AuthPrompt>),
        ..Default::default()
    };
    let r = Connection::connect_with(&host, &creds, verifier, options).await;
    let answers = answers.lock().unwrap().clone();
    (r, answers)
}

fn codes(cancel: bool) -> Arc<Codes> {
    Arc::new(Codes {
        cancel,
        shown: Mutex::default(),
    })
}

fn password() -> Credentials {
    Credentials::Password(Secret::new("hunter2").unwrap())
}

#[tokio::test]
async fn password_then_code() {
    if std::env::var("WIZ_DOCKER_TESTS").is_err() {
        return;
    }
    let prompt = codes(false);
    let (r, answers) = login(password(), Some(prompt.clone())).await;
    r.unwrap();
    // The password went out twice (password method, then its keyboard-interactive question)
    // and never as an answer to the code question.
    assert_eq!(answers, ["hunter2", "hunter2", CODE]);
    let shown = prompt.shown.lock().unwrap();
    assert_eq!(shown.len(), 1);
    let (host, text, questions) = &shown[0];
    assert_eq!(
        (host.as_str(), text.as_str()),
        ("two-factor", "Open your authenticator app.")
    );
    assert_eq!(
        questions,
        &[Question {
            text: "Verification code: ".into(),
            hidden: true,
        }]
    );
}

#[tokio::test]
async fn key_then_code() {
    if std::env::var("WIZ_DOCKER_TESTS").is_err() {
        return;
    }
    let creds = Credentials::Key {
        path: keys().join("ed25519"),
        passphrase: None,
    };
    let (r, answers) = login(creds, Some(codes(false))).await;
    r.unwrap();
    assert_eq!(answers, [CODE]);
}

#[tokio::test]
async fn cancelled_or_nobody_to_ask() {
    if std::env::var("WIZ_DOCKER_TESTS").is_err() {
        return;
    }
    let (r, _) = login(password(), Some(codes(true))).await;
    assert!(matches!(r, Err(Error::LoginCancelled)), "{:?}", r.err());
    // An automatic reconnect has no prompt: it must stop, not send something made up.
    let (r, answers) = login(password(), None).await;
    assert!(matches!(r, Err(Error::CodeRequired)), "{:?}", r.err());
    assert!(!answers.contains(&String::new()));
}
