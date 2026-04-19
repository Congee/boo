# Remote Boo Over SSH

Structured remote docs now live under [`docs/remote/`](./docs/remote). For the
docs-tree entrypoint, start with
[docs/remote/ssh-desktop.md](./docs/remote/ssh-desktop.md).

Status: active milestone, partially implemented.

This document tracks the current SSH-backed desktop remote design and the
remaining work for it. It is not the long-term remote product contract. That
lives in [REMOTE_REQUIREMENTS.md](./REMOTE_REQUIREMENTS.md).

## What This Document Is For

Use this file for the current desktop-remote implementation shape:

- `boo --host <ssh-host>`
- SSH bootstrap and trust boundary
- control-socket and `.stream` forwarding
- remaining desktop SSH hardening work

Do not use this file as the canonical cross-platform remote spec. The intended
long-term direction is still one Boo-native remote transport shared by desktop
and iOS, with SSH remaining a practical bootstrap path.

## Current Product Shape

The current desktop remote milestone is:

```bash
boo --host <ssh-host>
```

This should feel like connecting to "remote Boo", while using SSH underneath
to:

- ensure a compatible remote Boo server exists
- start or reuse that remote server
- forward Boo's local IPC endpoints back to the client machine
- attach the local GUI/client to those forwarded endpoints

## Why SSH Is Still The Right Milestone

Boo already has:

- a local control socket
- a local `.stream` socket for live updates
- a Boo-native TCP daemon path used by the iOS/mobile work

For desktop remote, the fastest low-risk path is still to reuse the local IPC
contract over SSH rather than rewriting the desktop GUI around the TCP daemon.

That means:

- the remote Boo server stays private on the target host
- SSH provides authentication, encryption, host trust, and tunneling
- the existing GUI/client continues talking to local-looking sockets

## Current Architecture

Desktop SSH remote uses a two-socket model:

- control socket for request/response RPCs such as ping, list, snapshots, and
  new-session
- `.stream` socket for long-lived state, deltas, and live UI/session updates

This split still makes sense because:

- control RPCs do not get blocked behind stream traffic
- stream traffic can keep its ordered push-oriented behavior
- control and stream have different reconnection and lifecycle expectations

On the wire, SSH bridges host-local sockets:

- remote Boo control socket, for example `/tmp/boo.sock`
- remote Boo stream socket, for example `/tmp/boo.sock.stream`
- local forwarded control socket, for example `/tmp/boo-<host>.sock`
- local forwarded stream socket, for example `/tmp/boo-<host>.sock.stream`

The local GUI/client connects only to the forwarded local sockets.

## Current Repo State

The repo already has first-cut SSH desktop remote support:

- `--host`
- remote config keys for host/socket/workdir/binary
- host-aware startup logic
- remote server bootstrap over SSH
- SSH forwarding for control and `.stream`
- remote-host-oriented verification scripts under [`scripts/`](./scripts)

The repo also already has the separate Boo-native transport work:

- `boo --headless --remote-port <port>`
- the SwiftUI iOS client under [`ios/`](./ios)

That split is intentional for now. Desktop SSH is the current milestone;
desktop/mobile transport convergence is later work.

## Remaining SSH Desktop Work

The main open items are product hardening, not greenfield design:

1. Expand remote path handling.
   `remote-binary`, `remote-workdir`, and related settings should expand `~`
   and `$HOME` naturally before building the SSH command.

2. Tighten tunnel lifecycle recovery.
   Stale local forwarded sockets, dead SSH masters, and partial-forward
   failures should recover more predictably.

3. Improve mismatch/error reporting.
   Version mismatch, missing remote binary, or remote startup failure should
   surface as clear remote-host-oriented errors.

4. Keep verification strong.
   The SSH path should keep relying on direct control-socket and stream-level
   checks rather than focus-sensitive GUI automation.

## Verification Guidance

Prefer direct, socket-level verification for SSH desktop mode.

Good acceptance checks:

- `ping` over the forwarded control socket
- `get-ui-snapshot` over the forwarded control socket
- creating a new session through the forwarded control socket
- observing `.stream` attach/update behavior through the forwarded path

The practical verification lane remains the repo scripts such as:

- [`scripts/verify-remote-host.sh`](/Users/example/dev/boo/scripts/verify-remote-host.sh)
- [`scripts/verify-remote-mac.sh`](/Users/example/dev/boo/scripts/verify-remote-mac.sh)

## Relationship To The Long-Term Remote Direction

This SSH design is:

- a real supported desktop milestone
- the current bootstrap and trust boundary
- not the final shared transport contract for desktop and iOS

The canonical long-term product direction remains:

- one Boo-native remote protocol/session model
- one transport story that can serve both desktop and iOS
- SSH retained where useful for bootstrap, identity, or deployment

## Current Acceptance Target

The practical acceptance target remains:

```bash
boo --host <ssh-host>
```

Expected behavior:

1. SSH to the remote host
2. ensure remote `boo server --socket <remote-socket>` is running
3. create both forwarded local sockets on the client machine
4. connect the current Boo GUI/client to those sockets

If that works reliably, the SSH desktop milestone is behaving as intended.

It does not mean the final transport architecture is done. It only means Boo
can deliver a useful remote desktop flow while the unified transport work
continues.
