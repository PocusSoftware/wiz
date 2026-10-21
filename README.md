# Wiz

A fast SSH client for macOS and Windows in Rust (terminal, SFTP, Docker). Frontend: GPUI. Backend in `crates/core`.

## Status
- Done: password, key (ed25519/RSA, encrypted) and keyboard-interactive auth, Keychain secrets,
  known_hosts verification (TOFU, hard fail on changed key), PTY shell with resize,
  terminal emulation (alacritty_terminal), Docker list/start/stop/restart/rm/logs/exec-shell
  (remote over SSH, local via bollard).
- SFTP: two-pane browser, any host or local on either side (server to server streams through the app), editable path field, drag and drop, conflict prompts, rsync hint for big transfers.
- Todo: ssh-agent, jump hosts, startup time measurement.

## Test
```sh
sh docker/gen-keys.sh && (cd docker && docker compose up -d --build)
SSHC_DOCKER_TESTS=1 cargo test --workspace
```
Unit tests sit in a `tests.rs` next to the code they test (`views/sftp/tests.rs` for `views/sftp.rs`). Backend tests that use only the public API are in `crates/core/tests/`.
Password server: `tester`/`hunter2` on :2222. Key-only server on :2223 (mounts docker.sock).
Server without SFTP, FTP on :21 instead: `tester`/`hunter2` on :2224.
Manual shell: `cargo run -p ssh-core --example connect -- tester@127.0.0.1:2222 --password hunter2`

## Settings
Open the Settings tab with the sliders icon in the title bar or ⌘, (Ctrl+, on Windows). Everything is saved as you change it to `settings.toml` in the data folder (see Data locations; all keys optional):

| Area | Keys |
|---|---|
| Privacy | `hide_sensitive`, `mask_hosts`, `mask_users`, `mask_ipv4`, `mask_ipv6`, `mask_ports`, `mask_secrets`, `hide_secret_files`, `secret_file_patterns`, `extra_mask` |
| Appearance | `ui_font`, `terminal_font` (restart), `terminal_font_size`, `accent_color` ("#RRGGBB"), `window_controls` ("left" or "right"), `animations` |
| Files | `show_dotfiles`, `rsync_hint_files`, `rsync_hint_mb` |
| Logs, Docker, terminal, network | `log_refresh_secs`, `log_tail_lines`, `docker_refresh_secs`, `terminal_scrollback`, `keepalive_secs` |
| Credentials | `secret_store` (`keychain` or `file`) |

The eye button in the title bar flips `hide_sensitive`.

