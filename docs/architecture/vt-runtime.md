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
style, key, mouse, focus, paste, and build-info APIs have Boo-owned typed
adapters. The facade also maps `GhosttyResult` into typed Rust errors and
supports caller-owned/reused output buffers for key, mouse, focus, and formatter
encoding paths. Raw ABI types are still exported where remote serialization,
rendering, or platform glue currently require them.

The upstream `libghostty-vt` crate does provide safer Rust wrappers, but it is
not yet a drop-in replacement for the whole Boo facade:

| Slice | Upstream replacement status | Notes |
| --- | --- | --- |
| Terminal lifecycle and VT writes | Feasible, but not isolated | Upstream `Terminal` supports `new`, `resize`, `vt_write`, title/pwd, scrollbar, and closure-based `on_pty_write`. Migrating it first would force render/key/mouse/formatter changes because upstream does not expose the raw terminal handle. |
| PTY write callback | Feasible with terminal migration | Boo can replace `set_userdata` plus `set_write_pty` with `Terminal::on_pty_write` capturing the PTY fd, removing the current userdata pointer lifetime concern. |
| Render state / rows / cells | Facade aligned; upstream replacement still needs API swap | Boo now consumes a scoped `RenderSnapshot` from `RenderState::update`, plus reusable row/cell iterator handles that lend `RowIteration`/`CellIteration` views tied to the snapshot, and typed cursor/style adapters. The remaining upstream swap is mostly about replacing the facade internals, not every snapshot caller at once. |
| Key encoder / key event | Facade aligned; upstream feasible after terminal migration | Boo accepts typed key actions/modifiers and offers fixed-buffer plus reusable-`Vec` encoding. `VtPane` reuses one key event and encode buffer per pane. Upstream encoders still require the upstream `Terminal` for `set_options_from_terminal`; they cannot be adopted cleanly while Boo's `Terminal` is still raw. |
| Mouse encoder / mouse event | Facade aligned; upstream feasible after terminal migration | Boo accepts typed mouse action/button/geometry/format/tracking adapters and offers fixed-buffer plus reusable-`Vec` encoding. `VtPane` reuses one mouse event and encode buffer per pane. Upstream mouse encoders have the same upstream-`Terminal` dependency for `set_options_from_terminal`. |
| Formatter / hyperlink lookup | Partly aligned; blocked upstream today | Boo exposes both allocated and caller-provided formatter buffers, but still uses `GhosttyFormatterScreenExtra { hyperlink: true }` to recover OSC 8 links at a grid position. Upstream `FormatterOptions` does not expose the screen hyperlink option, so moving `Terminal` fully upstream would currently regress `hyperlink_at`. |
| Focus, paste, build info, and errors | Facade aligned | Boo mirrors upstream's typed focus events, paste safety helper, build-info helpers, and `OutOfMemory`/`InvalidValue`/`OutOfSpace { required }` error mapping directly in `src/vt.rs`. |
| Raw color/style/constants used by UI and remote state | Done for current Boo internals | Internal VT snapshots and renderer code now use Boo-owned `RgbColor`, `RenderColors`, and `CursorStyle`. Remote wire payloads and UI snapshots still keep `[u8; 3]` colors and integer cursor style fields where that preserves current protocol/UI shapes. Raw ABI types are intentionally confined to the `src/vt.rs` facade. |

Current boundary and future upstream-adoption notes:

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
4. The current no-`Terminal`-adoption checklist is complete: snapshot internals,
   renderer code, and remote-state internals use Boo-owned render types, while
   remote/UI boundaries preserve their existing primitive shapes.

See also:

- [../modules/vt-backend-core.md](../modules/vt-backend-core.md)
- [../architecture/platform-linux.md](./platform-linux.md)
- [../architecture/platform-macos.md](./platform-macos.md)
