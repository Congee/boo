# Linux Platform Layer

Linux uses the same shared `libghostty-vt` runtime model as macOS.

## Responsibilities

The Linux-specific layer should stay focused on:

- host integration
- clipboard and platform event plumbing
- backend selection and Linux-specific runtime glue

The shared runtime should continue to own:

- PTY lifecycle
- terminal ownership
- snapshots
- rendering-facing state
- command-state tracking

## Primary Files

- `src/platform/linux.rs`
- `src/backend.rs`
- `src/vt_backend_core.rs`
- `src/unix_pty.rs`

## Verification Guidance

Linux testing should prefer:

- control-socket checks
- UI snapshot harnesses
- benchmark/profiling scripts

Video capture is useful for visual-only regressions, but not the primary proof
for most Linux runtime work.
