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

### Latency-tolerant remote UI

Follow [latency-tolerant-remote-ui.md](./latency-tolerant-remote-ui.md). High
`remote.heartbeat_rtt` is expected under bad Wi-Fi/LAN jitter, so Boo should
decouple local intent feedback from authoritative server reconciliation.

Implementation order:

1. Measurement and acknowledgements:
   - add a backward-compatible runtime-action envelope with `client_action_id`
   - keep legacy bare `RuntimeAction` decode working
   - add `RuntimeAction::Noop { view_id }`
   - report no-op/action-ack metrics separately from `remote.heartbeat_rtt`
   - update simulator+iPad comparison artifacts to include those metrics
2. iOS transport isolation:
   - move `NWConnection`, heartbeat, frame IO, and wire decode off MainActor
   - keep MainActor responsible only for reduced SwiftUI state publication
   - prove heartbeat/no-op ack progress while UI/AX work is busy
3. Safe optimistic view-local UI:
   - optimistically show focus-pane, viewed-tab/statusbar, and split-resize
     handle feedback
   - tag optimistic state with `client_action_id`
   - clear or roll back optimistic state on authoritative ack/revision
   - keep terminal text/content server-authored
4. Pane-aware QoS/backpressure:
   - prioritize health/control/action acknowledgements and focused visible pane
     updates per client
   - coalesce non-focused visible panes by pane
   - add starvation guards and queue-depth/render-ack feedback
   - preserve different priority ordering for clients with different viewed or
     focused panes
5. Future transport split:
   - evaluate QUIC multi-stream only after the above slices are measured
   - reserve unreliable delivery for staleable transient UI, not authoritative
     terminal state

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
- General terminal text prediction remains off. The first latency-tolerant pass
  is limited to action acknowledgements, off-main iOS transport, safe
  optimistic view-local UI, and pane-aware QoS/backpressure.
