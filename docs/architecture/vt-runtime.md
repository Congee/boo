# Shared VT Runtime

boo uses one shared `libghostty-vt`-based runtime model on macOS and Linux.

Primary files:

- `src/vt.rs`
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

## `libghostty-vt` Wrapper Layer

Current layering is:

```text
src/vt.rs Boo-specific wrapper facade
  -> libghostty-vt-sys raw bindings and link/build metadata
    -> native libghostty-vt from Nix or the sys-crate fallback
```

`src/vt.rs` exists as a small Boo-shaped API over the C ABI. It predates the
current shared macOS/Linux runtime and now mainly protects the rest of the app
from upstream crate layout churn while preserving Boo-specific snapshot,
renderer, input, and PTY callback shapes.

The upstream `libghostty-vt` crate does provide safer Rust wrappers, but it is
not yet a drop-in replacement for the whole Boo facade:

| Slice | Upstream replacement status | Notes |
| --- | --- | --- |
| Terminal lifecycle and VT writes | Feasible, but not isolated | Upstream `Terminal` supports `new`, `resize`, `vt_write`, title/pwd, scrollbar, and closure-based `on_pty_write`. Migrating it first would force render/key/mouse/formatter changes because upstream does not expose the raw terminal handle. |
| PTY write callback | Feasible with terminal migration | Boo can replace `set_userdata` plus `set_write_pty` with `Terminal::on_pty_write` capturing the PTY fd, removing the current userdata pointer lifetime concern. |
| Render state / rows / cells | Feasible with API rewrite | Upstream uses a borrowed `Snapshot` from `RenderState::update`, plus row/cell lending iterators and typed colors/styles. Boo's current refresh code reads by tag from `RenderState` after update, so this needs a focused rewrite of `VtPane::refresh_snapshot` and snapshot tests. |
| Key encoder / key event | Feasible after terminal migration | Upstream encoders are safer, but `set_options_from_terminal` requires the upstream `Terminal`; it cannot be adopted cleanly while Boo's `Terminal` is still raw. |
| Mouse encoder / mouse event | Feasible after terminal migration | Same dependency on upstream `Terminal` for `set_options_from_terminal`; Boo also needs adapters from existing raw runtime action values to upstream typed enums. |
| Formatter / hyperlink lookup | Blocked upstream today | Boo uses `GhosttyFormatterScreenExtra { hyperlink: true }` to recover OSC 8 links at a grid position. Upstream `FormatterOptions` does not expose the screen hyperlink option, so moving `Terminal` fully upstream would currently regress `hyperlink_at`. |
| Raw color/style/constants used by UI and remote state | Keep for now | Remote serialization, tests, and renderer code still use raw `GhosttyColorRgb`, `GhosttyRenderStateColors`, cursor-style constants, and style fields. These should migrate only after the snapshot layer owns typed conversions. |

Recommended migration order:

1. Keep `src/vt.rs` as the explicit Boo facade while we are not adopting the
   upstream wrapper immediately; it depends on `libghostty-vt-sys` directly.
2. Ask upstream or carry a very small wrapper improvement for formatter
   hyperlink options. Without this, full `Terminal` adoption loses
   `hyperlink_at`.
3. Migrate terminal lifecycle plus `on_pty_write`, render snapshot refresh,
   and key/mouse encoders as one coordinated slice because their safe wrapper
   APIs share the upstream `Terminal` type.
4. After snapshots are typed, shrink the raw aliases/constants exported from
   `src/vt.rs` and convert renderer/remote state to Boo-owned types where it
   improves clarity.

See also:

- [../modules/vt-backend-core.md](../modules/vt-backend-core.md)
- [../architecture/platform-linux.md](./platform-linux.md)
- [../architecture/platform-macos.md](./platform-macos.md)
