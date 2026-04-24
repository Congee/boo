# VT Wrapper Migration Plan

## Summary

The current no-upstream-`Terminal` migration checklist is complete. Boo now
keeps raw `libghostty-vt` ABI types inside `src/vt.rs` for the current internal
snapshot/render path, while renderer and remote-state internals use Boo-owned
typed wrappers.

The remaining VT migration work is a future upstream-adoption plan, not an
active TODO for the current checklist. It should only proceed when the upstream
wrapper can preserve Boo's current formatter, hyperlink, input, and snapshot
behavior.

## Completed Current Checklist

- Keep `src/vt.rs` as Boo's explicit facade over `libghostty-vt-sys`.
- Mirror upstream-style handle ownership and scoped render-snapshot borrowing.
- Mirror typed errors, focus events, paste safety, key/mouse encoding, and
  caller-owned/reused output buffers.
- Use Boo-owned `RgbColor`, `RenderColors`, and `CursorStyle` in internal
  terminal snapshots and renderer code.
- Preserve remote/UI wire shapes: `[u8; 3]` colors and raw integer cursor style
  remain only at serialization/UI boundaries.
- Do not adopt upstream `Terminal` in this pass.

## Future Upstream-Adoption Plan

1. **Close the formatter hyperlink gap first**
   - Upstream `FormatterOptions` must expose the screen hyperlink option Boo
     currently uses through `GhosttyFormatterScreenExtra { hyperlink: true }`.
   - Either upstream that option or carry the smallest possible local wrapper
     addition before attempting full `Terminal` adoption.

2. **Adopt upstream `Terminal` only as one coordinated slice**
   - Migrate terminal lifecycle, resize, VT writes, PTY write callback,
     render-state refresh, key encoders, and mouse encoders together.
   - Do not migrate only `Terminal::new`/`resize` first, because upstream key
     and mouse encoder options depend on the upstream `Terminal` type and Boo's
     current raw terminal handle would no longer be available.

3. **Replace PTY callback userdata with closure ownership**
   - Replace Boo's current `set_userdata` plus `set_write_pty` path with
     upstream `Terminal::on_pty_write`.
   - Capture only the PTY fd or a minimal writer handle in the closure.
   - Keep the callback lifetime owned by the terminal wrapper.

4. **Swap facade internals without churning app-facing shapes**
   - Keep Boo-facing APIs in `src/vt.rs` stable where possible.
   - Preserve `VtPane`, `TerminalSnapshot`, remote wire payloads, and UI
     snapshot shapes unless a separate protocol/UI migration explicitly needs
     them to change.
   - Treat `src/vt.rs` as the compatibility layer between upstream wrapper
     churn and Boo's renderer/runtime code.

## Test Plan

- `nix develop --command cargo check`
- `nix develop --command cargo clippy`
- `nix develop --command cargo test -- --test-threads=1`
- `bash scripts/test-headless.sh`
- `bash scripts/test-ui-snapshot.sh`
- `bash scripts/test-latency-traces.sh`
- For any `Terminal` adoption slice, additionally run the runtime-view metrics
  scenario before and after the change and compare terminal rendering, input,
  hyperlink, key, and mouse behavior.

## Assumptions

- Raw ABI exposure outside `src/vt.rs` is not an active goal unless it affects
  current renderer/runtime clarity or safety.
- Upstream `Terminal` adoption is deferred until it can be done without losing
  hyperlink lookup support.
- Broad `cargo fmt`/`rustfmt` should not be run as part of this migration plan.
