# Runtime-View Follow-up Plan

Updated: 2026-04-26

## Summary

`docs/remote/implementation-checklist.md` is now the canonical checklist for
completed runtime-view work. This file tracks only the remaining follow-up plan
and the current simulator/reconnect reliability slice.

The latency-tolerant remote UI work, macOS terminal UI regressions, and
`flake.nix` toolchain cleanup are no longer active TODOs here; they are recorded
as completed in `implementation-checklist.md`.

## Remaining TODOs

### 1. Real-device iOS smoke coverage

- Keep real-device smoke lanes current for both iPad and iPhone using existing
  argv-style script flags.
- 2026-04-26 physical iPad validation passed after restoring full Apple tooling
  access:
  - device discovery sees `Changsheng's iPad`
    (`00008110-00043032360A801E`)
  - device-state preflight reports developer mode enabled and unlocked
  - simulator-vs-iPad metrics comparison passes with signpost export
  - macOS+iPad two-client runtime-view scenario passes with one local client on
    tab 1 and the iPad client on tab 2 with three visible panes
- Keep physical-device artifacts useful for debugging:
  - screenshot attachments
  - signpost exports
  - runtime/action metrics
  - server logs
- Verify the two-client scenario periodically with a macOS client and an iPad
  client connected to the same server:
  - exactly two tabs
  - one tab with three panes, split horizontal once and vertical once
  - each client viewing a different tab
  - per-client focus/view state remains independent

## Completed / No Longer Active Here

### Reconnect UX and empty-runtime recovery

Completed for simulator/desktop-verifiable behavior:

- iOS distinguishes opening, detached, expired, unreachable, exited, and
  reachable runtime-tab health instead of collapsing every no-active-tab case
  into a generic disconnected state
- detached runtime views show a reattach-oriented banner and `Reconnect` action
- expired empty-runtime views show a recovery banner and explicit `Connect`
  action that requests a new tab
- passive remote connections still do not create tabs by themselves
- interactive attach/connect keeps the one-shot explicit `NewTab` bootstrap for
  empty runtimes
- remote client diagnostics expose `runtime_view_status` and `ui_attached` for
  active/detached/expired observability
- iOS protocol self-tests and Rust diagnostics tests cover the state mapping

### Per-screen scroll/search/copy semantics

Completed for protocol/server semantics:

- view-local scroll/search/copy state belongs to each remote `view_id`, not the
  shared tab/pane runtime
- runtime actions now cover scroll focused pane, enter/exit copy mode,
  copy/search navigation, and search query update
- desktop/local behavior remains unchanged unless a remote view explicitly
  drives these actions
- pure unit tests cover two clients viewing the same tab with independent focus
  and independent scroll/search/copy state

### Simulator metrics lane and CoreSimulator reliability

Automated for simulator-only verification on this Mac:

- simulator/iPad comparison scripts are argv-driven for device/team selection
- Boo server startup is early/non-blocking, with `wait-ready` deferred until
  just before client/UI connection
- readiness checks are deterministic socket checks rather than sleep-based
- simulator preflight captures `simctl-list-devices.json` and
  `simctl-list-devices.stderr`
- CoreSimulator/simdiskimaged failures fail fast with captured artifacts and
  manual repair guidance
- headed simulator live sessions are bounded by explicit duration
- scripts clean up their own Boo server, publishers, and temporary resources

### Latency-tolerant remote UI

Completed in the canonical checklist:

- backward-compatible runtime-action envelopes with `client_action_id`
- legacy bare `RuntimeAction` decode
- `RuntimeAction::Noop { view_id }`
- no-op/action-ack metrics separate from `remote.heartbeat_rtt`
- iOS transport isolation off `MainActor`
- deterministic protocol-state waits instead of sleep-based readiness
- safe optimistic focus/viewed-tab/resize feedback
- pane-aware QoS, coalescing, starvation guard, and render-ack feedback
- terminal text/content remains server-authored

### macOS terminal UI regressions

Completed in the canonical checklist:

- invisible/transparent terminal content
- inconsistent glyph width
- content background regression from translucent to fully dark
- desktop input routing
- statusbar tab clicks
- normal-click hyperlink crashes

### flake.nix toolchain cleanup

Completed in the canonical checklist:

- macOS/iOS toolchain policy moved into `flake.nix`
- scripts no longer need ad hoc Xcode/SDK unsetting or local VT library-path
  discovery as the primary workaround
- Cargo uses the Nix-built shared `libghostty-vt`
- Xcode/iOS commands stay on real Xcode toolchains

## Current Verification Targets

### Rust/unit

- `cargo check`
- targeted runtime-server tests for empty-runtime recovery and view-state/QoS
- pure tests for future scroll/search/copy semantics

### Desktop/runtime-view

- `bash scripts/test-runtime-view-e2e-metrics.sh --desktop-only --seed 42 --bytes-per-pane 1048576`
- verify `metrics.json`, `summary.md`, and `qos-baseline.md`
- process-targeted macOS recording with `scripts/record-macos-window.swift`
  when debugging visual regressions

### iOS/simulator/device

- `bash scripts/compare-ios-simulator-ipad-metrics.sh --simulator-only`
- `bash scripts/compare-ios-simulator-ipad-metrics.sh --ios-device-id <device-id> --team-id <team-id>`
- `bash scripts/test-ios-remote-view.sh`
- `bash scripts/verify-ios-signposts.sh --device-id <device-id> --team-id <team-id> --scenario runtime-view-e2e`
- `bash scripts/test-ios-ui.sh --destination 'id=<iphone-id>' --team-id <team-id>`
- `bash scripts/test-ios-ui.sh --destination 'id=<ipad-id>' --team-id <team-id>`

Latest 2026-04-26 physical iPad artifacts:

- simulator-vs-iPad metrics:
  `/tmp/boo-ios-sim-vs-ipad-current/comparison.md`
- macOS+iPad two-client runtime-view metrics:
  `/tmp/boo-runtime-view-two-client-ipad-current/comparison.md`

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
- General terminal text prediction remains off. Latency work remains limited to
  action acknowledgements, off-main iOS transport, safe optimistic view-local
  UI, pane-aware QoS/backpressure, and measured future transport experiments.
