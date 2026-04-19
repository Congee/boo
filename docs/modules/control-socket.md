# Module: Control Socket

Primary file:

- `src/control.rs`

## Role

The control socket is the local request/response IPC surface for boo.

It is used for:

- server readiness checks
- listing sessions
- creating sessions
- querying UI/session state
- diagnostic and automation workflows

Default transport:

- Unix domain socket, typically `/tmp/boo.sock`

## Relationship To The Stream Socket

The control socket is not the same as the long-lived `.stream` socket.

- control socket: request/response RPCs
- `.stream` socket: live state and delta updates for the GUI

That split is important for both local operation and SSH-backed desktop remote
mode.

## Related Files

- `src/client_gui.rs`
- `src/server.rs`
- `src/launch.rs`

## Verification

This module is central to deterministic testing. Prefer direct control-socket
checks over focus-sensitive GUI automation when validating startup or remote
behavior.
