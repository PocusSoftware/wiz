//! Saved snippets (`snippets.toml`): named commands to paste into a terminal.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    pub command: String,
}

impl Snippet {
    /// Check the fields a user typed.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Name is required.".into());
        }
        if self.name.chars().count() > 64 || self.name.chars().any(char::is_control) {
            return Err("Name must be 64 characters or fewer, on one line.".into());
        }
        if self.command.trim().is_empty() {
            return Err("Command is required.".into());
        }
        // A control character other than a newline or tab would be sent to the shell as a
        // keystroke (Ctrl+C, Escape) instead of being typed.
        if self
            .command
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err("Command can't contain control characters.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Snippets {
    #[serde(default, rename = "snippet")]
    pub items: Vec<Snippet>,
}

impl Snippets {
    pub fn default_path() -> Result<PathBuf> {
        Ok(crate::config_dir()?.join("snippets.toml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| Error::Config(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let s = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        crate::write_private(path, s.as_bytes())
    }
}
