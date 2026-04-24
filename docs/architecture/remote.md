# Remote Architecture

boo currently has two remote lanes:

1. Desktop remote over SSH
2. Boo-native TCP daemon for direct and iOS clients

These are intentionally separate in rollout, but they are expected to converge
on one canonical Boo-native remote model over time.

## Desktop SSH Lane

Desktop SSH mode uses:

- `boo --host <ssh-host>`
- remote server bootstrap over SSH
- local forwarded control socket
- local forwarded `.stream` socket

Primary code:

- `src/launch.rs`
- `src/control.rs`
- `src/client_gui.rs`

See [../remote/ssh-desktop.md](../remote/ssh-desktop.md).

## Boo-Native Remote Lane

The native daemon supports:

- runtime-state bootstrap
- server-owned runtime-view state per connected screen
- semantic runtime actions for tab, pane, split, resize, attach, and detach
- full state and deltas
- pane-scoped streaming keyed by `tab_id -> pane_id`
- auth and heartbeat
- iOS client consumption

The current live model is:

- clients are runtime viewers
- the server stays authoritative for runtime state, tabs, panes, and terminal
  content
- clients keep only thin viewer-local bookkeeping such as viewed-tab display,
  focused-pane display, viewport metadata, and stream caches
- iOS does not render its own runtime tab bar; the core statusbar remains the
  tab-list UI
- Rust and iOS emit shared runtime-view latency events so daemon handling,
  native signposts, pane updates, and render apply timing can be correlated

Remote clients do not own terminal lifecycle objects; they view and control the
server-owned runtime.

Post-v1 work should focus on scroll/search semantics across screens, baseline
latency measurement before deciding whether local prediction is needed,
focused-pane QoS under load, and host-scoped reconnect UX.

Primary code:

- `src/remote.rs`
- `src/remote_*`
- `ios/Sources/*`

See [../modules/remote-daemon.md](../modules/remote-daemon.md).
