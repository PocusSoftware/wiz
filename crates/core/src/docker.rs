//! Docker management through the `docker` CLI, run locally or on a host over SSH exec.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};

use serde::Deserialize;

use crate::session::{Connection, ExecOutput};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    /// e.g. "running", "exited"
    pub state: String,
    /// Human status, e.g. "Up 3 hours"
    pub status: String,
    pub ports: String,
}

impl Container {
    pub fn is_running(&self) -> bool {
        self.state == "running"
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Remove,
}

impl Action {
    fn verb(self) -> &'static str {
        match self {
            Action::Start => "start",
            Action::Stop => "stop",
            Action::Restart => "restart",
            Action::Remove => "rm",
        }
    }
}

/// Either the local daemon or a remote host reached over an existing SSH connection.
#[derive(Clone)]
pub enum Backend {
    Local,
    Remote(Arc<Connection>),
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct CliRow {
    #[serde(rename = "ID")]
    id: String,
    names: String,
    image: String,
    state: String,
    status: String,
    #[serde(default)]
    ports: String,
}

/// Reject anything that is not a plausible container id or name before it reaches a shell.
fn check_ref(s: &str) -> Result<&str> {
    let ok = !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if ok {
        Ok(s)
    } else {
        Err(Error::Docker(format!("invalid container reference: {s:?}")))
    }
}

impl Backend {
    pub async fn list(&self) -> Result<Vec<Container>> {
        let out = self
            .run(&["ps", "-a", "--no-trunc", "--format", "{{json .}}"], false)
            .await?;
        parse_cli_list(&check_status(out)?.stdout_str())
    }

    pub async fn act(&self, id: &str, action: Action) -> Result<()> {
        let out = self.run(&[action.verb(), check_ref(id)?], false).await?;
        check_status(out).map(drop)
    }

    pub async fn logs(&self, id: &str, tail: u32) -> Result<String> {
        let tail = tail.to_string();
        let out = self
            .run(
                &["logs", "--timestamps", "--tail", &tail, check_ref(id)?],
                true,
            )
            .await?;
        Ok(out.stdout_str())
    }

    async fn run(&self, args: &[&str], merge: bool) -> Result<ExecOutput> {
        match self {
            Backend::Local => {
                let args: Vec<String> = args.iter().map(ToString::to_string).collect();
                tokio::task::spawn_blocking(move || run_local(&args, merge))
                    .await
                    .map_err(|e| Error::Docker(e.to_string()))?
            }
            Backend::Remote(conn) => conn.exec(&remote_command(args, merge)).await,
        }
    }
}

/// Command line for an interactive shell inside a container, for `Connection::open_shell`.
pub fn exec_shell_command(id: &str) -> Result<String> {
    let id = check_ref(id)?;
    Ok(format!(
        "docker exec -it {id} sh -c 'command -v bash >/dev/null && exec bash || exec sh'"
    ))
}

fn check_status(out: ExecOutput) -> Result<ExecOutput> {
    if out.status == Some(0) {
        Ok(out)
    } else {
        Err(Error::Docker(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

fn remote_command(args: &[&str], merge: bool) -> String {
    let mut cmd = String::from("docker");
    for a in args {
        if a.contains(['{', ' ']) {
            cmd.push_str(&format!(" '{a}'"));
        } else {
            cmd.push(' ');
            cmd.push_str(a);
        }
    }
    if merge {
        cmd.push_str(" 2>&1");
    }
    cmd
}

fn run_local(args: &[String], merge: bool) -> Result<ExecOutput> {
    let mut cmd = Command::new(docker_bin());
    cmd.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let not_found = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => Error::Docker("docker command not found".into()),
        _ => e.into(),
    };
    let (stdout, stderr, status) = if merge {
        let (mut reader, writer) = std::io::pipe()?;
        cmd.stdout(writer.try_clone()?).stderr(writer);
        let mut child = cmd.spawn().map_err(not_found)?;
        drop(cmd);
        let mut stdout = Vec::new();
        reader.read_to_end(&mut stdout)?;
        (stdout, Vec::new(), child.wait()?)
    } else {
        let out = cmd.output().map_err(not_found)?;
        (out.stdout, out.stderr, out.status)
    };
    Ok(ExecOutput {
        stdout,
        stderr,
        status: status.code().and_then(|c| u32::try_from(c).ok()),
    })
}

fn docker_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs = std::env::split_paths(&path).chain(install_dirs());
        find_bin(dirs, DOCKER).unwrap_or_else(|| PathBuf::from(DOCKER))
    })
}

const DOCKER: &str = if cfg!(windows) {
    "docker.exe"
} else {
    "docker"
};

fn find_bin(dirs: impl IntoIterator<Item = PathBuf>, name: &str) -> Option<PathBuf> {
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

#[cfg(target_os = "macos")]
fn install_dirs() -> Vec<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".orbstack/bin"),
        home.join(".docker/bin"),
        home.join(".rd/bin"),
        PathBuf::from("/Applications/Docker.app/Contents/Resources/bin"),
    ]
}

#[cfg(windows)]
fn install_dirs() -> Vec<PathBuf> {
    std::env::var_os("ProgramFiles")
        .map(|p| PathBuf::from(p).join(r"Docker\Docker\resources\bin"))
        .into_iter()
        .collect()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn install_dirs() -> Vec<PathBuf> {
    Vec::new()
}

fn parse_cli_list(s: &str) -> Result<Vec<Container>> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let r: CliRow = serde_json::from_str(l)
                .map_err(|e| Error::Docker(format!("bad docker output: {e}")))?;
            Ok(Container {
                id: r.id,
                name: r.names,
                image: r.image,
                state: r.state,
                status: r.status,
                ports: r.ports,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Tests for `docker`.

    use super::*;

    #[test]
    fn parses_cli_json() {
        let l = r#"{"ID":"abc","Names":"web","Image":"nginx","State":"running","Status":"Up 1 hour","Ports":"0.0.0.0:80->80/tcp"}"#;
        let v = parse_cli_list(l).unwrap();
        assert_eq!(v[0].name, "web");
        assert!(v[0].is_running());
    }

    #[test]
    fn rejects_shell_metachars() {
        assert!(check_ref("a; rm -rf /").is_err());
        assert!(check_ref("web-1_a.b").is_ok());
    }

    #[test]
    fn remote_command_quotes_only_the_template() {
        assert_eq!(
            remote_command(&["ps", "-a", "--format", "{{json .}}"], false),
            "docker ps -a --format '{{json .}}'"
        );
        assert_eq!(
            remote_command(&["logs", "--tail", "5", "web"], true),
            "docker logs --tail 5 web 2>&1"
        );
    }

    #[test]
    fn finds_docker_in_install_dirs() {
        let empty = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join(DOCKER), "").unwrap();
        let dirs = [empty.path().to_path_buf(), bin.path().to_path_buf()];
        assert_eq!(find_bin(dirs, DOCKER), Some(bin.path().join(DOCKER)));
        assert_eq!(find_bin([empty.path().to_path_buf()], DOCKER), None);
    }
}
