# Feature Reference

This is the high-level feature reference for boo. It is the structured overview
for the feature surface and current backlog themes.

## Core Product Areas

### Terminal Multiplexing

- multiple tabs with independent split trees
- directional pane navigation and resize
- per-tab layout modes
- runtime persistence through the long-lived server model

### Terminal UX

- copy mode
- command prompt
- configurable keybindings
- search and status surfaces
- command-state tracking via shell integration

### Runtime Server

- `boo server` as the long-lived runtime owner
- GUI views the server instead of owning PTYs directly
- control socket and `.stream` socket IPC surfaces

### Remote

- SSH-backed desktop remote via `boo --host <ssh-host>`
- Boo-native remote daemon for direct and iOS clients
- iOS SwiftUI client with manual/saved/Tailscale endpoint connection and runtime-view bootstrap onto the shared server state

### Platform Runtime

- shared `libghostty-vt` runtime on macOS and Linux
- app-owned rendering and layout
- platform-specific host integration kept thinner than the shared runtime

## Backlog Themes

### tmux Parity

- remain-on-exit and respawn-pane style lifecycle controls
- session/window rename and move/link behavior
- hooks, formats, `run-shell`, and `if-shell`

### Remote

- remote path handling hardening for SSH desktop mode
- stronger transport convergence between desktop and iOS

### Performance

- continue renderer and transport profiling on representative workloads
- expand terminal-heavy regression coverage

## Related Docs

- [../architecture/remote.md](../architecture/remote.md)
- [../modules/runtime-server.md](../modules/runtime-server.md)
- [../modules/vt-backend-core.md](../modules/vt-backend-core.md)
- [../development/profiling.md](../development/profiling.md)
