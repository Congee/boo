# Roadmap

This file tracks the main product and engineering direction for boo. It is a
prioritized roadmap, not a changelog.

## Now

### Remote Hardening

- Expand SSH desktop remote path handling for `remote-binary`,
  `remote-workdir`, and related settings
- Tighten tunnel lifecycle recovery and stale-forward cleanup
- Improve remote version mismatch and startup error reporting
- Keep desktop SSH verification strong through control-socket and stream checks

### tmux Parity

- Remain-on-exit and respawn-pane style lifecycle controls
- Session/window rename and move/link behavior closer to tmux
- Hooks, formats, `run-shell`, and `if-shell`

### Performance

- Continue renderer and transport profiling on real workloads
- Tighten redraw cadence and publish timing on hot interactive paths
- Expand regression coverage for terminal-heavy scenarios

## Next

### Unified Remote Direction

- Keep desktop SSH as the supported milestone
- Continue converging desktop and iOS on one canonical Boo-native remote model
- Improve channel framing, resume behavior, and diagnostic surfaces
- Replace Boo-native shared-secret auth ideas with SSH-style public-key trust:
  server-side `~/.ssh/authorized_keys` verification, client-side platform
  keychain or agent usage, and no private-key storage inside Boo

### UX Hardening

- Better remote-host status and degraded-state presentation
- More shell integration coverage for command-state and title updates
- Better mouse/text selection parity

### Documentation

- Keep the new top-level docs as the canonical entrypoints
- Add ADRs for decisions that should not get buried in code or PR history
- Continue modular subsystem docs as the codebase evolves

## Later

### Remote Transport Convergence

- Move beyond the first-cut SSH-forwarded-socket milestone when the Boo-native
  unified transport is mature enough
- Improve reconnect behavior across mobile and desktop clients

### Platform Breadth

- Continue Linux polish around host integration and verification
- Keep macOS IME, dead-key, and notification behavior hardened

### Contributor Experience

- More targeted docs for high-churn or high-risk modules
- Cleaner release and packaging guidance once distribution stabilizes

## Related Docs

- [docs/remote/requirements.md](./docs/remote/requirements.md)
- [docs/remote/ssh-desktop.md](./docs/remote/ssh-desktop.md)
- [docs/development/profiling.md](./docs/development/profiling.md)
- [docs/reference/features.md](./docs/reference/features.md)
