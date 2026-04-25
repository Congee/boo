# Remote Implementation Checklist

This page tracks the current remote-runtime redesign toward a shared server-owned
runtime plus per-screen view state.

## Status

This redesign pass is complete.

Implemented outcomes:

- one shared server-owned runtime truth for tabs, panes, layout, and semantic
  mutations
- one server-owned per-screen view state for viewed tab, focused pane, visible
  panes, viewport, and attach/detach lifecycle
- pane-scoped streaming with `tab_id + pane_id + pane/runtime revision`
  linkage
- focused-pane-first per-screen publish ordering
- normalized split-ratio resizing that maps correctly into different screen
  sizes
- detached-view timeout cleanup without immediately destroying shared runtime
  state

Remaining follow-up work is intentionally post-v1 and is documented in the
deferred section at the bottom of this file.

## Current TODO (updated 2026-04-25)

- [ ] Define scroll/search/copy-mode semantics across per-screen runtime views.
- [ ] Implement the latency-tolerant remote UI plan:
      action acknowledgements, no-op baseline metrics, off-main iOS transport,
      safe optimistic view-local UI, and pane-aware QoS/backpressure.
      See [latency-tolerant-remote-ui.md](./latency-tolerant-remote-ui.md).
- [ ] Refine canonical host/runtime reconnect UX and view timeout affordances.
- [ ] Keep real-device iOS UI smoke tests current for both iPad and iPhone.

Recently closed:

- [x] Added runtime-view E2E/iOS verification hooks:
      argv result bundles for iOS UI tests, physical-device screenshot export,
      desktop base no-op RTT metrics, iOS heartbeat RTT tracing, and
      `runtime-view-e2e` signpost defaults.
- [x] Fixed real-device iOS runtime-view pane rendering and tap focus:
      pane UI state now decodes the local input-sequence prefix, SwiftUI is
      forced to observe pane-state publication, and the physical iPad
      three-pane screenshot/tap-focus smoke test passes.
- [x] Fixed macOS terminal UI regressions: invisible/transparent content,
      inconsistent glyph width, fully-dark background regression, desktop
      input routing, statusbar tab clicks, and normal-click hyperlink crashes.
- [x] Moved remaining macOS/iOS toolchain cleanup into `flake.nix` so scripts no
      longer need ad hoc Xcode/SDK unsetting or local VT library discovery.

## Latency-Tolerant Remote UI Tracking

Canonical design:
[latency-tolerant-remote-ui.md](./latency-tolerant-remote-ui.md).

### Measurement and acknowledgements

- [x] Add a backward-compatible runtime-action envelope with
      `client_action_id`.
- [x] Continue accepting legacy bare `RuntimeAction` payloads.
- [x] Add `RuntimeAction::Noop { view_id }`.
- [x] Add action acknowledgement metadata to runtime-view state.
- [x] Add `remote.noop_roundtrip`.
- [x] Add `remote.action_ack`.
- [x] Add `remote.optimistic_apply`.
- [x] Add `remote.reconcile`.
- [x] Update simulator+iPad metrics comparison so no-op/action metrics are
      reported separately from `remote.heartbeat_rtt`.

### iOS transport isolation

- [ ] Move `NWConnection`, heartbeat, frame IO, and decode off MainActor.
- [ ] Keep MainActor responsible only for reduced state publication into
      SwiftUI.
- [ ] Verify heartbeat/no-op ack can progress while UI/AX work is busy.
- [ ] Replace sleep-based readiness checks with deterministic protocol-state
      waits.

### Safe optimistic view-local UI

- [x] Optimistically apply focus-pane UI state immediately.
- [x] Optimistically apply viewed-tab/statusbar selection immediately.
- [x] Optimistically apply split-resize handle geometry while dragging.
- [x] Tag optimistic focus/viewed-tab state with `client_action_id`.
- [x] Clear optimistic focus/viewed-tab state on matching server ack/revision.
- [x] Roll back optimistic focus/viewed-tab state on conflicting
      authoritative state.
- [x] Keep terminal text output server-authored.

### Pane-aware QoS and backpressure

