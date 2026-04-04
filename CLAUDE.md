# boo — Rust/iced terminal app built on libghostty-vt

## Build

```bash
# Enter dev shell (provides zig + rust + apple-sdk)
nix develop

# Build
cargo build
```

## Architecture

- `libghostty-vt` is the shared terminal runtime on macOS and Linux
- iced owns window chrome, terminal rendering, overlays, and pane layout
- macOS host code handles native view focus, text input/IME, clipboard, and notifications
- Linux host code provides platform glue while sharing the same VT core

## Project structure

- `src/ffi.rs` — hand-written FFI bindings where Boo still talks to native APIs
- `src/vt_backend_core.rs` — shared VT pane/runtime core
- `src/main.rs` — iced application and shared app state
- `src/platform/macos.rs` — macOS host integration
- `build.rs` — links Boo against the vendored/native dependencies it needs

## Conventions

- No bindgen — hand-write FFI for the ~30 functions we use
- The shipped app depends on the vendored `libghostty-vt` crates, not the full Ghostty app runtime
- macOS and Linux share one VT architecture; platform code should stay thin
