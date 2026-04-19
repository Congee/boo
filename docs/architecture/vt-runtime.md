# Shared VT Runtime

boo uses one shared `libghostty-vt`-based runtime model on macOS and Linux.

Primary files:

- `src/vt_backend_core.rs`
- `src/backend.rs`
- `src/unix_pty.rs`
- `src/vt_terminal_canvas.rs`

Responsibilities:

- PTY lifecycle and IO
- terminal state ownership
- snapshot generation
- input encoding
- shell integration and command-state tracking
- rendering support for the app shell

Important design rule:

- platform layers stay thin; terminal ownership and state logic stay in the
  shared runtime where possible

See also:

- [../modules/vt-backend-core.md](../modules/vt-backend-core.md)
- [../architecture/platform-linux.md](./platform-linux.md)
- [../architecture/platform-macos.md](./platform-macos.md)
