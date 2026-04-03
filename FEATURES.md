# boo — Feature List & Architecture (macOS)

boo is a Rust/iced terminal multiplexer built on top of libghostty. It provides tmux-like window management, copy mode, and session persistence while delegating all terminal rendering to ghostty's Metal backend.

## Features

### Tab & Pane Management
- Multiple tabs, each with an independent binary split tree of panes
- 4-directional splits (up/down/left/right) with configurable ratios
- Vim-style focus navigation between panes (h/j/k/l)
- Pane resize in any direction with configurable step size
- Per-tab layout modes: even-horizontal, even-vertical, main-horizontal, main-vertical, tiled, manual
- Automatic relayout when panes are created, closed, or resized

### Copy Mode
- Vim-like modal text selection activated via keybind
- Three selection modes: character, line, rectangle
- Navigation:
  - Character: h/j/k/l
  - Word: w/b/W/B
  - Line position: 0/$/^ (start, end, first non-blank)
  - Screen position: H/M/L (top, middle, bottom of viewport)
  - Scrollback: g/G (top/bottom of history), Home/End
  - Page: Ctrl+B/F (full page), Ctrl+D/U (half page)
- Copy selection to system clipboard on yank

### Command Prompt
- tmux-style `:` command prompt overlay
- Runtime action execution (set config values, dispatch commands)
- Suggestion display for available commands

### Session Manager
- Declarative session layouts in `~/.config/boo/sessions/<name>.boo`
- Launch with `boo --session <name>`
- Per-pane shell command and working directory
- Split specifications with direction and ratio
- Save current session layout (captures actual split tree state)
- Layout types: even-horizontal, even-vertical, main-horizontal, main-vertical, tiled

### Keybindings
- tmux-style configurable prefix key (e.g. `prefix-key = ctrl+s`)
- Prefix-mode bindings (press prefix, then key)
- Direct bindings (no prefix required)
- Full modifier support: ctrl, alt/option, shift, super/cmd
- Available actions:
  - `new_split:{right,down,left,up}`
  - `goto_split:{up,down,left,right}`
  - `resize_split:{direction}:{amount}`
  - `new_tab`, `next_tab`, `prev_tab`, `close_tab`, `goto_tab:{n}`
  - `close_surface`, `search`, `enter_copy_mode`, `reload_config`

### Configuration
- Single config file: `~/.config/boo/config.boo` (respects `XDG_CONFIG_HOME`)
- Key=value format with `#` comments
- Boo-specific keys: `prefix-key`, `control-socket`, `keybind`
- Ghostty-native keys passed through to libghostty (font-family, font-size, background-opacity, etc.)
- Runtime config reload via `reload_config` action

### Scrollback & Scrolling
- Full scrollback history access via copy mode
- Page and half-page scrolling
- Smooth scroll event handling via macOS NSEvent monitor

### Clipboard
- System clipboard read/write (Cmd+C/V)
- Selection clipboard support
- Copy URL under cursor
- Copy terminal title
- Bidirectional sync with macOS pasteboard

### UI
- Native macOS window (no title bar decorations — iced provides chrome)
- Tab bar with active tab indicator
- Overlay scrollbar track (6pt width)
- Status bar (20pt height)
- Search overlay for in-terminal find
- Background transparency support

### Control Socket
- Unix domain socket IPC at configurable path (default `/tmp/boo.sock`)
- External programs can send actions to a running boo instance

---

## Architecture

### Overview

