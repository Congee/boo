# Module Group: Session Server

Primary files:

- `src/server.rs`
- `src/runtime_server.rs`
- `src/runtime.rs`
- `src/runtime_*`
- `src/client_gui.rs`

## Role

The session server is the long-lived owner of:

- PTYs
- tabs and panes
- session lifecycle
- local control socket state
- `.stream` session/update state

The GUI is a client of this server model, not the owner of terminal processes.

## Why It Matters

This is one of boo's central design decisions:

- sessions survive GUI exit
- local and remote attach paths can share the same model
- automation can talk to the control socket without owning terminal state

## Adjacent Subsystems

- [control-socket.md](./control-socket.md)
- [vt-backend-core.md](./vt-backend-core.md)
- [remote-daemon.md](./remote-daemon.md)

## Change Risks

Changes here can affect:

- local startup behavior
- session persistence
- GUI attach/detach behavior
- remote attach flows
