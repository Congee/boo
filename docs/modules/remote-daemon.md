# Module Group: Remote Daemon

Primary files:

- `src/remote.rs`
- `src/remote_auth.rs`
- `src/remote_listener.rs`
- `src/remote_wire.rs`
- `src/remote_client.rs`
- `src/remote_identity.rs`
- `src/remote_transport.rs`
- `src/remote_state.rs`
- `src/remote_server_*`

## Role

This subsystem implements Boo's native remote protocol and daemon behavior.

It powers:

- direct remote daemon access
- iOS client connectivity
- daemon diagnostics
- session attach/detach, create, resize, input, and state publishing

## Current Shape

The old `src/remote.rs` monolith has been split into focused modules. `remote.rs`
now acts as the top-level coordination layer, while surrounding `remote_*`
files own narrower responsibilities such as:

- auth and handshake
- listener and transport setup
- wire encode/decode
- daemon state
- session/full-state handling
- diagnostics, broadcast, and attachment behavior

## Important Properties

- supports full-state and delta publishing
- supports heartbeat and reconnect-oriented metadata
- supports daemon identity metadata
- exposed diagnostics through `boo remote-clients`

## Related Docs

- [../architecture/remote.md](../architecture/remote.md)
- [../remote/requirements.md](../remote/requirements.md)
- [../remote/implementation-checklist.md](../remote/implementation-checklist.md)