```
┌─────────────────────────────────────────────┐
│  iced application (Rust)                    │
│  ┌──────────┐ ┌──────────┐ ┌─────────────┐ │
│  │ tab bar  │ │ status   │ │ overlays    │ │
│  │ widget   │ │ bar      │ │ (search,    │ │
│  └──────────┘ └──────────┘ │  cmd prompt)│ │
│                             └─────────────┘ │
│  ┌─────────────────────────────────────┐    │
│  │  split tree (binary)               │    │
│  │  ┌─────────┐  ┌─────────┐         │    │
│  │  │ NSView  │  │ NSView  │  ...    │    │
│  │  │ (pane)  │  │ (pane)  │         │    │
│  │  └────┬────┘  └────┬────┘         │    │
│  └───────┼─────────────┼──────────────┘    │
└──────────┼─────────────┼───────────────────┘
           │             │
     ┌─────▼─────────────▼─────┐
     │  libghostty (C API)     │
     │  Metal + IOSurface      │
     │  VT parsing, rendering  │
     └─────────────────────────┘
```

### Rendering Pipeline

1. **libghostty** owns all terminal rendering — VT parsing, text shaping, Metal GPU calls
2. Each pane gets a native **NSView** created via libghostty's apprt API
3. ghostty renders into an **IOSurface** framebuffer attached to a **CALayer**
4. The NSView is positioned as a child of the iced window's content view
5. **iced** renders window chrome (tab bar, status bar, overlays) via wgpu
6. Background transparency is achieved by configuring both iced (transparent window) and ghostty (background-opacity)

### Event Flow

```
User input (key/mouse/scroll)
  → iced event handler
  → bindings.rs: check prefix mode, match keybind
  ├─ Consumed by boo → dispatch Action (NewSplit, GotoTab, etc.)
  │   → update split tree / tab state
  │   → relayout pane geometry
  │   → reposition NSViews
  └─ Forwarded to ghostty → ghostty_surface_key() / mouse FFI
      → ghostty processes internally
      → action callback fires (e.g. SET_TITLE, CLOSE_SURFACE)
      → boo handles callback → update state → relayout
```

### Module Map

| Module | Role |
|--------|------|
| `main.rs` | iced application shell, message loop, view rendering, layout engine |
| `tabs.rs` | Tab collection, per-tab pane tree, focus tracking |
| `splits.rs` | Binary split tree, geometry computation, relayout algorithm |
| `bindings.rs` | Keybind parser, prefix-mode FSM, action enum, copy mode dispatcher |
| `config.rs` | Config file parser, default values, ghostty passthrough |
| `session.rs` | Session file parser, layout templates, save/load |
| `appkit.rs` | macOS NSView lifecycle, CALayer setup, IOSurface, scroll monitor |
| `ffi.rs` | Hand-written libghostty C FFI (~30 functions) |
| `control.rs` | Unix domain socket IPC server |
| `keymap.rs` | Physical key code → logical key mapping |
| `tmux.rs` | tmux compatibility layer |

### Build System

```
nix develop                    # provides zig, rust, apple-sdk
  → cargo build
    → build.rs
      → zig build (ghostty submodule → libghostty.a / xcframework)
      → link: Cocoa, Metal, QuartzCore, IOSurface, CoreGraphics,
              CoreText, Foundation
    → rustc compiles src/*.rs against linked libghostty
```

### Dependencies

| Crate | Purpose |
|-------|---------|
| `iced` 0.14 (wgpu, tokio) | Window, widgets, event loop |
| `objc2` + app-kit/quartz-core/foundation | Type-safe Objective-C interop |
| `raw-window-handle` 0.6 | Window handle for NSView embedding |
| `anyhow` | Error handling |
| `serde` + `serde_json` | Serialization (session save/load) |
| `libc` | POSIX primitives (socket, signals) |

### Key Design Decisions

- **No bindgen** — FFI is hand-written for the ~30 C functions used, keeping the build simple
- **Wrapper, not fork** — ghostty is an unmodified git submodule (ghostty-org/ghostty)
- **macOS first** — Linux support deferred (needs upstream ghostty contribution for GTK-less apprt)
- **iced for chrome only** — terminal rendering is entirely ghostty's domain; iced handles tab bar, status bar, and overlays
- **Binary split tree** — each tab's pane layout is a binary tree, supporting arbitrary nesting
