# ssh-client

Fast, macOS-only SSH client in Rust. Frontend: GPUI (pending Xcode). Backend in `crates/core`.

## Status
- Done: password, key (ed25519/RSA, encrypted) and keyboard-interactive auth, Keychain secrets,
  known_hosts verification (TOFU, hard fail on changed key), PTY shell with resize,
  terminal emulation (alacritty_terminal), Docker list/start/stop/restart/rm/logs/exec-shell
  (remote over SSH, local via bollard).
- Todo: `crates/app` GPUI frontend (needs Xcode + Metal toolchain), ssh-agent, jump hosts, SFTP.

## Test
```sh
sh docker/gen-keys.sh && (cd docker && docker compose up -d --build)
SSHC_DOCKER_TESTS=1 cargo test -p ssh-core
```
Password server: `tester`/`hunter2` on :2222. Key-only server on :2223 (mounts docker.sock).
Manual shell: `cargo run -p ssh-core --example connect -- tester@127.0.0.1:2222 --password hunter2`
