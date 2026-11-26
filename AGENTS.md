# Rules for working on Wiz

Wiz is an SSH client for macOS, Windows and Linux in Rust: `crates/core` (backend, no UI) and `crates/app` (GPUI).
These rules apply to every change, by people and by agents. A change is done only when
the checks at the end pass.

## 1. Correct

- **Errors are values.** Fallible code returns `ssh_core::Result` with a variant of
  `ssh_core::Error`. Add a variant when callers need to react to a case (like `Locked`);
  otherwise reuse one. No `unwrap()`/`expect()` outside tests, except for invariants
  the code itself guarantees, and then `expect("why it can't fail")`.
- **Never block the UI thread.** Network goes through `runtime::spawn`. Keychain access,
  key derivation and other slow sync work go through `spawn_blocking` or
  `cx.background_spawn`.
- **Secrets stay secret.** Passwords and passphrases live only in the secret store
  (`ssh_core::secrets`), never in `hosts.toml`, `settings.toml`, logs, status messages or
  error text. Error messages never repeat what the user typed into a masked field.
- **Privacy masking.** Any host, user, address or port shown in the UI goes through
  `ssh_core::privacy::redact` / `privacy::port`.
- **Files.** Config and secrets are written to a temp file and renamed into place. Secret
  files are created with mode 0600. Code honors `SSHC_CONFIG_DIR`, and tests always set it
  to a temporary folder.
- **Untrusted input.** Hosts, users and ports are validated (`Host::validate`). Anything
  passed to a remote shell is quoted or validated (see `docker::Remote`).
- **`unsafe`** only with a `// SAFETY:` comment saying why it holds (enforced by lint).
- **Behavior changes come with a test** that fails without the change.

## 2. Nothing unnecessary

- Write the smallest change that does the job. No speculative options, parameters,
  traits, generics or config for cases nobody asked for.
- Before writing a helper, search for an existing one (`privacy`, `widgets`, `config`,
  `secrets`, `runtime`). Two copies of the same logic means one should go.
- No wrappers that only forward, no `.clone()` of something that is moved anyway, no
  `collect()` just to iterate again (several of these are enforced by lints).
- Delete instead of commenting out. No dead code, unused imports, unused dependencies or
  `TODO`s left behind. Remove anything that a change made unused.
- New dependency only when nothing in the tree covers it. Check `Cargo.lock` first, and
  prefer crates already pulled in (for example `aes-gcm` and `argon2` came with russh).
- Silencing a lint uses `#[expect(lint, reason = "...")]`, never `#[allow]`. A stale
  `expect` then fails the build.
- Comments say *why*, not *what*. Match the density of the surrounding code: a doc line
  on most items, short notes where something is not obvious.

## 3. Rust style

- **Formatting:** `cargo fmt` with default settings (the Rust Style Guide). No
  `rustfmt.toml`.
- **Naming** follows the Rust API Guidelines: `snake_case` functions and modules,
  `UpperCamelCase` types, `SCREAMING_SNAKE_CASE` consts. Getters have no `get_` prefix.
  Conversions are `as_`/`to_`/`into_`. Booleans read as questions (`is_closed`, `ready`).
- **Arguments:** take `&str`, `&Path` or `&[T]` rather than owned values when only
  reading, and take small `Copy` types by value. Take ownership only when it is stored.
- **Control flow:** early returns, `let ... else`, `if let` chains (edition 2024).
  Iterators over index loops. `match` over long `if`/`else` chains on the same value.
- **Types:** derive `Debug`, `Clone`, `PartialEq` where they make sense. Enums over
  boolean flags or strings for states. `Option` instead of sentinel values.
- **Imports:** grouped `std`, external crates, then `crate::`, each block sorted
  (rustfmt does this). No glob imports except `gpui::prelude::*`, `crate::theme::*`, and
  `use super::*` in a module's own submodules (`workspace/`, `views/sftp/`, ...) and tests.
- **Docs:** every file starts with a `//!` line saying what it is for. Public items and
  non-obvious private ones get `///`.
- **Modules:** a module with submodules is `foo.rs` next to a `foo/` folder (no `mod.rs`).
  `mod` lines come first, then `use` (an inline `mod tests` goes at the bottom). Re-export (`pub use`) to keep public paths short,
  as `ssh_core::config` does.
- **Size:** split a file by concern once it passes roughly 500 lines (see
  `crates/app/src/views/sftp/`); inline tests don't count. Functions over about 80 lines
  usually hide a helper.

## 4. Project conventions

- `crates/core` must build without the UI. GPUI code lives only in `crates/app`.
- **Layout:**
  - `crates/core/src`: `config/` (hosts, settings, quick connect targets), `secrets/`
    (Keychain, encrypted file), `session`, `hostkey`, `sftp`, `docker`, `terminal`,
    `privacy` (masking), `rsync` (command lines and the bridge over an SSH connection,
    unix only).
  - `crates/app/src`: `workspace/` (the window: tabs, splits, connecting, dialogs on top),
    `views/` (what a tab or page shows), `dialogs/` (forms shown over the window),
    `widgets/` (buttons, text input, animations), `platform/` (what differs per OS), plus
    `prefs`, `runtime`, `theme`, `assets` at the top.
- **Platforms:** macOS is the main platform and must keep behaving the same. Anything
  OS-specific goes in `platform/macos.rs`, `platform/windows.rs` and `platform/linux.rs`,
  with the same functions in each (what Windows and Linux share lives in `platform/ctrl.rs`); the rest of the app calls `crate::platform::...` and never checks the
  OS itself. A feature that only makes sense on macOS is left out on Windows and Linux (Keychain,
  rsync) rather than faked. In core, gate with `cfg(target_os = "macos")`.
- **New setting:** add an `Option` field to `Settings`, a default in the `getters!`
  macro, a row in `views/settings.rs`, and a line in the README settings table.
- **Split modules** (`workspace`, `views/sftp`, `views/terminal`): types several
  submodules use live in the parent file, so every submodule can reach their fields. A
  type only one concern uses (a sub-struct of `Workspace`, say) lives with its methods in
  that concern's submodule, `pub(super)` where the parent needs it. Behavior lives in the
  submodule for its concern. Methods used across submodules are `pub(super)`,
  everything else stays private.
- **Tests:** unit tests go at the bottom of the file in `#[cfg(test)] mod tests { use
  super::*; ... }`. Only when `foo.rs` already has a `foo/` folder for submodules do they
  go in `foo/tests.rs`, declared with `#[cfg(test)] mod tests;`. Never add a folder just
  for tests. Backend tests that only need the public API go in `crates/core/tests/`.
  Tests that change process-wide state hold `testing::GLOBALS`. Tests that need the SSH
  servers check `SSHC_DOCKER_TESTS`.
- **Text:** UI text uses sentence case and plain words. No em dashes anywhere (code,
  comments, UI, docs).
- **Commits:** a single short summary line, a body only for a non-obvious reason. No
  AI co-author or "generated with" trailers.

## 5. Checks before calling a change done

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets      # zero warnings: warnings count as failures
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu   # the Windows side
# The Linux side: in a Linux container or with `gh workflow run linux.yml`.
cargo test --workspace
# Connection, SFTP, Docker or secret store changes, with the servers from docker/ running:
SSHC_DOCKER_TESTS=1 cargo test --workspace
```

Then read the diff once more against sections 1 and 2: is every added line needed, is
anything left unused, does a test cover the change?
