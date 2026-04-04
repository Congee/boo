# Linux Backend

Linux now uses the same shared `libghostty-vt` runtime model as macOS. Boo owns
the PTY lifecycle, scrollback snapshots, input encoding, rendering, and layout;
the old embedded Ghostty EGL readback path is no longer part of the app.

## Current Architecture

- `src/backend.rs` selects the Linux backend implementation.
- `src/vt_backend_core.rs` owns the shared pane runtime, snapshots, and command
  lifecycle state.
- `src/vt_terminal_canvas.rs` renders terminal snapshots into Boo's UI.
- `src/platform/linux.rs` only handles host integration such as clipboard and
  platform event plumbing.
- `src/unix_pty.rs` owns PTY process creation, resize, IO, and shutdown.

## Notes

- The historical `ghostty/` submodule and EGL embedding experiments are no
  longer required to build or run Boo.
- Linux and macOS now share the same terminal runtime model, which keeps the
  remaining platform-specific code much thinner.
