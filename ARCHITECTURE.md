# Architecture

This document is the top-level system map for boo. It describes the major
runtime pieces and how they relate. For subsystem details, follow the linked
docs under [`docs/`](./docs).

## Overview

boo has four major layers:

1. App shell and UI orchestration
2. Shared VT runtime and rendering
3. Local session server and control/stream IPC
4. Remote transport layers for desktop SSH and Boo-native TCP

```text
┌──────────────────────────────────────────────┐
│ Desktop GUI (iced)                          │
│ tabs, splits, overlays, copy mode, chrome   │
└─────────────────────┬────────────────────────┘
                      │
        ┌─────────────▼─────────────┐
        │ Shared VT backend         │
        │ PTYs, snapshots, input,   │
        │ OSC 133, command state    │
        └─────────────┬─────────────┘
                      │
        ┌─────────────▼─────────────┐
        │ Session server            │
        │ panes, tabs, control IPC, │
        │ stream IPC, lifecycle     │
        └───────┬─────────┬─────────┘
                │         │
      ┌─────────▼───┐   ┌─▼──────────────────┐
      │ SSH desktop │   │ Boo-native daemon  │
      │ remote      │   │ iOS / direct remote│
      └─────────────┘   └────────────────────┘
```

## Process Model

There are three important runtime modes:

- Desktop app: local native GUI with auto-attach to a local or forwarded server
- Session server: `boo server`, the long-lived owner of sessions, PTYs, and tabs
- Headless server: `boo --headless`, optionally with `--remote-port` for native remote clients

The key architectural rule is that PTYs and session ownership belong to the
server/runtime side, not to the GUI process.

## Core Subsystems

### App Shell

Primary files:

- `src/main.rs`
- `src/tabs.rs`
- `src/splits.rs`
- `src/bindings.rs`
- `src/client_gui.rs`

Responsibilities:

- top-level `iced` app lifecycle
- pane layout and tab management
- keybinding dispatch
- copy mode, command prompt, and overlays
- attaching to the server-side control and stream surfaces

### Shared VT Runtime

Primary files:

- `src/vt_backend_core.rs`
- `src/vt_terminal_canvas.rs`
- `src/unix_pty.rs`
- `src/backend.rs`

Responsibilities:

- PTY lifecycle and IO
- terminal ownership through `libghostty-vt`
- snapshot generation
- terminal input encoding
- command-state tracking and shell integration
- terminal rendering support for both macOS and Linux

See [docs/modules/vt-backend-core.md](./docs/modules/vt-backend-core.md).

### Session Server And IPC

Primary files:

- `src/server.rs`
- `src/runtime_server.rs`
- `src/control.rs`
- `src/client_gui.rs`

Responsibilities:

- long-lived session ownership
- local Unix control socket
- local Unix `.stream` socket
- request/response RPCs and live UI/session updates

See [docs/modules/control-socket.md](./docs/modules/control-socket.md).
See [docs/modules/session-server.md](./docs/modules/session-server.md).

### Remote Transport

Remote support currently has two lanes:

- SSH-backed desktop remote: `boo --host <ssh-host>`
- Boo-native remote daemon for iOS and direct remote work

Primary files:

- `src/launch.rs`
- `src/remote.rs`
- `src/remote_*`
- `ios/Sources/*`

See:

- [docs/architecture/remote.md](./docs/architecture/remote.md)
- [docs/modules/remote-daemon.md](./docs/modules/remote-daemon.md)
- [docs/modules/ios-client.md](./docs/modules/ios-client.md)
- [docs/modules/renderer.md](./docs/modules/renderer.md)

## Platform Split

### Shared

- pane/session/runtime model
- VT ownership
- renderer
- remote server/client logic

### macOS-specific

- AppKit host integration
- IME/text input bridge
- notification center integration

### Linux-specific

- platform clipboard/event integration
- Linux host-layer behavior around the shared VT runtime

See:

- [docs/architecture/platform-macos.md](./docs/architecture/platform-macos.md)
- [docs/architecture/platform-linux.md](./docs/architecture/platform-linux.md)

## Documentation Layout

Use these docs by question:

- “What is boo?”: [README.md](./README.md)
- “How is the system structured?”: [ARCHITECTURE.md](./ARCHITECTURE.md)
- “How do I contribute?”: [CONTRIBUTING.md](./CONTRIBUTING.md)
- “What is currently planned?”: [ROADMAP.md](./ROADMAP.md)
- “Where is the deeper subsystem material?”: [docs/index.md](./docs/index.md)
