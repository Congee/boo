# Remote Boo Over SSH: Plan And Context

Date: April 17, 2026

Goal: implement the first remote desktop milestone where the user runs something like:

```bash
boo --host <ssh-host>
```

and Boo automatically uses SSH under the hood to:

- ensure a remote Boo server is running on the target host
- create a secure tunnel back to Boo on that host
- connect the local GUI/client to the remote Boo session

The intended direction is to develop this on the Linux machine and make it connect back to this Mac.

This document is intentionally about the SSH milestone, not the final cross-platform remote
transport. The long-term product contract is in [REMOTE_REQUIREMENTS.md](./REMOTE_REQUIREMENTS.md),
where SSH is treated as:

- the first remote desktop milestone
- a practical bootstrap and trust path
- not the final unified transport contract for desktop and iOS

## Product Direction

Do not make Boo listen on TCP port 22.

For this milestone, use SSH as the transport and security boundary. Boo should feel host-native to
the user, but internally ride on SSH.

Recommended milestone user experience:

```bash
boo --host mac-hostname.local
```

with no manual SSH or tunnel setup required in the normal case.

This is a milestone UX, not the final universal remote story. The canonical long-term direction is
still a Boo-native transport shared by desktop and iOS.

## Why SSH For The First Milestone Instead Of Raw Boo TCP

Boo already has:

- a local control socket
- a local `.stream` socket for live UI/session updates
- a separate TCP remote daemon path

The best near-term implementation is not to rewrite the client around the TCP daemon. Instead:

- keep Boo server private on the remote host
- use SSH to bridge the existing control and stream sockets
- let the current GUI/client continue to talk to local-looking sockets

That preserves current architecture and minimizes protocol churn while the canonical Boo-native
transport is still being built.

## Two-Socket Model

Boo currently uses two local IPC endpoints:

- control socket: request/response RPCs such as ping, list, snapshots, new-session
- stream socket: long-lived state and delta transport for the GUI

This split is useful because:

- control RPCs do not get blocked behind heavy stream traffic
- the stream path can remain optimized for ordered, continuous updates
- control and live rendering have different lifecycle and reconnection semantics

So the SSH-based remote design should forward both sockets, not try to collapse them at first.
Collapsing control and stream belongs to the later unified transport work, not to this milestone.

## Key Architectural Clarification

A Unix socket is host-local only.

So a “remote socket” in this design means:

- a Unix socket path that exists on the remote machine

That is only useful cross-host when SSH forwards it.

There are two layers:

- remote Boo socket on the remote machine, for example `/tmp/boo.sock`
- local forwarded socket on the client machine, for example `/tmp/boo-macbook.sock`

The local Boo GUI connects only to the local forwarded socket. SSH carries the bytes to the remote Unix socket.

## Recommended Implementation

### User-facing CLI/config

Add:

- `--host <ssh-host>`

Add config keys such as:

- `remote-host`
- `remote-workdir`
- `remote-socket`
- `remote-binary`

Keep:

- `--socket` for explicit local override

### Startup behavior

In the host case, replace local autostart with:

1. determine the remote host
2. determine the remote Boo socket path
3. determine a host-specific local forwarded socket path
4. SSH into the remote host and start Boo server if needed
5. create SSH forwards for:
   - local control socket -> remote control socket
   - local stream socket -> remote stream socket
6. connect the existing local GUI/client to the forwarded local socket

### Transport choice

Preferred milestone version:

- SSH forwarding of Unix sockets

Fallback if Unix-socket forwarding turns out awkward in the target environment:

- remote Boo server exposes loopback-only TCP
- SSH forwards local socket or local TCP to remote loopback TCP

But the first implementation target should be “reuse existing local socket contract through SSH”.

## Concrete Operational Model

Remote host:

- Boo server runs on something like `/tmp/boo.sock`
- `.stream` companion exists at `/tmp/boo.sock.stream`
- Boo is not directly exposed to the network