- [ ] Replace single latest-screen coalescing with pane-aware scheduling.
- [x] Prioritize health/control/action acknowledgement frames.
- [ ] Prioritize focused visible pane updates per client.
- [ ] Coalesce non-focused visible pane updates by pane.
- [ ] Add starvation guard for non-focused visible panes.
- [ ] Add queue-depth/render-ack feedback for passive pane throttling.
- [ ] Preserve per-client priority differences when clients view/focus
      different tabs or panes.

### Future transport separation

- [ ] Evaluate multi-stream QUIC only after action ack, off-main iOS transport,
      optimistic UI, and QoS metrics are available.
- [ ] Evaluate unreliable delivery only for staleable transient UI state.
- [ ] Keep authoritative terminal state on reliable state/delta paths.

## Post-completion verification notes

- 2026-04-23 real-device verification on connected physical iPad and iPhone
  hardware now gets through:
  - Swift compile
  - link
  - signing
  - install
  - UI test runner launch
  - execution of focused XCUITest methods
  - discovered-daemon connection
  - terminal screen entry
  - typing into the remote terminal and observing the echoed marker
- the previous device-build blocker was first isolated as `xcodebuild`
  environment leakage from the repo shell, especially `LD`, `CC`, `CXX`,
  `SDKROOT`, and `NIX_LDFLAGS`; that cleanup now belongs in `flake.nix`, so
  iOS scripts can call `xcodebuild` directly from `nix develop`
- real-device setup failures seen during verification were provisioning,
  locked-device, developer-disk-image, or UI Automation readiness issues before
  the app launched; once those were resolved, the discovered-daemon
  connect-and-type path passed on real hardware

## Current Emphasis

- move remote targeting/focus semantics from client-owned selection to
  server-owned per-view state
- keep the shared runtime truth authoritative for tabs, pane trees, pane
  content, and layout
- verify behavior with protocol/unit coverage while compatibility shims are
  retired

## Runtime Protocol Redesign

### Protocol model

- [x] introduce a shared authoritative runtime metadata payload
- [x] introduce explicit per-screen runtime-view state on the server
- [x] add first-class `RuntimeAction` protocol messages for semantic mutations
- [x] route all semantic runtime mutations through `RuntimeAction` in main flows
- [x] retire compatibility-only `ListTabs` / `Create` / `Destroy` framing from
      primary paths

### Runtime and view data model

- [x] model per-view state with `view_id`
- [x] track per-view viewed tab
- [x] track per-view focused pane
- [x] track per-view viewport size
- [x] track per-view visible pane membership
- [x] reflect explicit `tab -> pane_ids` hierarchy in runtime metadata
- [x] include runtime/view revision metadata in runtime payloads
- [x] maintain explicit per-pane terminal revisions independent of runtime/view
- [x] use runtime/view revisions for stale-update rejection and pane refresh

### Publish scoping and transport

- [x] publish runtime metadata per client view instead of one shared active-tab
      snapshot
- [x] scope pane updates by `tab_id + pane_id`
- [x] scope non-focused pane streaming to panes visible on that screen
- [x] prove only visible panes are streamed to each screen with targeted tests
- [x] implement explicit per-screen focused-pane prioritization/coalescing
- [x] cover divergent focused-pane scheduling across multiple screens

### Shared runtime vs per-screen semantics

- [x] keep shared runtime truth for tab/pane lifecycle and layout
- [x] allow different screens to track different viewed tabs server-side
- [x] allow different screens to track different focused panes server-side
- [x] deterministically resolve viewed-tab fallback when a viewed tab closes
- [x] keep iOS view/session alive after UI close until timeout
- [x] implement idle cleanup timeout semantics

### iOS/runtime-view client work

- [x] decode richer runtime metadata on the client wire model
- [x] preserve compatibility with the current single-screen bootstrap flow
- [x] add iOS-side `RuntimeAction`-driven semantic interactions in the main UI
- [x] render all visible panes for the viewed tab on iOS
- [x] rely on the Boo core statusbar for tab-list UI instead of rendering
      native iOS runtime tab chrome
- [x] drive pane hit-testing/focus changes from server-provided pane frames
- [x] preserve semantic runtime actions for statusbar/tab effects without
      client-owned tab lifecycle state
- [x] drive divider resize semantically using normalized split ratios
- [x] keep focused pane interaction hottest without local prediction

### Testing

