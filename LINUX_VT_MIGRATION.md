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
- Linux runtime now uses the VT backend only; the old EGL readback path has
  been removed from the app flow.
- Linux now depends on the published `libghostty-vt` crate; the vendored
  `ghostty/` submodule remains for the macOS surface backend.
- [src/vt.rs](/home/example/dev/boo/src/vt.rs) provides Boo's Linux-facing wrapper for:
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
- [src/control.rs](/home/example/dev/boo/src/control.rs) and the Wayland scripts now
  provide an app-owned snapshot-based UI testing surface for Linux.
- The control socket can drive raw terminal input with `send-text`, so
  integration tests can assert on visible terminal content instead of relying
  on one-key-at-a-time injection.
- Linux terminal rendering now goes through Boo's canvas-based VT renderer
  instead of the old `rich_text` fallback.

## Next implementation steps

1. Improve Linux renderer fidelity and performance further:
   glyph caching, wide/combining glyph handling, and visual polish.
2. Expand integration coverage for terminal-heavy workflows and edge cases.
3. Switch Linux VT integration fully over to the external crate surface as
   that crate grows safer/higher-level APIs, while keeping Boo's wrapper API
   stable.

## Constraint

Feature parity with the macOS backend remains the goal. The Linux backend is
now usable, testable, and VT-only; remaining work is mostly polish and
maintainability rather than the original embedding bring-up.
