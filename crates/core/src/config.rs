//! Everything the app keeps on disk besides secrets: saved hosts and settings.

mod hosts;
mod settings;
mod target;

pub use hosts::{AuthMethod, Config, Host};
pub use settings::{DEFAULT_SECRET_FILES, Settings, WindowControls};
pub use target::parse_target;
