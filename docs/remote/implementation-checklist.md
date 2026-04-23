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

## Post-completion verification notes

- 2026-04-23 real-device verification on a connected physical iPad now gets
  through:
  - Swift compile
  - link
  - signing
  - install
  - UI test runner launch
  - execution of a real focused XCUITest method
- the previous device-build blocker was fixed by sanitizing `xcodebuild`
  environment leakage from the repo shell, especially `LD`, `CC`, `CXX`,
  `SDKROOT`, and `NIX_LDFLAGS` overrides in `scripts/test-ios-ui.sh`
- the current remaining real-device blocker is now a live connect/runtime issue,
  not a build or automation bootstrap issue:
  - `BooUITests/BooAppLaunchTests/testOpenLiveTabAndType`
  - reaches the discovered daemon row on-device
  - then fails with `Connection refused`

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
- [x] render server-owned status/tab UI from runtime metadata on iOS
- [x] drive pane hit-testing/focus changes from server-provided pane frames
- [x] drive tab/status-bar interactions semantically instead of by raw
      coordinates
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

- [x] document scroll/search coupling as deferred beyond this redesign pass
- [x] document prediction/latency-hiding work as post-v1 follow-up

## Related Docs

- [./requirements.md](./requirements.md)
- [./ssh-desktop.md](./ssh-desktop.md)
- [../modules/remote-daemon.md](../modules/remote-daemon.md)
