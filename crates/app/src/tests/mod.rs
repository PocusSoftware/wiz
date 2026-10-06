//! Tests for the app, kept out of the source files. Each file here is attached as the
//! `tests` child module of the code it tests (`#[path = ...]`), so it can reach private
//! items: `root.rs` from `root/mod.rs`, `quick_connect.rs` from `quick_connect.rs`.
//! Deleting a file means deleting that `mod tests` declaration too.

/// Held by tests that change process-wide state (settings, privacy engine, secret store).
pub static GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());