Client host:

- Boo creates host-specific local socket names like `/tmp/boo-<host>.sock`
- SSH forwards:
  - `/tmp/boo-<host>.sock` -> remote `/tmp/boo.sock`
  - `/tmp/boo-<host>.sock.stream` -> remote `/tmp/boo.sock.stream`
- local GUI uses `/tmp/boo-<host>.sock`

## Best-Practice User Story

The user should not have to think about SSH in the common path.

Internally Boo should:

- reuse an SSH master connection if available
- start the remote Boo server if it is not already running
- establish forwards automatically
- connect the local GUI
- show remote-host-oriented errors if setup fails

In other words:

- user concept: “connect to remote Boo on host X”
- implementation detail: SSH

For the final product, that implementation detail should become optional. Desktop and iOS are
expected to converge on the canonical Boo-native transport described in
[REMOTE_REQUIREMENTS.md](./REMOTE_REQUIREMENTS.md).

## Implementation Plan

1. Define the transport contract.
   `--host <ssh-host>` is the only new remote entrypoint.

2. Extend config and CLI.
   Add:
   - `--host`
   - `remote-host`
   - `remote-workdir`
   - `remote-socket`
   - `remote-binary`

3. Add host-aware startup path.
   In startup logic:
   - local mode keeps current behavior
   - host mode runs remote bootstrap + SSH forwarding

4. Implement remote server bootstrap over SSH.
   The helper should:
   - SSH to the host
   - `cd` into configured workdir
   - verify remote Boo binary exists
   - start `boo server --socket <remote-socket>` if not already running
   - pass through any relevant session/auth flags

5. Implement SSH forwarding for both sockets.
   Forward:
   - control socket
   - `.stream` socket

6. Keep the local GUI/client unchanged.
   Connect it to the locally forwarded socket path.

7. Add lifecycle handling.
   Handle:
   - stale local socket cleanup
   - tunnel reuse or restart
   - disconnect/reconnect policy

8. Add tests.
   Unit tests:
   - config parsing for remote-host settings
   - host-specific socket naming
   - startup branch selection

9. Add a real integration check.
   On the Linux machine, verify:
   - remote Boo server starts on the Mac
   - both forwarded sockets exist locally on Linux
   - control ping works through the tunnel
   - UI snapshot works through the tunnel

10. Polish UX later.
   Optional:
   - status text like `Connected to <host>`
   - reuse of SSH ControlMaster/ControlPersist
   - automatic reconnect

## Recommended Verification Strategy

Prefer direct, socket-based verification.

Do not rely on focus-sensitive GUI injection or global OS key injection for the transport validation.

Good acceptance checks:

- `ping` over the forwarded control socket
- `get-ui-snapshot` over the forwarded control socket
- creating a new session via the forwarded control socket
- observing the local `.stream` attach/update path

## Notes About Current Repo State On This Mac

Before switching development to Linux, the following work happened on the Mac repo:

- `vendor/libghostty-vt-sys/build.rs` was updated and committed separately for cross-platform shared-library alias/rpath handling
- commit created on this Mac repo:
  - `5a577a0` `Fix cross-platform libghostty-vt linking`

There was also a partial, local-only attempt to begin `--host` support on this Mac branch. That work is not the recommended source of truth. The cleaner plan is to reimplement the feature on Linux using the design above, aimed at connecting back to this Mac.

At the time of writing, the local Mac worktree also had an unrelated modified `Cargo.lock`.

## Preferred First Milestone

Implement exactly this:

```bash
boo --host <mac-host>
```

Behavior:

1. SSH to the Mac
2. ensure remote `boo server --socket <remote-socket>` is running
3. create both forwarded local sockets on Linux
4. connect the current Boo GUI/client on Linux to those sockets

If that works, the architecture is validated.

It does not mean the final transport architecture is done. It only validates that Boo can deliver a
useful remote desktop flow immediately while the unified transport work continues.
