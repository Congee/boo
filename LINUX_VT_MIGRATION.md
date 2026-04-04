# Linux `libghostty-vt` Migration

This migration is complete.

## Result

- Boo no longer depends on the historical `ghostty/` submodule.
- Linux and macOS both run on the shared `libghostty-vt` runtime model.
- Boo owns pane lifecycle, PTYs, snapshots, rendering, shell integration, and
  command-state tracking directly.
- The old Linux EGL readback experiment and the old macOS embedded Ghostty path
  are both gone from the active runtime.

## Remaining Linux Work

1. Improve renderer fidelity and performance further.
2. Expand terminal-heavy regression coverage.
3. Continue tightening the app-owned VT wrapper surface as upstream crates gain
   safer APIs.