- [x] cover runtime-action decode at the protocol layer
- [x] cover server-side client view initialization
- [x] connect a new screen and verify initial runtime/view bootstrap semantics
- [x] verify changing viewed tab only affects that screen
- [x] verify closing a shared tab remaps viewed tab deterministically
- [x] verify different screens can keep different focused panes on one tab
- [x] verify stale pane updates are rejected/refreshed via revision linkage
- [x] verify normalized split resize reflects across different screen sizes
- [x] verify focused-pane traffic is scheduled ahead of non-focused panes
- [x] verify tab/status-bar semantic actions propagate shared runtime changes

### Deferred / TODO

- [ ] define scroll/search/copy-mode semantics across per-screen views
- [ ] implement latency-tolerant remote UI architecture:
      action acknowledgements, no-op roundtrip baseline, safe optimistic
      view-local UI, iOS transport off MainActor, pane-aware QoS, and
      backpressure. Keep terminal text prediction deferred until those slices
      are measured.
- [x] revisit terminal UI regressions found during macOS runtime-view testing:
      invisible/transparent content, inconsistent glyph width, and the content
      background changing from translucent to fully dark
      - fixed the macOS font fallback/metrics path so Menlo is first and
        measured through CoreText instead of using a square-cell fallback
      - fixed the remote GUI terminal scene so translucent backgrounds are
        painted once rather than compounded by the outer container
      - fixed desktop remote input routing so typed text, app key events,
        mouse actions, splits, and resize actions target the requesting
        client's viewed tab/focused pane instead of the process-global active
        tab
      - made statusbar tab labels actionable in both the client GUI and
        standalone/runtime UI, sending the same runtime viewed-tab action used
        by other tab controls
      - guarded hyperlink lookup so normal pane clicks do not call Ghostty's
        formatter unless the snapshot cell is actually marked as a hyperlink
      - verified with process-targeted `scripts/record-macos-window.swift` and
        `scripts/capture-macos-window.sh` using a 0.55-opacity terminal body
- [x] move remaining macOS/iOS toolchain cleanup into `flake.nix` so scripts no
      longer need ad hoc `env -u SDKROOT` wrappers or local library-path
      discovery to work around dev-shell leakage
- [ ] refine canonical host/runtime reconnect UX and view timeout affordances
- [ ] keep real-device iOS UI smoke tests current for both iPad and iPhone

### Latency tracing and local prediction follow-up

- [x] add shared runtime-view latency tracing foundation:
      Rust `tracing`/`tracing-subscriber`, iOS `Logger`/`OSSignposter`, and
      shared event/field names
- [x] manually verify Rust latency traces with the `RUST_LOG`-style
      `--trace-filter boo::latency=info` CLI override
- [x] add repeatable Rust local-stream trace verification for
      `remote.focus_pane` -> focused screen update + `remote.pane_update`
      using `scripts/test-latency-traces.sh`
- [x] add repeatable iOS trace-state verification that pending input,
      focus-pane, and runtime-action spans end as `remote.render_apply`
- [x] verify iOS Logger output and Instruments signpost intervals for
      the same event names
- [x] add automated trace-output assertions for the remaining core latency flows:
  - [x] iOS tap pane -> `FocusPane` -> runtime state/pane update -> render
  - [x] iOS statusbar/tab runtime action -> update -> render
  - [x] iOS key/input -> terminal delta/full-state -> render
- [x] add no-op/action-ack metrics so minimal protocol roundtrip and
      user-perceived action latency are reported separately from
      `remote.heartbeat_rtt`
- [x] add safe optimistic UI for view-local actions only:
  - [x] focus pane
  - [x] viewed tab/statusbar selection
  - [x] split-resize handle geometry
- [ ] keep terminal text/content prediction deferred until action acks,
      optimistic view-local UI, off-main iOS transport, and pane-aware QoS are
      measured
- [x] bridge Rust traces to Apple OSLog with `tracing-oslog`
- [x] resolve the current device `xcodebuild` linker
      boundary if full iOS build verification is required

Trace verification notes:

- 2026-04-23 Rust smoke verification built `target/debug/boo`, ran a headless
  server with `RUST_LOG=boo::latency=info`, connected to the local stream
  socket, and sent minimal runtime-action/input frames.
