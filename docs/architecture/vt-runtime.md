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

The facade intentionally mirrors the useful parts of upstream's safe wrapper
style where that can be done without adopting the upstream `Terminal` type yet:
owned native handles are kept non-null, `RenderState::update` returns a scoped
`RenderSnapshot`, reusable `RowIterator`/`CellIterator` handles create borrowed
`RowIteration`/`CellIteration` views from that snapshot, and common cursor,
style, key, and mouse inputs have Boo-owned typed adapters. Raw ABI types are
still exported where remote serialization, rendering, or platform glue
currently require them.

The upstream `libghostty-vt` crate does provide safer Rust wrappers, but it is
not yet a drop-in replacement for the whole Boo facade:

| Slice | Upstream replacement status | Notes |
| --- | --- | --- |
| Terminal lifecycle and VT writes | Feasible, but not isolated | Upstream `Terminal` supports `new`, `resize`, `vt_write`, title/pwd, scrollbar, and closure-based `on_pty_write`. Migrating it first would force render/key/mouse/formatter changes because upstream does not expose the raw terminal handle. |
| PTY write callback | Feasible with terminal migration | Boo can replace `set_userdata` plus `set_write_pty` with `Terminal::on_pty_write` capturing the PTY fd, removing the current userdata pointer lifetime concern. |
| Render state / rows / cells | Facade aligned; upstream replacement still needs API swap | Boo now consumes a scoped `RenderSnapshot` from `RenderState::update`, plus reusable row/cell iterator handles that lend `RowIteration`/`CellIteration` views tied to the snapshot, and typed cursor/style adapters. The remaining upstream swap is mostly about replacing the facade internals, not every snapshot caller at once. |
| Key encoder / key event | Facade adapters added; upstream feasible after terminal migration | Boo accepts typed key actions with raw compatibility, but upstream encoders still require the upstream `Terminal` for `set_options_from_terminal`; they cannot be adopted cleanly while Boo's `Terminal` is still raw. |
| Mouse encoder / mouse event | Facade adapters added; upstream feasible after terminal migration | Boo accepts typed mouse action/button/geometry adapters with raw compatibility. Upstream mouse encoders have the same upstream-`Terminal` dependency for `set_options_from_terminal`. |
| Formatter / hyperlink lookup | Blocked upstream today | Boo uses `GhosttyFormatterScreenExtra { hyperlink: true }` to recover OSC 8 links at a grid position. Upstream `FormatterOptions` does not expose the screen hyperlink option, so moving `Terminal` fully upstream would currently regress `hyperlink_at`. |
| Raw color/style/constants used by UI and remote state | Shrinking gradually | `RenderCursor` and `CellStyle` now own common conversions, but remote serialization, tests, and renderer code still use raw `GhosttyColorRgb`, `GhosttyRenderStateColors`, and cursor-style constants where that preserves current wire/UI shapes. |

Recommended migration order:

1. Keep `src/vt.rs` as the explicit Boo facade while we are not adopting the
   upstream wrapper immediately; it depends on `libghostty-vt-sys` directly and
   should keep concentrating handle ownership, render snapshot lifetimes, and
   typed conversions there.
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