## Split terminals
⌘D opens another shell on the same server in the current tab, up to 4. Two sit side by side, a third and fourth go below them as a 2x2 grid. On a short window they go side by side, on a narrow one they stack. Splitting is refused when a pane would get smaller than about 320x150 px. Dividers between columns and rows can be dragged. ⌘] and ⌘[ move between the panes.

## Keyboard
Everything works without a mouse. Tab and Shift+Tab, or the arrow keys, move between controls. Enter or Space presses the focused one. A dialog keeps Tab inside it, fields first, then its buttons. The terminal keeps Tab for the shell, so ⌘L jumps to the tab bar. There Left and Right move between tabs, Home and End go to the ends, Enter selects, Shift+Left and Shift+Right move a tab, Delete closes it, and Shift+F10 opens its menu (Up and Down to pick). In the file panes Left and Right switch between the two lists.

## Quick connect
⌘K, the arrow button in the title bar, or the card on the hosts page. Type `user@host:port`, `user@[::1]:2222` or paste `ssh -p 2222 user@host`, then a password or a key (empty key path tries `~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa`). The credentials stay in memory for the session, so a dropped terminal still reconnects. Tick "Save as host" to add it to the hosts page.

## Credentials: Keychain or encrypted file
Settings, Credentials picks where passwords and key passphrases are kept:
- Keychain (default): macOS Keychain.
- Encrypted file: `secrets.toml` next to `hosts.toml`, mode 0600. A master password goes through Argon2id (64 MiB, 3 passes, random 32-byte salt) into a 256-bit key that only lives in memory. Each secret is sealed with AES-256-GCM under its own random nonce, with its kind and account as associated data. These are symmetric primitives, which stay safe against quantum attacks (the SSH connection itself already negotiates the hybrid post-quantum `mlkem768x25519-sha256` key exchange first). The file holds an encrypted check value, so a wrong master password is reported as such.

The file is unlocked on first use (connecting, adding or editing a host) and stays unlocked until Wiz quits or you press "Lock now". Switching stores moves every saved secret across. The master password can't be recovered: forget it and the saved passwords are gone (the hosts stay).

## Notes
- Startup: `SSHC_TIMING=1 target/release/wiz` prints launch milestones (about 170-195 ms to first render warm).
- Big SFTP transfers (500+ files or 1 GB+) show a hint with a ready-made `rsync` command. rsync must be installed on the server too.
- Directory symlinks are skipped by transfers so link loops cannot run forever. File symlinks are followed.

## macOS and Windows
macOS is the main platform. Everything that differs lives in `crates/app/src/platform/` (`macos.rs`, `windows.rs`) and behind `cfg(target_os = "macos")` in the core's secret store. On Windows:
- Shortcuts use Ctrl+Shift (Ctrl+Shift+D split, Ctrl+Shift+K quick connect, Ctrl+Shift+W close, Ctrl+Shift+L tab bar, Alt+Left and Alt+Right split panes, Ctrl+, settings), because plain Ctrl+D/W/K belong to the shell. In the terminal, copy, paste, select all and find are Ctrl+Shift+C/V/A/F; Ctrl+Shift+Alt+P types the saved password. Ctrl-click opens links. Alt+F4 quits (with the same open-session check).
- Missing on purpose: the Keychain (passwords always live in the encrypted secrets file), rsync transfers and the rsync hint (files go over SFTP), and the installed-editor picks (the editor setting takes a program name or path; empty opens Notepad, never the file's default app).
- Build on Windows with `cargo build --release -p ssh-app` (gives `target\release\wiz.exe`), or run the "Windows build" workflow (`gh workflow run windows.yml`), which tests on Windows and uploads `Wiz.exe` as an artifact. Release builds compile GPUI's shaders with `fxc.exe` from the Windows SDK. From a Mac, `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu` checks the Windows side (needs `brew install mingw-w64` and `rustup target add x86_64-pc-windows-gnu`).

## Building
- `cargo build -p ssh-app` for development (incremental, a couple of seconds).
- `cargo build --profile fast -p ssh-app` for optimized local runs (incremental, no LTO, a few seconds).
- `cargo build --release -p ssh-app` for the shipped binary (thin LTO, one codegen unit, 1-3 minutes).
- SSH crypto uses `ring` (russh's default `aws-lc` compiles a large C library).

## Data locations
Hosts, known hosts and settings live in `~/Library/Application Support/dev.enty.ssh-client/` on macOS and `%APPDATA%\dev.enty.ssh-client\` on Windows. Passwords and key passphrases are in the Keychain, or in `secrets.toml` when the encrypted file store is picked (always on Windows). Set `SSHC_CONFIG_DIR` to use a different folder (handy for testing without touching real data). If `hosts.toml` can't be parsed, a copy is kept as `hosts.toml.bak` before anything is overwritten.

## Versioning
The version lives only in `[workspace.package]` in `Cargo.toml` (semver). `scripts/version.sh` prints it, `scripts/version.sh 0.2.0` sets it; then commit and tag `v0.2.0`. The app shows the version and build commit at the bottom of Settings, and `scripts/bundle.sh` writes it into `Info.plist` (build number = commit count).

## App bundle and icon
`scripts/bundle.sh` builds `Wiz.app` (release binary, icon, ad-hoc signature) and installs it to `/Applications`, replacing the previous build. The logo source is `crates/app/assets/brand/icon.svg`; the in-app mark is `crates/app/assets/icons/castle.svg`. The config folder keeps its original name (`dev.enty.ssh-client`) so existing hosts and Keychain entries carry over.

## Hide sensitive information
Masks hostnames, usernames, IPv4 and IPv6 addresses, ports (`host:2222`, `ssh -p 2222`, `Port 22`, `PORT=3000`, Docker port mappings), and the values in `PASSWORD=...` / `token: ...` style lines, in host cards, terminal output, container logs, SFTP paths, tab titles, messages and the trust dialog. File lists also leave out secret files (`.env`, `*.pem`, private keys, ... configurable patterns). Masking keeps text length, so layout and selections do not shift. Host, user and port fields in the add/edit form show bullets until focused. Copying still copies the real text.
