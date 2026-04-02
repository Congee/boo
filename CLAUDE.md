# boo — Rust/iced UI wrapper around libghostty

## Build

```bash
# Enter dev shell (provides zig + rust + apple-sdk)
nix develop

# Build (runs zig build on ghostty submodule, then cargo)
cargo build

# Submodule init (if ghostty/ is empty)
git submodule update --init
```

## Architecture

- **Option 1: Full libghostty embedding** — ghostty owns all rendering (Metal on macOS, OpenGL on Linux future)
- iced provides window chrome (tab bar, status bar) as widgets
- Native NSView child embedded inside iced window for terminal surface
- libghostty renders into the NSView via IOSurfaceLayer

## Project structure

- `ghostty/` — git submodule (ghostty-org/ghostty, official upstream)
- `src/ffi.rs` — hand-written FFI bindings for ghostty.h C API
- `src/main.rs` — iced application with ghostty runtime callbacks
- `build.rs` — runs `zig build` on ghostty submodule, links libghostty

## Conventions

- No bindgen — hand-write FFI for the ~30 functions we use
- Official upstream ghostty as submodule — we are a wrapper, not a fork
- macOS first, Linux deferred (needs ~50 LOC upstream Zig contribution)
