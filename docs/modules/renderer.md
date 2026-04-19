# Module Group: Renderer

Primary files:

- `src/vt_terminal_canvas.rs`
- `src/client_gui.rs`
- `src/main.rs`

## Role

boo renders terminal snapshots through its own app-owned rendering path rather
than embedding a full terminal widget as the product shell.

## Responsibilities

- draw VT snapshots into the GUI
- react to snapshot and delta changes efficiently
- keep app chrome and terminal rendering aligned
- support the shared runtime model on both macOS and Linux

## Important Context

Renderer work should be evaluated against real workloads, not toy cases. The
repo already has profiling and benchmark guidance for this.

Related docs:

- [../development/profiling.md](../development/profiling.md)
- [vt-backend-core.md](./vt-backend-core.md)
