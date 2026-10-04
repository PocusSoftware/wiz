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
SSHC_DOCKER_TESTS=1 cargo test -p ssh-core
```
Password server: `tester`/`hunter2` on :2222. Key-only server on :2223 (mounts docker.sock).
Manual shell: `cargo run -p ssh-core --example connect -- tester@127.0.0.1:2222 --password hunter2`

## Settings
Optional `~/Library/Application Support/dev.enty.ssh-client/settings.toml`:
```toml
ui_font = "IBM Plex Sans"
terminal_font = "JetBrainsMono Nerd Font"
terminal_font_size = 13.0
hide_sensitive = false
```
Defaults: UI font IBM Plex Sans, then Noto Sans; terminal font JetBrainsMono Nerd Font, then Menlo.

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
Hosts, known hosts and settings live in `~/Library/Application Support/dev.enty.ssh-client/`. Passwords and key passphrases are in the Keychain. Set `SSHC_CONFIG_DIR` to use a different folder (handy for testing without touching real data). If `hosts.toml` can't be parsed, a copy is kept as `hosts.toml.bak` before anything is overwritten.

## App bundle and icon
`scripts/bundle.sh` builds `dist/Bastion.app` (release binary, castle icon, ad-hoc signature). The logo source is `crates/app/assets/brand/icon.svg`; the in-app mark is `crates/app/assets/icons/castle.svg`. The config folder keeps its original name (`dev.enty.ssh-client`) so existing hosts and Keychain entries carry over.

## Hide sensitive information
Settings (sliders icon in the title bar, or ⌘,) has a "Hide sensitive information" switch. It masks hostnames, IP addresses and usernames (configured hosts and users, the local user, and any IPv4 address) in host cards, terminal output, container logs, SFTP paths, tab titles, messages and the trust dialog. Masking keeps text length, so layout and selections do not shift. Host and user fields in the add/edit form show bullets until focused. Copying still copies the real text.
