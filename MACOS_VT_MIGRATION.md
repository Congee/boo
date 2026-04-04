# macOS `libghostty-vt` Migration

Status: complete.

boo now uses the shared `libghostty-vt` runtime on both macOS and Linux. The
old embedded macOS `libghostty` surface path is no longer part of the active
architecture.

## Final Architecture

### Shared VT core

Owned by Boo and shared across macOS and Linux:

- PTY lifecycle
- `libghostty-vt` terminal ownership
- VT snapshots for UI rendering
- OSC 133 command-state tracking
- title, cwd, and scrollbar extraction
- selection reads and clipboard-facing text extraction
- key and mouse encoding for VT panes

Primary files:

- [src/vt_backend_core.rs](/Users/example/dev/boo/src/vt_backend_core.rs)
- [src/vt.rs](/Users/example/dev/boo/src/vt.rs)
- [src/unix_pty.rs](/Users/example/dev/boo/src/unix_pty.rs)
- [src/backend.rs](/Users/example/dev/boo/src/backend.rs)

### macOS host layer

Still platform-specific:

- native host views and focus
- AppKit text input / preedit / IME hooks
- clipboard integration
- desktop notifications
- macOS event monitoring and scroll delivery

Primary files:

- [src/macos_vt_backend.rs](/Users/example/dev/boo/src/macos_vt_backend.rs)
- [src/platform/macos.rs](/Users/example/dev/boo/src/platform/macos.rs)
- [src/keymap.rs](/Users/example/dev/boo/src/keymap.rs)

### Shared app layer

Still shared and backend-agnostic in responsibility:

- tabs and splits
- command prompt and copy mode
- control socket
- shared VT rendering
- notification policy and command-state UI

Primary files:

- [src/main.rs](/Users/example/dev/boo/src/main.rs)
- [src/tabs.rs](/Users/example/dev/boo/src/tabs.rs)
- [src/splits.rs](/Users/example/dev/boo/src/splits.rs)
- [src/control.rs](/Users/example/dev/boo/src/control.rs)
- [src/vt_terminal_canvas.rs](/Users/example/dev/boo/src/vt_terminal_canvas.rs)

## What Landed

- Extracted a shared VT runtime core out of the old Linux-only path
- Added a real `MacVtBackend`
- Moved macOS rendering onto the shared VT canvas path
- Replaced macOS pane focus and IME cursor ownership with pane-based VT state
- Added an AppKit text-input bridge for committed text and preedit state
- Routed macOS VT keyboard, mouse, scroll, clipboard, and command-finish paths
  through the shared backend model
- Removed the embedded `surface_backend` path from the active architecture
- Made macOS use the VT backend by default

## Regression Coverage

The migration is covered by:

- `cargo test`
- `bash scripts/test-ui-snapshot.sh`
- `bash scripts/test-ui-scenarios.sh`

Recent migration-specific coverage includes:

- command-finish notification policy tests
- preedit/text-input state tests
- FFI layout tests
- shared VT rendering tests
- control-path backspace regression via the UI scenario harness

## Remaining Hardening

The architectural migration is done. What remains is product hardening:

- manual validation with real macOS IMEs and dead keys
- UX tuning for notifications and tab-title command display
- broader shell integration so more shells emit `OSC 133` consistently
