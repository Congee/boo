# Feature Reference

This is the high-level feature reference for boo. The full backlog-style detail
still lives in [../../FEATURES.md](../../FEATURES.md), but this page is the
structured overview.

## Core Product Areas

### Terminal Multiplexing

- multiple tabs with independent split trees
- directional pane navigation and resize
- per-tab layout modes
- session persistence through the long-lived server model

### Terminal UX

- copy mode
- command prompt
- configurable keybindings
- search and status surfaces
- command-state tracking via shell integration

### Session Server

- `boo server` as the long-lived session owner
- GUI attaches to the server instead of owning PTYs directly
- control socket and `.stream` socket IPC surfaces

### Remote

- SSH-backed desktop remote via `boo --host <ssh-host>`
- Boo-native remote daemon for direct and iOS clients
- iOS SwiftUI client with Bonjour discovery and session attach

### Platform Runtime

- shared `libghostty-vt` runtime on macOS and Linux
- app-owned rendering and layout
- platform-specific host integration kept thinner than the shared runtime

## Detailed Sources

- [../../FEATURES.md](../../FEATURES.md)
- [../architecture/remote.md](../architecture/remote.md)
- [../modules/session-server.md](../modules/session-server.md)
- [../modules/vt-backend-core.md](../modules/vt-backend-core.md)
