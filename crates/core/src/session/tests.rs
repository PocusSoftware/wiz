//! Tests for `session`.

use super::*;

#[test]
fn a_missing_agent_points_to_the_1password_switch() {
    let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
    let e = agent::agent_error(&russh::keys::Error::IO(missing), "/tmp/agent.sock");
    assert!(
        e.to_string()
            .contains("no SSH agent is running at /tmp/agent.sock")
    );
    assert!(e.to_string().contains("Use the SSH agent"));

    let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    let e = agent::agent_error(&russh::keys::Error::IO(denied), "/tmp/agent.sock");
    assert!(
        e.to_string()
            .contains("can't reach the SSH agent at /tmp/agent.sock")
    );
}

/// Logging in through a throwaway ssh-agent against the docker key server (`docker/`).
mod agent_login {
    use std::path::PathBuf;

    use russh::keys::agent::client::AgentClient;
    use russh::keys::{PrivateKey, PublicKey, load_secret_key};
    use russh::{AgentAuthError, Signer};

    use super::*;
    use crate::hostkey::TrustPrompt;

    struct TrustAll;
    impl TrustPrompt for TrustAll {
        fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
            true
        }
    }

    fn key_file(name: &str, pass: Option<&str>) -> PrivateKey {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docker/keys");
        load_secret_key(dir.join(name), pass).unwrap()
    }

    /// An ssh-agent on its own socket holding `keys`, stopped when dropped.
    struct TestAgent {
        child: std::process::Child,
        socket: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl TestAgent {
        async fn start(keys: &[PrivateKey]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("agent.sock");
            let child = std::process::Command::new("ssh-agent")
                .arg("-D")
                .arg("-a")
                .arg(&socket)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            for _ in 0..200 {
                if socket.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let agent = Self {
                child,
                socket,
                _dir: dir,
            };
            let mut client = agent.client().await;
            for k in keys {
                client.add_identity(k, &[]).await.unwrap();
            }
            agent
        }

        async fn client(&self) -> AgentClient<tokio::net::UnixStream> {
            AgentClient::connect_uds(&self.socket).await.unwrap()
        }
    }

    impl Drop for TestAgent {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Signs through the agent but declines `refuse`, like a cancelled Touch ID prompt.
    struct Declines {
        agent: AgentClient<tokio::net::UnixStream>,
        refuse: PublicKey,
        declined: usize,
    }

    impl Signer for Declines {
        type Error = AgentAuthError;

        async fn auth_sign(
            &mut self,
            key: &russh::keys::agent::AgentIdentity,
            hash_alg: Option<russh::keys::HashAlg>,
            to_sign: Vec<u8>,
        ) -> std::result::Result<Vec<u8>, Self::Error> {
            if *key.public_key() == self.refuse {
                self.declined += 1;
                return Err(AgentAuthError::Key(russh::keys::Error::AgentFailure));
            }
            self.agent.auth_sign(key, hash_alg, to_sign).await
        }
    }

    #[tokio::test]
    async fn a_declined_key_moves_on_to_the_next() {
        if std::env::var("WIZ_DOCKER_TESTS").is_err() {
            return;
        }
        let (ed, rsa) = (
            key_file("ed25519", None),
            key_file("rsa_enc", Some("testpass")),
        );
        let agent = TestAgent::start(&[ed.clone(), rsa.clone()]).await;
        let dir = tempfile::tempdir().unwrap();
        let handler = Handler {
            host: "127.0.0.1".into(),
            port: 2223,
            verifier: HostKeyVerifier {
                known_hosts: dir.path().join("known_hosts"),
                prompt: Arc::new(TrustAll),
            },
            announced: None,
        };
        let mut handle =
            client::connect(Arc::new(Default::default()), ("127.0.0.1", 2223), handler)
                .await
                .unwrap();
        let mut signer = Declines {
            agent: agent.client().await,
            refuse: ed.public_key().clone(),
            declined: 0,
        };
        let keys = vec![ed.public_key().clone(), rsa.public_key().clone()];
        let r = agent::login(&mut handle, "tester", keys, &mut signer)
            .await
            .unwrap();
        assert!(r.success());
        assert_eq!(signer.declined, 1);
    }

    #[tokio::test]
    async fn agent_logins_and_the_key_file_fallback() {
        if std::env::var("WIZ_DOCKER_TESTS").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let verifier = HostKeyVerifier {
            known_hosts: dir.path().join("known_hosts"),
            prompt: Arc::new(TrustAll),
        };
        let host = Host {
            name: "t".into(),
            host: "127.0.0.1".into(),
            port: 2223,
            user: "tester".into(),
            group: None,
            pinned: false,
            jump: None,
            os: None,
            logo: None,
            tags: Vec::new(),
            auth: AuthMethod::Agent,
        };
        let connect = |creds: Credentials| {
            let (host, verifier) = (host.clone(), verifier.clone());
            async move { Connection::connect(&host, &creds, verifier).await }
        };
        let encrypted = Credentials::Key {
            path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docker/keys/rsa_enc"),
            passphrase: None,
        };

        let empty = TestAgent::start(&[]).await;
        set_agent_socket(Some(empty.socket.clone()));
        assert!(matches!(
            connect(Credentials::Agent).await,
            Err(Error::AuthFailed)
        ));
        // Not in the agent: the file is read, and it needs its passphrase.
        assert!(matches!(
            connect(encrypted.clone()).await,
            Err(Error::PassphraseRequired)
        ));

        let full = TestAgent::start(&[key_file("rsa_enc", Some("testpass"))]).await;
        set_agent_socket(Some(full.socket.clone()));
        let c = connect(Credentials::Agent).await.unwrap();
        assert_eq!(
            c.exec("whoami").await.unwrap().stdout_str().trim(),
            "tester"
        );
        // The agent's copy of the key file stands in for the passphrase.
        connect(encrypted).await.unwrap();
        set_agent_socket(None);
    }
}
