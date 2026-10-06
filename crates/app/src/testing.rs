//! Shared test helpers.

/// Held by tests that change process-wide state (settings, privacy engine, secret store).
pub static GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());
