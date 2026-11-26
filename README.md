# Wiz

A fast SSH client for macOS, Windows and Linux in Rust (terminal, SFTP, Docker). Frontend: GPUI. Backend in `crates/core`.

## Status
- Done: password, key (ed25519/RSA, encrypted), ssh-agent and keyboard-interactive auth with one-time codes, jump hosts, Keychain secrets,
  known_hosts verification (TOFU, hard fail on changed key, OpenSSH host key rotation), PTY shell with resize,
  terminal emulation (alacritty_terminal), Docker list/start/stop/restart/rm/logs/exec-shell
  (through the `docker` CLI, remote over SSH or local).
- SFTP: two-pane browser, any host or local on either side (server to server streams through the app), editable path field, drag and drop between panes and from Finder or Explorer, conflict prompts, rsync hint for big transfers.
- Todo: startup time measurement.

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
Open the Settings tab with the sliders icon in the title bar or ⌘, (Ctrl+, on Windows and Linux). The keyboard icon next to it (⌘/, F1 on Windows and Linux) lists every shortcut. Everything is saved as you change it to `settings.toml` in the data folder (see Data locations; all keys optional):

| Area | Keys |
|---|---|
| Privacy | `hide_sensitive`, `mask_hosts`, `mask_users`, `mask_names` (off), `mask_ipv4`, `mask_ipv6`, `mask_ports`, `mask_secrets`, `hide_secret_files`, `secret_file_patterns`, `extra_mask` |
| Appearance | `appearance` ("system", "light" or "dark"), `contrast` ("system", "standard" or "high"), `ui_font`, `terminal_font` (restart), `accent_color` ("#RRGGBB", when colorful), `window_controls` ("left" or "right"), `oled_black`, `show_os_logos` (the OS logo found when connecting; one picked in the host form always shows), `colorful` (off: the gray theme), `animations`, `show_latency` (off), `show_split_button`, `show_quick_connect_button`, `show_privacy_button` |
| Keyboard | `shortcuts` (a table of command id to keystrokes, like `split = "cmd-alt-d"`; edited under Shortcuts in Settings) |
| Files | `show_dotfiles`, `rsync_hint_files`, `rsync_hint_mb` |
| Logs, Docker, terminal, network | `log_refresh_secs`, `log_tail_lines`, `docker_refresh_secs`, `terminal_font_size` (zoom), `terminal_scrollback`, `keepalive_secs` |
| Experimental | `max_splits` (terminals per split tab, default 4) |
| Credentials | `secret_store` (`keychain` or `file`), `agent_socket` (empty for `SSH_AUTH_SOCK`, or the OpenSSH agent pipe on Windows) |

The eye button in the title bar flips `hide_sensitive`.

