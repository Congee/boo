# boo — Feature List & Architecture

boo is a Rust/iced terminal multiplexer built around a shared `libghostty-vt`
runtime on both macOS and Linux. It provides tmux-like window management, copy
mode, command-state tracking, and session persistence while Boo owns the app
chrome, layout, and VT rendering path.

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
- Boo-specific keys: `prefix-key`, `control-socket`, `remote-port`, `remote-auth-key`, `keybind`
- Shared terminal/UI keys: `font-family`, `font-size`, `background-opacity`, `background-opacity-cells`
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
- VT-backed tab bar can show a running-command spinner when shell integration emits `OSC 133`
- macOS can send command-finished notifications to Notification Center
- Overlay scrollbar track (6pt width)
- Status bar (20pt height)
- Search overlay for in-terminal find
- Background transparency support
- Remaining UI task: configurable cursor shape and optional blinking cursor

### Control Socket
- Unix domain socket IPC at configurable path (default `/tmp/boo.sock`)
- External programs can send actions to a running boo instance

### Session Server
- `boo server` runs the long-lived session owner without a GUI
- `boo` auto-connects to the local server and auto-starts it when needed
- `boo ls`, `boo new-session`, and `boo kill-server` operate against the local server
- Live sessions persist when the GUI client exits because PTYs and tabs belong to the server process

### Headless Mode
- `boo --headless` runs the shared VT backend and control socket without creating a GUI window
- `boo --headless --socket /path/to.sock` overrides the control-socket path at startup
- `boo --headless --remote-port 7337` starts the Boo TCP remote daemon alongside the headless runtime
- Headless mode exposes the same snapshot/query/control surface as the GUI app
- Tabs, splits, sessions, PTYs, and terminal snapshots stay on the same runtime path as the GUI build

### Remote Daemon
- Optional Boo-native TCP daemon for the iOS remote viewer and other LAN clients
- Bonjour advertisement on `_boo._tcp`
- Uses the Boo GSP-compatible framing already consumed by the iOS client
- Supports session listing, attach/detach, create, resize, destroy, text input, and full terminal-state publishing
- Can require HMAC-SHA256 challenge/response auth when `remote-auth-key` is configured
- Remaining architecture task: revisit whether PTY ingest should become fully event-driven instead of loop/poll driven once the current profiling work stabilizes
- Remaining architecture task: move process exit/reap handling to an explicit PTY EOF/worker event path so the hot PTY ingest loop never probes for child exit status

### iOS Remote Viewer
- A native SwiftUI iOS app lives under [`ios/`](/Users/example/dev/boo/ios)
- Bundle identifier: `me.congee.boo`
- Connects to a compatible remote daemon using the existing GSP wire protocol
- Browses `_boo._tcp` Bonjour services
- Includes manual host/auth-key entry, saved nodes, connection history, session listing, and a VT cell-grid viewer
- Uses iOS local-network permissions via `NSLocalNetworkUsageDescription` and `NSBonjourServices`
- Automated validation lives in [`scripts/test-ios-remote-view.sh`](/Users/example/dev/boo/scripts/test-ios-remote-view.sh) and covers discovery, auth, session listing, attach, resize, and terminal-state updates against a live Boo daemon
- Remaining manual validation: simulator/device pass for touch UI, local-network permission prompts, and real attach/resize behavior from the rendered app

---

## Architecture

### Overview

```text
┌──────────────────────────────────────────────┐
│ iced application (Rust)                      │
│ ┌──────────┐ ┌──────────┐ ┌───────────────┐  │
│ │ tab bar  │ │ status   │ │ overlays      │  │
│ │ widget   │ │ bar      │ │ search/prompt │  │
│ └──────────┘ └──────────┘ └───────────────┘  │
│ ┌──────────────────────────────────────────┐ │
│ │ tabs + binary split tree                │ │
│ │  ┌──────────────┐  ┌──────────────┐     │ │
│ │  │ platform view│  │ platform view│ ... │ │
│ │  │ + VT canvas  │  │ + VT canvas  │     │ │
│ │  └──────┬───────┘  └──────┬───────┘     │ │
│ └─────────┼──────────────────┼────────────┘ │
└───────────┼──────────────────┼──────────────┘
            │                  │
      ┌─────▼──────────────────▼─────┐
      │ shared VT backend core       │
      │ PTY, scrollback, OSC 133,    │
      │ snapshots, input encoding    │
      └──────────────┬───────────────┘
                     │
               ┌─────▼─────┐
               │libghostty-│
               │vt         │
               └───────────┘
```

### Rendering Pipeline

1. Boo owns pane layout, focus, tabs, overlays, and copy mode.
2. Each pane gets a native host view positioned inside the iced window.
3. The shared VT backend core manages the PTY, `libghostty-vt`, scrollback, and snapshots.
4. Boo renders VT snapshots through the shared terminal canvas.
5. Background transparency comes from the VT canvas alpha plus a transparent top-level iced window.

### Event Flow

