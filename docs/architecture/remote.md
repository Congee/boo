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

- tab listing
- attach/detach
- create/resize/input/destroy
- full state and deltas
- auth and heartbeat
- iOS client consumption

Primary code:

- `src/remote.rs`
- `src/remote_*`
- `ios/Sources/*`

See [../modules/remote-daemon.md](../modules/remote-daemon.md).