- Observed shared event names and fields for `remote.connect`,
  `remote.runtime_action`, `remote.set_viewed_tab`, `remote.focus_pane`, and
  `remote.input`.
- 2026-04-24 added and ran `scripts/test-latency-traces.sh`. The script starts
  Boo with `--trace-filter boo::latency=info`, drives the local stream through
  `new_split` and `focus_pane`, asserts a focused screen update plus a
  non-focused `UiPaneFullState`/`UiPaneDelta`, and verifies trace output for
  `remote.connect`, `remote.runtime_action`, `remote.focus_pane`, and
  `remote.pane_update`.
- Rust/server `remote.pane_update` now has an end-to-end local-stream
  verification path.
- 2026-04-24 moved iOS pending render span completion into
  `BooRenderTraceTracker`, completed spans for focused `FullState`/`Delta`
  render applies as well as focused `UiPaneFullState`/`UiPaneDelta`, and added
  `ios/Validation/TraceRenderApplySelfTestMain.swift`. `scripts/test-ios-remote-view.sh`
  now runs that self-test and verifies `remote.render_apply` end records for
  input, focus-pane, and set-viewed-tab traces.
- 2026-04-24 added `scripts/verify-ios-signposts.sh` for real-device native
  verification. The script builds/installs Boo, starts a traced Boo daemon with
  the first-class `--trace-filter` flag, launches the iOS app under
  `xcrun xctrace record --template Logging --launch`, exports `os-log`,
  `os-signpost`, and `os-signpost-interval` tables, and asserts
  `remote.connect`, `remote.runtime_action`, `remote.pane_update`,
  `remote.input`, and `remote.render_apply`.
- Real-device signpost verification passed against a physical iPad. The
  Instruments interval export included a `remote.input` interval whose end
  metadata was `remote.render_apply`.
- `xctrace` all-processes Logging recordings ended early in this environment
  with `Device disconnected`; targeted `--launch -- me.congee.boo ...`
  recordings stayed alive for the requested time limit and are the repeatable
  verification path.
- The initial automated iOS native trace assertion covered auto-connect plus the
  input -> render path. It has since been extended to cover the focus-pane and
  set-viewed-tab core latency flows as well.
- `scripts/build-ios-device.sh` and `scripts/test-ios-remote-view.sh` now run
  `xcodebuild` through a clean environment so Nix/Xcode variables do not leak
  into the linker invocation. With that wrapper, physical-device build/install
  and the remote-view iOS build smoke both pass.
- 2026-04-24 extended `scripts/verify-ios-signposts.sh` with automated
  trace-output assertions for the remaining core flows. The launched iOS app can
  drive a UI-test-only runtime-action sequence of `new_split` -> `focus_pane`,
  `new_tab` -> `set_viewed_tab`, and input without rendering native tab chrome;
  the script asserts both the begin events and `remote.render_apply` end records
  via `source_event` plus Instruments interval rows for `remote.focus_pane`,
  `remote.set_viewed_tab`, and `remote.input`.
- 2026-04-24 post-change real-device verification passed after extending the
  verifier to set the local Boo server's `libghostty-vt` dynamic-library search
  path. The passing run used:
  `bash scripts/verify-ios-signposts.sh --device-id <device-udid> --team-id <team-id> --time-limit 20s --output-dir /tmp/boo-ios-signpost-core-flows2 --skip-build --skip-install`.
- 2026-04-24 added the Rust-to-Apple-OSLog bridge with `tracing-oslog` on
  Apple targets. Rust traces still go to the normal formatted tracing sink, and
  the same events are also emitted to OSLog under subsystem `dev.boo.rust` and
  category `latency` for Console/Instruments correlation with iOS
  `dev.boo.ios` latency events.
- Verified the Rust OSLog bridge with `/usr/bin/log stream --predicate
  'subsystem == "dev.boo.rust" && category == "latency"'` while running
  `scripts/test-latency-traces.sh`; the OSLog stream included
  `remote.connect`, `remote.runtime_action`, `remote.focus_pane`, and
  `remote.pane_update`.

## Related Docs

- [./requirements.md](./requirements.md)
- [./ssh-desktop.md](./ssh-desktop.md)
- [../modules/remote-daemon.md](../modules/remote-daemon.md)
