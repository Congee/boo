# Module Group: Runtime Server

Primary files:

- `src/server.rs`
- `src/runtime_server.rs`
- `src/runtime.rs`
- `src/runtime_*`
- `src/client_gui.rs`

## Role

The runtime server is the long-lived owner of:

- PTYs
- tabs and panes
- terminal runtime state
- local control socket state
- live stream/update state

Desktop GUI, automation, and iOS remote clients are views/controllers of this
single runtime state. They do not own separate terminal lifecycles.

## Why It Matters

This is one of Boo's central design decisions:

- terminal state survives individual client/view disconnects
- local and remote clients observe the same runtime updates
- automation can talk to the control socket without owning terminal state
- tab and pane lifecycle is server-owned runtime state, not client-owned state

## Adjacent Subsystems

- [control-socket.md](./control-socket.md)
- [vt-backend-core.md](./vt-backend-core.md)
- [remote-daemon.md](./remote-daemon.md)

## Change Risks

Changes here can affect:

- local startup behavior
- runtime persistence
- GUI stream behavior
- remote runtime-view propagation
