//! Docker management: local daemon via bollard, remote via `docker` CLI over SSH exec.

use std::sync::Arc;

use bollard::Docker;
use bollard::query_parameters::{
    ListContainersOptionsBuilder, RestartContainerOptions, StartContainerOptions,
    StopContainerOptions,
};
use serde::Deserialize;

use crate::session::Connection;
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

// ---------- local (bollard) ----------

pub struct Local {
    docker: Docker,
}

impl Local {
    /// Connects to the default socket. Lazy: no I/O until the first call.
    pub fn new() -> Result<Self> {
        Ok(Self {
            docker: Docker::connect_with_local_defaults()?,
        })
    }

    pub async fn list(&self) -> Result<Vec<Container>> {
        let opts = ListContainersOptionsBuilder::default().all(true).build();
        let list = self.docker.list_containers(Some(opts)).await?;
        Ok(list
            .into_iter()
            .map(|c| Container {
                id: c.id.unwrap_or_default(),
                name: c
                    .names
                    .and_then(|n| n.into_iter().next())
                    .unwrap_or_default()
                    .trim_start_matches('/')
                    .to_string(),
                image: c.image.unwrap_or_default(),
                state: c.state.map(|s| s.to_string()).unwrap_or_default(),
                status: c.status.unwrap_or_default(),
                ports: c
                    .ports
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|p| p.public_port.map(|pp| format!("{pp}->{}", p.private_port)))
                    .collect::<Vec<_>>()
                    .join(", "),
            })
            .collect())
    }

    pub async fn logs(&self, id: &str, tail: u32) -> Result<String> {
        use bollard::query_parameters::LogsOptionsBuilder;
        use futures_util::StreamExt;
        let opts = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .tail(&tail.to_string())
            .build();
        let mut stream = self.docker.logs(id, Some(opts));
        let mut out = String::new();
        while let Some(chunk) = stream.next().await {
            out.push_str(&chunk?.to_string());
        }
        Ok(out)
    }

    pub async fn act(&self, id: &str, action: Action) -> Result<()> {
        match action {
            Action::Start => {
                self.docker
                    .start_container(id, None::<StartContainerOptions>)
                    .await?
            }
            Action::Stop => {
                self.docker
                    .stop_container(id, None::<StopContainerOptions>)
                    .await?
            }
            Action::Restart => {
                self.docker
                    .restart_container(id, None::<RestartContainerOptions>)
                    .await?
            }
            Action::Remove => self.docker.remove_container(id, None).await?,
        }
        Ok(())
    }
}

// ---------- remote (docker CLI over ssh exec) ----------

pub struct Remote<'a> {
    conn: &'a Connection,
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
        Err(Error::Remote(format!("invalid container reference: {s:?}")))
    }
}

impl<'a> Remote<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub async fn list(&self) -> Result<Vec<Container>> {
        let out = self
            .conn
            .exec("docker ps -a --no-trunc --format '{{json .}}'")
            .await?;
        if out.status != Some(0) {
            return Err(Error::Remote(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        parse_cli_list(&out.stdout_str())
    }

    pub async fn act(&self, id: &str, action: Action) -> Result<()> {
        let id = check_ref(id)?;
        let out = self
            .conn
            .exec(&format!("docker {} {id}", action.verb()))
            .await?;
        if out.status == Some(0) {
            Ok(())
        } else {
            Err(Error::Remote(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    }

    pub async fn logs(&self, id: &str, tail: u32) -> Result<String> {
        let id = check_ref(id)?;
        let out = self
            .conn
            .exec(&format!("docker logs --tail {tail} {id} 2>&1"))
            .await?;
        Ok(out.stdout_str())
    }

    /// Command line for an interactive shell inside a container, for `Connection::open_shell`.
    pub fn exec_shell_command(id: &str) -> Result<String> {
        let id = check_ref(id)?;
        Ok(format!(
            "docker exec -it {id} sh -c 'command -v bash >/dev/null && exec bash || exec sh'"
        ))
    }
}

/// Either the local daemon or a remote host reached over an existing SSH connection.
#[derive(Clone)]
pub enum Backend {
    Local(Arc<Local>),
    Remote(Arc<Connection>),
}

impl Backend {
    pub async fn list(&self) -> Result<Vec<Container>> {
        match self {
            Backend::Local(l) => l.list().await,
            Backend::Remote(c) => Remote::new(c).list().await,
        }
    }

    pub async fn act(&self, id: &str, action: Action) -> Result<()> {
        match self {
            Backend::Local(l) => l.act(id, action).await,
            Backend::Remote(c) => Remote::new(c).act(id, action).await,
        }
    }

    pub async fn logs(&self, id: &str, tail: u32) -> Result<String> {
        match self {
            Backend::Local(l) => l.logs(id, tail).await,
            Backend::Remote(c) => Remote::new(c).logs(id, tail).await,
        }
    }
}

fn parse_cli_list(s: &str) -> Result<Vec<Container>> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let r: CliRow = serde_json::from_str(l)
                .map_err(|e| Error::Remote(format!("bad docker output: {e}")))?;
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
mod tests;
