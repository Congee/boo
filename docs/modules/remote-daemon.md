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
- runtime-view publishing plus tab create, resize, input, and diagnostics

## Current Shape

The old `src/remote.rs` monolith has been split into focused modules. `remote.rs`
now acts as the top-level coordination layer, while surrounding `remote_*`
files own narrower responsibilities such as:

- auth and handshake
- listener and transport setup
- wire encode/decode
- daemon state
- runtime-view, tab, and pane/full-state handling
- diagnostics and broadcast behavior

## Important Properties

- supports full-state and delta publishing
- supports heartbeat and reconnect-oriented metadata
- supports daemon identity metadata
- keeps only thin per-view runtime-view and stream-cache bookkeeping in the transport layer
- exposed diagnostics through `boo remote-clients`
- emits shared Rust latency trace events for connection, runtime actions, input,
  focus, pane updates, and Apple OSLog correlation on Apple platforms

## Post-v1 Follow-up

- strengthen transport QoS for focused-pane-first delivery under load
- keep non-focused visible pane streams coalesced without starvation
- preserve pane-scoped `tab_id -> pane_id` revision linkage when adding new
  remote messages

## Related Docs

- [../architecture/remote.md](../architecture/remote.md)
- [../remote/requirements.md](../remote/requirements.md)
- [../remote/implementation-checklist.md](../remote/implementation-checklist.md)
