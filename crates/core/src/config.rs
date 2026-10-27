//! Everything the app keeps on disk besides secrets: saved hosts and settings.

mod hosts;
mod settings;
mod target;

pub use hosts::{AuthMethod, Config, Host};
pub use settings::{DEFAULT_SECRET_FILES, Settings, WindowControls};
pub use target::parse_target;

/// `path` with a leading `~/` turned into the home folder, as people type key and socket paths.
pub fn expand_home(path: &str) -> std::path::PathBuf {
    match (path.strip_prefix("~/"), std::env::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => path.into(),
    }
}
