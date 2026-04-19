# Module: `vt_backend_core`

Primary file:

- `src/vt_backend_core.rs`

## Why This Module Matters

This is one of the highest-value modules in boo. It owns the shared VT runtime
behavior that both macOS and Linux depend on.

## Responsibilities

- pane runtime ownership
- PTY read/write coordination
- snapshot refresh and caching
- command-state tracking via shell integration
- renderer-facing terminal state

## Adjacent Modules

- `src/unix_pty.rs`
- `src/vt_terminal_canvas.rs`
- `src/backend.rs`
- `src/runtime_*`

## Change Risks

Changes here can affect:

- input latency
- redraw cadence
- scrollback correctness
- command-state UI behavior
- both macOS and Linux at once

When changing this module, pair code changes with targeted profiling and the
relevant UI/headless verification scripts.
