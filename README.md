# boo

boo is a Rust terminal multiplexer and terminal UI built on `iced` and a shared
`libghostty-vt` runtime on macOS and Linux. It combines tmux-like tab and
pane management with a native GUI, a long-lived runtime server, SSH-backed
desktop remote mode, and a Boo-native remote daemon for iOS.

## Current Status

- Shared VT runtime on macOS and Linux
- Long-lived local runtime server via `boo server`
- Native desktop GUI client
- SSH-backed desktop remote mode via `boo --host <ssh-host>`
- Boo-native TCP remote daemon for the iOS client via `--remote-port`
- Active work on remote hardening, tmux parity, and performance

## Highlights

- Multiple tabs with nested split panes
- Copy mode, command prompt, and configurable keybindings
- Declarative startup layouts
- Local control socket and stream socket for GUI/runtime coordination
- Headless mode and control-socket automation
- iOS client with Bonjour discovery and runtime-view subscription

## Quick Start

Build:

```bash
cargo build
```

Build with Nix:

```bash
nix build
```

Run the desktop app:

```bash
cargo run
```

Run directly from the flake:

```bash
nix run
```

Run the long-lived server explicitly:

```bash
cargo run -- server
```

Run headless with a native remote daemon:

```bash
cargo run -- --headless --remote-port 7337
```

Run desktop remote over SSH:

```bash
cargo run -- --host my-host.local
```

The same remote path works through the flake app:

```bash
nix run . -- --host my-host.local
```

## Verification

Core Rust tests:

```bash
cargo test
```

Nix check:

```bash
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).default --no-link
```

High-value repo scripts:

- `bash scripts/test-ui-snapshot.sh`
- `bash scripts/test-ui-scenarios.sh`
- `bash scripts/test-headless.sh`
- `bash scripts/test-ios-remote-view.sh`
- `bash scripts/verify-remote-host.sh`

## Documentation Map

- [ARCHITECTURE.md](./ARCHITECTURE.md): system shape, process model, and major subsystem boundaries
- [CONTRIBUTING.md](./CONTRIBUTING.md): environment setup, workflow, tests, and contribution expectations
- [ROADMAP.md](./ROADMAP.md): current product and engineering priorities
- [docs/index.md](./docs/index.md): structured documentation hub
- [docs/reference/features.md](./docs/reference/features.md): detailed feature and capability reference

## Existing Specialized Docs

- [docs/remote/requirements.md](./docs/remote/requirements.md)
- [docs/remote/ssh-desktop.md](./docs/remote/ssh-desktop.md)
- [docs/development/profiling.md](./docs/development/profiling.md)
- [docs/architecture/platform-linux.md](./docs/architecture/platform-linux.md)
- [ios/README.md](./ios/README.md)

## Project Layout

Key top-level areas:

- `src/`: Rust application, runtime server, transport, and platform code
- `ios/`: SwiftUI iOS remote viewer
- `scripts/`: verification, profiling, benchmarking, and workflow helpers
- `shell-integration/`: shell-side integration helpers
- `bench/`: terminal benchmark helpers and notes
- `docs/`: curated project documentation

## Notes

- boo is under active development. Some docs under `docs/remote/` are product
  and implementation docs for work still in progress.
- `AGENTS.md` and `CLAUDE.md` are tool-facing repo instructions, not end-user
  project documentation.
