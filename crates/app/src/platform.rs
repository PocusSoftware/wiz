//! Everything that differs between operating systems, behind one set of functions.
//! `platform/macos.rs` is the macOS side, `platform/windows.rs` the Windows side and
//! `platform/linux.rs` the Linux side, with what Windows and Linux share in
//! `platform/ctrl.rs`; the rest of the app calls these and never checks the OS itself.
//! Some features exist only on macOS (the Keychain, rsync transfers) and are simply
//! missing elsewhere.

#[cfg(not(target_os = "macos"))]
mod ctrl;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
compile_error!("Wiz runs on macOS, Windows and Linux");
