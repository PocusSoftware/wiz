# ssh-client

Fast, macOS-only SSH client in Rust. Frontend: GPUI (pending Xcode). Backend in `crates/core`.

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
```
Defaults: UI font IBM Plex Sans, then Noto Sans; terminal font JetBrainsMono Nerd Font, then Menlo.

## Notes
- Startup: `SSHC_TIMING=1 target/release/ssh-client` prints launch milestones (about 170-195 ms to first render warm).
- Big SFTP transfers (500+ files or 1 GB+) show a hint with a ready-made `rsync` command. rsync must be installed on the server too.
- Directory symlinks are skipped by transfers so link loops cannot run forever. File symlinks are followed.