## Split terminals
⌘D opens another shell on the same server in the current tab, up to 4 (`max_splits` under Experimental raises it). Two sit side by side, a third and fourth go below them as a 2x2 grid, more grow the grid. On a short window they go side by side, on a narrow one they stack. Splitting is refused when a pane would get smaller than about 320x150 px. Dividers between columns and rows can be dragged. ⌘] and ⌘[ move between the panes.

To put another server in the split, click the split button or press ⇧⌘D and pick a saved host. To join two open tabs, open the menu of the tab you want to move (right-click, or ⇧⌘M for the open tab) and pick "Merge into" the other one. Any tab can be merged, so files, Docker or logs can sit beside a shell; "Files on same server" in the split menu opens one directly. "Open in new window" in the same menu moves a tab into a window of its own, and "Move to other window" brings it back. Both work from the keyboard (Tab or the arrow keys move in the menu, Enter picks, Escape closes). All shortcuts above can be changed under Shortcuts in Settings: select the shortcut, press Enter, type the new keys (hold Cmd, Ctrl or Option).

## Keyboard
Everything works without a mouse. Tab and Shift+Tab, or the arrow keys, move between controls. Enter or Space presses the focused one. A dialog keeps Tab inside it, fields first, then its buttons. The terminal keeps Tab for the shell, so ⌘L jumps to the tab bar. There Left and Right move between tabs, Home and End go to the ends, Enter selects, Shift+Left and Shift+Right move a tab, Delete closes it, and Shift+F10 opens its menu (Up and Down to pick). In the file panes Left and Right switch between the two lists. ⌘= (or ⌘+) and ⌘- make the terminal text larger and smaller, ⌘0 sets it back to the default (Ctrl on Windows). The zoom shows under Settings, Terminal and network, with a button to reset it.

## Hosts page
Saved hosts are listed on the left, pinned ones first, then each folder, then the rest. The folder is set in the host form, typed or picked from the ones in use, and a click on a folder's title folds it. The host you pick shows on the right: open a terminal, files or Docker on it, switch to the tabs already open for it, and see how it connects (address, sign-in, jump hosts). Pin, edit, duplicate and delete are there too. Duplicate opens the add form filled in from that host, with its saved password or passphrase carried over; change the address, user or port before saving. Hosts can carry optional tags (comma separated in the host form). Typing searches by name, address, folder or tag (and shows matches inside folded folders), Up and Down pick a host, Enter opens a terminal on it, and Escape clears the search. A double click on a host opens a terminal. Right-click a host for the same actions, or the empty part of the list to add a host. In a narrow window only the list shows, and one click opens it.

## Quick connect
⌘K, the arrow button in the title bar, or type `user@host` in the hosts page search and press Enter when no saved host matches. Type `user@host:port`, `user@[::1]:2222` or paste `ssh -p 2222 user@host`, then a password or a key (empty key path tries `~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa`). The credentials stay in memory for the session, so a dropped terminal still reconnects. Tick "Save as host" to add it to the hosts page.

## Snippets
Save commands you type often under Settings, Snippets (stored in `snippets.toml` next to `hosts.toml`). ⌘J (Ctrl+Shift+J on Windows) opens a picker over the terminal in front: type to filter, arrows and Enter to pick. The + button adds a snippet and the pencil on a snippet edits or deletes it. The command is pasted without Enter, so you can read it before running it. Multi-line commands go in the file by hand.

## Credentials: Keychain or encrypted file
Settings, Credentials picks where passwords and key passphrases are kept:
- Keychain (default): macOS Keychain.
- Encrypted file: `secrets.toml` next to `hosts.toml`, mode 0600. A master password goes through Argon2id (64 MiB, 3 passes, random 32-byte salt) into a 256-bit key that only lives in memory. Each secret is sealed with AES-256-GCM under its own random nonce, with its kind and account as associated data. These are symmetric primitives, which stay safe against quantum attacks (the SSH connection itself already negotiates the hybrid post-quantum `mlkem768x25519-sha256` key exchange first). The file holds an encrypted check value, so a wrong master password is reported as such.

The file is unlocked on first use (connecting, adding or editing a host) and stays unlocked until Wiz quits or you press "Lock now". Switching stores moves every saved secret across. The master password can't be recovered: forget it and the saved passwords are gone (the hosts stay).


## Jump hosts
"Jump host" in the add/edit form names another saved host to connect through, like `ssh -J`. That host can have a jump host of its own (up to 5 in a row). The target's name is resolved by the jump host, so internal names work. A jump host already open in another tab is used as it is. Editing a jump host's address keeps the hosts that use it pointing at it; deleting it makes them connect directly, and the status line says which. The copied rsync command is not offered for hosts behind a jump host.

## One-time codes
When a server asks for more than the saved password or key (a one-time code, `AuthenticationMethods publickey,keyboard-interactive`), Wiz shows its questions in a dialog. The saved password answers only the first hidden password question, never a code question. The connect timeout pauses while the dialog is open. Automatic reconnects don't ask: they stop with "the server asks for a code", and Enter in the terminal retries and asks.

## Latency
The tab bar shows the round trip to the server of the terminal in front, measured every 3 seconds (yellow from 150 ms). Behind a jump host it covers the whole way. Typing can't feel faster than this number.

## SSH agent
Hosts set to "SSH agent" log in with the keys in an ssh-agent, offered one by one; a key the server refuses or the agent declines (a cancelled Touch ID prompt) moves on to the next. Nothing is stored for them, so they connect while the secrets file is locked. Key file hosts ask the agent first too, but only for their own key, then read the file: a key already loaded with `ssh-add` needs no passphrase. Wiz uses `SSH_AUTH_SOCK` (the system agent, also for apps started from the Dock). Other agents, like 1Password or Secretive, need their socket under Settings, Passwords. When the 1Password agent is on, that row offers a "Use 1Password" button that fills in its socket (on Windows 1Password serves the OpenSSH pipe, so nothing needs changing once its agent is on and the OpenSSH Authentication Agent service is disabled). When no agent is listening, the error names the socket or pipe it tried. Agent certificates aren't used yet.

## Host keys
On first contact Wiz asks whether to trust the host's key, and asks again with both fingerprints if it ever changes. After login OpenSSH servers announce their other host keys; Wiz asks the server to prove it holds them and remembers those of new key types, so a later switch (say from RSA to ed25519) isn't reported as a change. A different key for a type already known is never taken this way.
## Notes
- Startup: `SSHC_TIMING=1 target/release/wiz` prints launch milestones (about 170-195 ms to first render warm).
- Big SFTP transfers (500+ files or 1 GB+) show a hint with a ready-made `rsync` command. rsync must be installed on the server too.
- Directory symlinks are skipped by transfers so link loops cannot run forever. File symlinks are followed.

## macOS, Windows and Linux
macOS is the main platform. Everything that differs lives in `crates/app/src/platform/` (`macos.rs`, `windows.rs`, `linux.rs`, with what Windows and Linux share in `ctrl.rs`) and behind `cfg(target_os = "macos")` in the core's secret store. On Windows:
- Shortcuts use Ctrl+Shift (Ctrl+Shift+D split, Ctrl+Shift+S split with another server, Ctrl+Shift+M tab menu, Ctrl+Shift+K quick connect, Ctrl+Shift+J snippets, Ctrl+Shift+W close, Ctrl+Shift+L tab bar, Alt+Left and Alt+Right split panes, Ctrl+, settings), because plain Ctrl+D/W/K belong to the shell. In the terminal, copy, paste, select all and find are Ctrl+Shift+C/V/A/F; Ctrl+Shift+Alt+P types the saved password. Ctrl-click opens links. Alt+F4 closes the window; on the last window it quits (with the same open-session check).
- Missing on purpose: the Keychain (passwords always live in the encrypted secrets file), rsync transfers and the rsync hint (files go over SFTP), and the installed-editor picks (the editor setting takes a program name or path; empty opens Notepad, never the file's default app).
- Build on Windows with `cargo build --release -p ssh-app` (gives `target\release\wiz.exe`), or run the "Windows build" workflow (`gh workflow run windows.yml`), which tests on Windows and uploads `Wiz.exe` as an artifact. Release builds compile GPUI's shaders with `fxc.exe` from the Windows SDK. From a Mac, `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu` checks the Windows side (needs `brew install mingw-w64` and `rustup target add x86_64-pc-windows-gnu`).


On Linux (Wayland or X11):
- Shortcuts are the Windows ones (Ctrl+Shift+D split, Ctrl+Shift+C/V copy and paste in the terminal, and so on). Shift+Insert also pastes; plain Ctrl+V stays with the shell. Alt+F4 or the window manager's close key closes the window; on the last window it quits, with the same open-session check.
- Wiz draws its own title bar and resize borders. Drag the empty part of the tab bar to move the window, double-click it to maximize.
- Missing on purpose, like on Windows: the Keychain (passwords live in the encrypted secrets file) and rsync transfers. The editor setting picks an installed editor found on `PATH` (VS Code, Zed, Kate, GNOME Text Editor, ...) or takes a program; empty uses the first installed one, never `xdg-open`.
- Needs a Vulkan driver (`vulkan-radeon`, `vulkan-intel` or `nvidia-utils` on Arch), `libxkbcommon`, `wayland` or `libxcb`, and `fontconfig`.
- Run the "Linux build" workflow (`gh workflow run linux.yml`), which tests on Linux and uploads `wiz-linux-x86_64.tar.gz` (the `wiz` binary, a `.desktop` file and the icon). To install for one user:
  ```sh
  tar -xzf wiz-linux-x86_64.tar.gz && cd wiz
  install -Dm755 wiz ~/.local/bin/wiz
  install -Dm644 software.pocus.wiz.desktop ~/.local/share/applications/software.pocus.wiz.desktop
  install -Dm644 software.pocus.wiz.png ~/.local/share/icons/hicolor/512x512/apps/software.pocus.wiz.png
  ```
- Building on Linux needs the development packages of the above (Arch: `base-devel vulkan-icd-loader libxkbcommon wayland libxcb fontconfig`).

## Building
- `cargo build -p ssh-app` for development (incremental, a couple of seconds).
- `cargo build --profile fast -p ssh-app` for optimized local runs (incremental, no LTO, a few seconds).
- `cargo build --release -p ssh-app` for the shipped binary (thin LTO, one codegen unit, 1-3 minutes).
- SSH crypto uses `ring` (russh's default `aws-lc` compiles a large C library).

## Data locations
Hosts, snippets, known hosts and settings live in `~/Library/Application Support/software.pocus.wiz/` on macOS, `%APPDATA%\software.pocus.wiz\` on Windows and `~/.config/software.pocus.wiz/` on Linux. Passwords and key passphrases are in the Keychain, or in `secrets.toml` when the encrypted file store is picked (always on Windows and Linux). Set `SSHC_CONFIG_DIR` to use a different folder (handy for testing without touching real data). If `hosts.toml` can't be parsed, a copy is kept as `hosts.toml.bak` before anything is overwritten.

## Versioning
The version lives only in `[workspace.package]` in `Cargo.toml` (semver). `scripts/version.sh` prints it, `scripts/version.sh 0.2.0` sets it; then commit and tag `v0.2.0`. The app shows the version and build commit at the bottom of Settings, and `scripts/bundle.sh` writes it into `Info.plist` (build number = commit count).

## App bundle and icon
`scripts/bundle.sh` builds `Wiz.app` (release binary, icon, ad-hoc signature) and installs it to `/Applications`, replacing the previous build. The logo source is `crates/app/assets/brand/icon.svg`; the in-app mark is `crates/app/assets/icons/hat.svg`.

## Hide sensitive information
Masks hostnames, usernames, IPv4 and IPv6 addresses, ports (`host:2222`, `ssh -p 2222`, `Port 22`, `PORT=3000`, Docker port mappings), and the values in `PASSWORD=...` / `token: ...` style lines, in the hosts page, terminal output, container logs, SFTP paths, tab titles, messages and the trust dialog. File lists also leave out secret files (`.env`, `*.pem`, private keys, ... configurable patterns). Masking keeps text length, so layout and selections do not shift. Host, user and port fields in the add/edit form show bullets until focused. Copying still copies the real text.

## License
Wiz is MIT licensed. The bundled fonts are not: IBM Plex Sans and JetBrains Mono are under the SIL Open Font License 1.1 (texts in `crates/app/assets/fonts/`). `scripts/bundle.sh` copies both texts into `Wiz.app/Contents/Resources`; a Windows build ships them next to `Wiz.exe`, a Linux build next to `wiz`.
