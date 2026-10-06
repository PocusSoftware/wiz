# Bastion

A castle-keep for your servers: a fast, macOS-only SSH client in Rust (terminal, SFTP, Docker). Frontend: GPUI (pending Xcode). Backend in `crates/core`.

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
Manual shell: `cargo run -p ssh-core --example connect -- tester@127.0.0.1:2222 --password hunter2`

## Settings
Open the Settings tab with the sliders icon in the title bar or ⌘, . Everything is saved as you change it to `~/Library/Application Support/dev.enty.ssh-client/settings.toml` (all keys optional):

| Area | Keys |
|---|---|
| Privacy | `hide_sensitive`, `mask_hosts`, `mask_users`, `mask_ipv4`, `mask_ipv6`, `mask_ports`, `mask_secrets`, `hide_secret_files`, `secret_file_patterns`, `extra_mask` |
| Appearance | `ui_font`, `terminal_font` (restart), `terminal_font_size`, `animations` |
| Files | `show_dotfiles`, `rsync_hint_files`, `rsync_hint_mb` |
| Logs, Docker, terminal, network | `log_refresh_secs`, `log_tail_lines`, `docker_refresh_secs`, `terminal_scrollback`, `keepalive_secs` |
| Credentials | `secret_store` (`keychain` or `file`) |

The eye button in the title bar flips `hide_sensitive`.

## Split terminals
⌘D opens another shell on the same server in the current tab, up to 4. Two sit side by side, a third and fourth go below them as a 2x2 grid. On a short window they go side by side, on a narrow one they stack. Splitting is refused when a pane would get smaller than about 320x150 px. Dividers between columns and rows can be dragged.

## Quick connect
⌘K, the arrow button in the title bar, or the card on the hosts page. Type `user@host:port`, `user@[::1]:2222` or paste `ssh -p 2222 user@host`, then a password or a key (empty key path tries `~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa`). The credentials stay in memory for the session, so a dropped terminal still reconnects. Tick "Save as host" to add it to the hosts page.

## Credentials: Keychain or encrypted file
Settings, Credentials picks where passwords and key passphrases are kept:
- Keychain (default): macOS Keychain.
- Encrypted file: `secrets.toml` next to `hosts.toml`, mode 0600. A master password goes through Argon2id (64 MiB, 3 passes, random 32-byte salt) into a 256-bit key that only lives in memory. Each secret is sealed with AES-256-GCM under its own random nonce, with its kind and account as associated data. These are symmetric primitives, which stay safe against quantum attacks (the SSH connection itself already negotiates the hybrid post-quantum `mlkem768x25519-sha256` key exchange first). The file holds an encrypted check value, so a wrong master password is reported as such.

The file is unlocked on first use (connecting, adding or editing a host) and stays unlocked until Bastion quits or you press "Lock now". Switching stores moves every saved secret across. The master password can't be recovered: forget it and the saved passwords are gone (the hosts stay).

## Notes
- Startup: `SSHC_TIMING=1 target/release/bastion` prints launch milestones (about 170-195 ms to first render warm).
- Big SFTP transfers (500+ files or 1 GB+) show a hint with a ready-made `rsync` command. rsync must be installed on the server too.
- Directory symlinks are skipped by transfers so link loops cannot run forever. File symlinks are followed.

## Building
- `cargo build -p ssh-app` for development (incremental, a couple of seconds).
- `cargo build --profile fast -p ssh-app` for optimized local runs (incremental, no LTO, a few seconds).
- `cargo build --release -p ssh-app` for the shipped binary (thin LTO, one codegen unit, 1-3 minutes).
- SSH crypto uses `ring` (russh's default `aws-lc` compiles a large C library).

## Data locations
Hosts, known hosts and settings live in `~/Library/Application Support/dev.enty.ssh-client/`. Passwords and key passphrases are in the Keychain, or in `secrets.toml` when the encrypted file store is picked. Set `SSHC_CONFIG_DIR` to use a different folder (handy for testing without touching real data). If `hosts.toml` can't be parsed, a copy is kept as `hosts.toml.bak` before anything is overwritten.

## Versioning
The version lives only in `[workspace.package]` in `Cargo.toml` (semver). `scripts/version.sh` prints it, `scripts/version.sh 0.2.0` sets it; then commit and tag `v0.2.0`. The app shows the version and build commit at the bottom of Settings, and `scripts/bundle.sh` writes it into `Info.plist` (build number = commit count).

## App bundle and icon
`scripts/bundle.sh` builds `Bastion.app` (release binary, castle icon, ad-hoc signature) and installs it to `/Applications`, replacing the previous build. The logo source is `crates/app/assets/brand/icon.svg`; the in-app mark is `crates/app/assets/icons/castle.svg`. The config folder keeps its original name (`dev.enty.ssh-client`) so existing hosts and Keychain entries carry over.

## Hide sensitive information
Masks hostnames, usernames, IPv4 and IPv6 addresses, ports (`host:2222`, `ssh -p 2222`, `Port 22`, `PORT=3000`, Docker port mappings), and the values in `PASSWORD=...` / `token: ...` style lines, in host cards, terminal output, container logs, SFTP paths, tab titles, messages and the trust dialog. File lists also leave out secret files (`.env`, `*.pem`, private keys, ... configurable patterns). Masking keeps text length, so layout and selections do not shift. Host, user and port fields in the add/edit form show bullets until focused. Copying still copies the real text.
