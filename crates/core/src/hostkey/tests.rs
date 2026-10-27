use russh::keys::PublicKey;

use super::*;

const OLD: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIATKfzAVZWdpymfMMyAiA5xW1xvcBk/YozBypJWkgcYF";
const NEW: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICkaoUjGg2CaG300B1xs1rNSxHnhuRp64WA9JOvEdD7W";

struct Replace(bool);
impl TrustPrompt for Replace {
    fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
        false
    }
    fn replace_changed_host(&self, _: &str, _: u16, previous: &str, fingerprint: &str) -> bool {
        // The prompt can show both keys side by side.
        let fp = |k: &str| {
            PublicKey::from_openssh(k)
                .unwrap()
                .fingerprint(HashAlg::Sha256)
                .to_string()
        };
        assert_eq!(
            (previous, fingerprint),
            (fp(OLD).as_str(), fp(NEW).as_str())
        );
        self.0
    }
}

fn verifier(dir: &std::path::Path, replace: bool) -> HostKeyVerifier {
    let known_hosts = dir.join("known_hosts");
    // The comment shifts russh's line numbers against the file's.
    std::fs::write(
        &known_hosts,
        format!("# note\nother.example {NEW}\nh.example {OLD}\n"),
    )
    .unwrap();
    HostKeyVerifier {
        known_hosts,
        prompt: Arc::new(Replace(replace)),
    }
}

#[test]
fn changed_key_is_refused_unless_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let key = PublicKey::from_openssh(NEW).unwrap();
    let v = verifier(dir.path(), false);
    assert!(matches!(
        v.verify("h.example", 22, &key),
        Err(Error::HostKeyChanged { line: 2, .. })
    ));

    let v = verifier(dir.path(), true);
    v.verify("h.example", 22, &key).unwrap();
    let text = std::fs::read_to_string(&v.known_hosts).unwrap();
    assert!(text.starts_with(&format!("# note\nother.example {NEW}\n")));
    assert!(!text.contains(OLD.split(' ').nth(1).unwrap()));
    // The new key is now the remembered one, no prompt needed.
    verifier_check(&v, &key);
}

fn verifier_check(v: &HostKeyVerifier, key: &PublicKey) {
    let strict = HostKeyVerifier {
        known_hosts: v.known_hosts.clone(),
        prompt: Arc::new(RejectUnknown),
    };
    strict.verify("h.example", 22, key).unwrap();
}

#[test]
fn new_host_is_remembered_owner_only() {
    struct Trust;
    impl TrustPrompt for Trust {
        fn trust_new_host(&self, _: &str, _: u16, _: &str) -> bool {
            true
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let v = HostKeyVerifier {
        known_hosts: dir.path().join("known_hosts"),
        prompt: Arc::new(Trust),
    };
    let key = PublicKey::from_openssh(NEW).unwrap();
    v.verify("h.example", 22, &key).unwrap();
    verifier_check(&v, &key);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&v.known_hosts)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
