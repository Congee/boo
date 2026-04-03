# Linux `libghostty-vt` Migration

This project is moving the Linux backend away from patched full `libghostty`
surface embedding and toward `libghostty-vt`.

## Why

The old Linux path relied on an experimental offscreen EGL readback flow:

1. create a private EGL context
2. let patched `libghostty` render into it
3. `glReadPixels` into system memory
4. show the result through an Iced image widget

That path required local patches in `ghostty/` and does not match upstream
Ghostty's supported Linux rendering model.

`libghostty-vt` is the upstream cross-platform terminal core and is the right
base for Boo's Linux implementation.

## Current transition state

- `ghostty/` is clean again with no local EGL embedding patches.
- Linux build plumbing now links both `libghostty.so` and `libghostty-vt.so`.
- [src/vt.rs](/home/example/dev/boo/src/vt.rs) provides the first Rust wrapper for:
  - terminal creation and resize
  - VT stream writes
  - render-state snapshots
  - key encoding
  - mouse encoding
- [src/unix_pty.rs](/home/example/dev/boo/src/unix_pty.rs) now owns Linux PTY creation,
  child process spawning, IO reads, writes, and resize signaling in Rust.
- [src/linux_vt_backend.rs](/home/example/dev/boo/src/linux_vt_backend.rs) combines
  the PTY layer with `libghostty-vt` terminal state, render-state updates, and
  a serializable snapshot model for future Iced/wgpu rendering.

## Next implementation steps

1. Introduce a backend boundary so Linux panes stop depending directly on
   `ghostty_surface_t`.
2. Add a Linux pane state object backed by `vt::Terminal` + `vt::RenderState`.
3. Feed PTY output into `vt::Terminal::write`.
4. Replace Linux key and mouse forwarding with `vt::KeyEncoder` and
   `vt::MouseEncoder`.
5. Replace the Linux shell lifecycle currently hidden inside full `libghostty`
   with `unix_pty.rs` + `linux_vt_backend.rs`.
6. Render Linux panes directly in Iced instead of round-tripping through PNGs.
7. Port copy-mode, search, selection, and scrollback to the render-state model.

## Constraint

Feature parity with the macOS backend is still the goal, but it will require a
proper Linux renderer in Boo. The code added in this change is the ABI and
linker foundation for that work, not the final Linux backend.