```
User input (key/mouse/scroll)
  → iced event handler and AppKit text-input bridge
  → bindings.rs: check prefix mode, match keybind
  ├─ Consumed by boo → dispatch Action (NewSplit, GotoTab, etc.)
  │   → update split tree / tab state
  │   → relayout pane geometry
  │   → resize platform host views
  └─ Forwarded to shared VT backend
      → key / mouse encoding
      → PTY write
      → libghostty-vt updates terminal state
      → boo polls snapshots / OSC state / command lifecycle
```

### Module Map

| Module | Role |
|--------|------|
| `main.rs` | iced application shell, message loop, view rendering, layout engine |
| `tabs.rs` | Tab collection, per-tab pane tree, focus tracking |
| `splits.rs` | Binary split tree, geometry computation, relayout algorithm |
| `bindings.rs` | Keybind parser, prefix-mode FSM, action enum, copy mode dispatcher |
| `config.rs` | Config file parser, default values, appearance and notification policy |
| `session.rs` | Session file parser, layout templates, save/load |
| `vt_backend_core.rs` | Shared VT pane runtime, snapshots, OSC 133 command state |
| `macos_vt_backend.rs` | macOS VT backend and host-view integration |
| `backend.rs` | Platform backend selection and shared backend façade |
| `vt_terminal_canvas.rs` | Shared snapshot renderer for VT panes |
| `platform/macos.rs` | macOS AppKit host view, text input, clipboard, notifications |
| `ffi.rs` | Hand-written ABI-compatible structs/enums kept at the app boundary |
| `control.rs` | Unix domain socket IPC server |
| `keymap.rs` | Physical key code → logical key mapping |
| `tmux.rs` | tmux compatibility layer |

### Build System

```text
nix develop
  → cargo build
    → build.rs
      → link platform frameworks and libghostty-vt runtime
    → rustc compiles src/*.rs against Boo's shared VT backend
```

### Dependencies

| Crate | Purpose |
|-------|---------|
| `iced` 0.14 (wgpu, tokio) | Window, widgets, event loop |
| `objc2` + app-kit/quartz-core/foundation | Type-safe Objective-C interop |
| `libghostty-vt` | Shared terminal runtime |
| `anyhow` | Error handling |
| `serde` + `serde_json` | Serialization (session save/load) |
| `libc` | POSIX primitives (socket, signals) |

### Key Design Decisions

- **Shared VT core** — macOS and Linux now share the same terminal runtime model
- **No bindgen** — the remaining FFI is hand-written and layout-tested
- **iced owns the app shell** — terminal rendering, overlays, and pane chrome live in Boo
- **Binary split tree** — each tab's pane layout is a binary tree, supporting arbitrary nesting

## Planned Work

### Shell Integration
- Boo shell integration now ships for bash, zsh, and fish in [shell-integration/README.md](/Users/example/dev/boo/shell-integration/README.md)
- These scripts emit `OSC 133` prompt markers and `cmdline_url` command metadata for the tab spinner/title path
- `boo completions bash|zsh|fish` prints shell completions for the server/client lifecycle commands
- Remaining integration task: migrate `~/.config/kitty/watcher.py` behavior into `~/.config/boo/config.boo`
- Kitty config migration backlog:
  - [x] support Kitty-style direct punctuation bindings needed by the config, such as `ctrl+super+,`
  - [ ] migrate supported keybinds from `~/.config/kitty/kitty.conf` into `~/.config/boo/config.boo`
  - [x] expose a bindable copy-to-clipboard action for the current Boo selection
  - [ ] implement configurable macOS option-as-alt behavior
  - [x] implement `set_tab_title`
  - [ ] implement Boo-native search-pane action to replace the Kitty `search.py` split binding
  - [ ] implement configurable tab bar style/separator/alignment/title template
  - [ ] implement `window-decoration` config instead of ignoring it
  - [ ] decide how to map Kitty’s `close_on_child_death` / `macos_quit_when_last_window_closed` semantics onto Boo’s client/server model
  - [ ] audit Kitty theme/include settings and decide which visual settings Boo should support directly

### tmux Parity Backlog
- [x] support `set-tab-title`
- [x] expose a bindable `copy` action for the current Boo selection
- [ ] add pane lifecycle commands:
  - [x] `break-pane`
  - [x] `join-pane` / `move-pane` using a marked pane
  - [x] `swap-pane`
  - [x] `rotate-window`
- [ ] add live preset layout commands:
  - [x] `select-layout`
  - [x] `next-layout`
  - [x] `previous-layout`
  - [x] rebalance/spread panes evenly
- [ ] add interactive tmux-style chooser UIs:
  - [x] display-panes
  - [x] choose-tree
  - [x] choose-buffer
- [x] add paste buffer management beyond the system clipboard
- [ ] add `find-window` style search across titles, names, and visible content
- [ ] add remain-on-exit / respawn-pane style process lifecycle controls
- [ ] add marked-pane targeting for pane move/swap/join commands
- [ ] add session/window rename and move/link semantics closer to tmux

### Command Finish Notifications
- `notify-on-command-finish` config and duration thresholds are implemented
- macOS notifications use `UNUserNotificationCenter`, with an AppleScript fallback if the native path is unavailable
- Linux notifications are delivered through the desktop notification client path (`notify-send`) with the same Boo policy controls
