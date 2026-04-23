# Contributing

This file covers the practical expectations for changing boo. It is for human
contributors. Tool-facing repo instructions remain in `AGENTS.md` and
`CLAUDE.md`.

## Prerequisites

- Rust toolchain with Cargo
- macOS or Linux development environment
- platform dependencies required by `iced` and `libghostty-vt`
- Xcode for iOS work

If you are working on Linux-specific runtime behavior, also read
[docs/architecture/platform-linux.md](./docs/architecture/platform-linux.md).

## Build

```bash
cargo build
```

If you are working through the flake entrypoints, these are the main commands:

```bash
nix build
nix run
nix develop
```

For profiling-oriented builds:

```bash
cargo build --profile profiling
```

## Test

Baseline:

```bash
cargo test
```

Nix validation:

```bash
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).default --no-link
```

Useful targeted checks:

- `bash scripts/test-ui-snapshot.sh`
- `bash scripts/test-ui-scenarios.sh`
- `bash scripts/test-headless.sh`
- `bash scripts/test-headless-scenarios.sh`
- `bash scripts/test-ios-remote-view.sh`
- `bash scripts/verify-remote-host.sh`

Pick the checks that match the area you changed. Do not claim coverage you did
not actually run.

## Workflow Expectations

1. Read the relevant module docs before changing architecture-heavy areas.
2. Make the smallest coherent change that solves the problem.
3. Verify with direct, reproducible checks first.
4. Keep docs aligned with behavior changes.

For remote transport, profiling, and iOS work, prefer repo-local scripts and
socket-level validation over focus-sensitive GUI automation.

## Repo Conventions

- Keep new code paths testable with deterministic harnesses where possible.
- Preserve the runtime-server ownership model; do not push PTY, tab, pane, or
  focus authority into GUI or remote clients.
- Keep platform-specific code thin when shared runtime logic is possible.
- Prefer crate-private interfaces for internal subsystem boundaries.
- Keep user-facing docs in canonical locations rather than scattering notes
  into transient markdown files.

## Documentation Expectations

Update docs when you change:

- CLI behavior
- control or remote protocol behavior
- architecture or module boundaries
- verification workflow
- roadmap status

Canonical docs:

- [README.md](./README.md)
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [ROADMAP.md](./ROADMAP.md)
- [docs/index.md](./docs/index.md)

## Pull Request Guidance

A good change should usually include:

- the problem being solved
- the key design choice or tradeoff
- what was verified
- any remaining risks or manual validation still needed

If a change is architectural, add or update a focused doc under `docs/` rather
than expanding a catch-all file indefinitely.

## Areas That Need Care

- `src/vt_backend_core.rs`: hot path, large state surface
- `src/control.rs`: local IPC contract
- `src/launch.rs`: local and SSH-backed startup logic
- `src/remote_*`: protocol and daemon behavior
- `ios/Sources/*`: mobile client protocol/UI behavior

Start with these docs when touching those areas:

- [docs/modules/vt-backend-core.md](./docs/modules/vt-backend-core.md)
- [docs/modules/control-socket.md](./docs/modules/control-socket.md)
- [docs/modules/remote-daemon.md](./docs/modules/remote-daemon.md)
- [docs/modules/ios-client.md](./docs/modules/ios-client.md)
