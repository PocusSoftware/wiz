//! Everything that differs between operating systems, behind one set of functions.
//! `platform/macos.rs` is the macOS side, `platform/windows.rs` the Windows side; the rest
//! of the app calls these and never checks the OS itself. Some features exist only on
//! macOS (the Keychain, rsync transfers, picking an installed editor) and are simply
//! missing on Windows.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(target_os = "macos", windows)))]
compile_error!("Wiz runs on macOS and Windows");
