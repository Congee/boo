# Finish All Open Runtime-View TODOs

## Summary

Complete every unchecked item in `docs/remote/implementation-checklist.md` that
is still relevant to runtime-view work, excluding the already-completed VT
wrapper checklist. The work should end with those checklist items checked,
reproducible metrics/artifacts committed or generated under ignored paths, and
validated desktop + iOS paths.

## Key Changes

### Per-screen scroll/search/copy semantics

- Define remote runtime-view semantics in docs and code: view-local
  scroll/search/copy state belongs to each remote `view_id`, not the shared
  tab/pane runtime.
- Add runtime-view actions only where needed for remote clients: scroll focused
  pane, enter/exit copy mode, copy/search navigation, and search query update.
- Preserve desktop/local behavior unless a remote view explicitly drives these
  actions.
- Add pure unit tests for two clients viewing the same tab with independent
  focus, scroll/search/copy state.

### Transport QoS

- Replace focused-pane-first-only publishing with explicit per-client
  scheduling:
  - focused pane always emitted first
  - non-focused visible panes coalesced within a small bounded window
  - starvation guard forces visible non-focused panes after a max skipped/update
    count
- Keep wire formats unchanged; change only server-side scheduling/coalescing
  behavior.
- Add deterministic tests for focused priority, non-focused coalescing,
  starvation, and multi-client focused-pane differences.

### Runtime-view E2E metrics and local prediction decision

- Run and fix the desktop `runtime-view-e2e` metrics path until it reliably
  produces parseable `metrics.json`, `summary.md`, and `qos-baseline.md`.
- Use the baseline to close the local-prediction TODO with a documented
  decision: no local prediction in this pass unless QoS still leaves
  user-visible input/focus p95 latency above the recorded threshold.
- Keep artifacts generated/ignored, not committed as large files.

### macOS terminal UI regressions

- Reproduce with the process-targeted capture path, especially
  `scripts/record-macos-window.swift`.
- Fix:
  - invisible/transparent terminal content
  - inconsistent glyph width
  - content background regression from translucent to fully dark
- Add snapshot/visual assertions where practical so future runs catch these
  regressions.

### flake.nix toolchain cleanup

- Move Xcode/SDK and `libghostty-vt` environment policy into `flake.nix`.
- Keep Cargo using the Nix-built shared `libghostty-vt`.
- Keep Xcode/iOS commands using real Xcode toolchains, not Nix SDK leakage.
- Remove script-local `env -u SDKROOT` and library-path workarounds once the
  flake provides the right clean shell behavior.

### Reconnect UX and iOS smoke coverage

- Refine host-scoped reconnect UX so a detached mobile runtime view clearly
  shows recoverable, detached, and expired states and reconnects to the
  canonical host view.
- Ensure timeout affordances are visible in iOS UI state and control snapshots.
- Keep real-device smoke lanes current for both iPad and iPhone using existing
  argv-style script flags.

## Test Plan

### Rust/unit

- `nix develop --command cargo check`
- `nix develop --command cargo clippy`
- `nix develop --command cargo test -- --test-threads=1`
- targeted runtime-server QoS/view-state tests

### Desktop/runtime-view

- `bash scripts/test-runtime-view-e2e-metrics.sh --desktop-only --seed 42 --bytes-per-pane 1048576`
- verify `metrics.json`, `summary.md`, and `qos-baseline.md`
- process-targeted macOS recording with `scripts/record-macos-window.swift`

### iOS/device

- `bash scripts/test-ios-remote-view.sh`
- `bash scripts/verify-ios-signposts.sh --device-id <device-id> --team-id <team-id> --scenario runtime-view-e2e`
- `bash scripts/test-ios-ui.sh --destination 'id=<iphone-id>' --team-id <team-id>`
- `bash scripts/test-ios-ui.sh --destination 'id=<ipad-id>' --team-id <team-id>`

### Hygiene

- `nix flake check --no-build`
- `git diff --check`
- verify no tracked generated artifacts or stale background Boo processes remain

## Assumptions

- Do not reopen the completed VT wrapper checklist; future upstream `Terminal`
  adoption notes are not active TODOs.
- Preserve remote wire formats unless a new runtime action is strictly required.
- Do not run broad `cargo fmt`/`rustfmt`; avoid formatting crate roots unless
  specifically necessary.
- Generated metrics/workload files stay under ignored generated paths.
- QoS changes are allowed; local prediction remains off unless baseline results
  prove it is necessary.
