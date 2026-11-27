# Wiz

A fast SSH client for macOS, Windows and Linux, written in Rust with [GPUI](https://www.gpui.rs).

- Terminal with splits, tabs, jump hosts, ssh-agent, keys, passwords and one-time codes
- Two-pane SFTP between your machine and any server, or server to server
- Docker containers and logs, locally or on a server
- Passwords in the macOS Keychain or an encrypted file
- Updates itself from GitHub releases

## Install

Download the latest build from [Releases](https://github.com/PocusSoftware/wiz/releases/latest).

- **macOS** (Apple Silicon): unzip and move `Wiz.app` to Applications. It isn't notarized, so open it the first time with right click, Open.
- **Windows** (x64): run `Wiz-windows-x64.exe`.
- **Linux** (x86_64): needs Vulkan, `libxkbcommon`, Wayland or X11 and `fontconfig`.
  ```sh
  tar -xzf wiz-linux-x86_64.tar.gz && install -Dm755 wiz/wiz ~/.local/bin/wiz
  ```

## Build

```sh
cargo run -p wiz                 # development
cargo build --release -p wiz     # release binary
scripts/bundle.sh                    # macOS: build Wiz.app and install it
```

## Test

```sh
sh docker/gen-keys.sh && (cd docker && docker compose up -d --build)
WIZ_DOCKER_TESTS=1 cargo test --workspace
```

The test servers listen on 2222 (password `tester`/`hunter2`), 2223 (key only) and 2224 (FTP, no SFTP).

## License

[LGPL-3.0](COPYING.LESSER). The bundled fonts, IBM Plex Sans and JetBrains Mono, are under the SIL Open Font License 1.1 (texts in `crates/app/assets/fonts/`).
