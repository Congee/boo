# Remote Implementation Checklist

This page tracks the current remote-runtime redesign toward a shared server-owned
runtime plus per-screen view state.

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
- [ ] route all semantic runtime mutations through `RuntimeAction` in main flows
- [ ] retire compatibility-only `ListTabs` / `Create` / `Destroy` framing from
      primary paths

### Runtime and view data model

- [x] model per-view state with `view_id`
- [x] track per-view viewed tab
- [x] track per-view focused pane
- [x] track per-view viewport size
- [x] track per-view visible pane membership
- [x] reflect explicit `tab -> pane_ids` hierarchy in runtime metadata
- [x] include runtime/view revision metadata in runtime payloads
- [ ] maintain explicit per-pane terminal revisions independent of runtime/view
- [ ] use runtime/view revisions for stale-update rejection and pane refresh

### Publish scoping and transport

- [x] publish runtime metadata per client view instead of one shared active-tab
      snapshot
- [x] scope pane updates by `tab_id + pane_id`
- [x] scope non-focused pane streaming to panes visible on that screen
- [ ] prove only visible panes are streamed to each screen with targeted tests
- [ ] implement explicit per-screen focused-pane prioritization/coalescing
- [ ] cover divergent focused-pane scheduling across multiple screens

### Shared runtime vs per-screen semantics

- [x] keep shared runtime truth for tab/pane lifecycle and layout
- [x] allow different screens to track different viewed tabs server-side
- [x] allow different screens to track different focused panes server-side
- [x] deterministically resolve viewed-tab fallback when a viewed tab closes
- [ ] keep iOS view/session alive after UI close until timeout
- [ ] implement idle cleanup timeout semantics

### iOS/runtime-view client work

- [x] decode richer runtime metadata on the client wire model
- [x] preserve compatibility with the current single-screen bootstrap flow
- [x] add iOS-side `RuntimeAction`-driven semantic interactions in the main UI
- [ ] render all visible panes for the viewed tab on iOS
- [x] render server-owned status/tab UI from runtime metadata on iOS
- [ ] drive pane hit-testing/focus changes from server-provided pane frames
- [x] drive tab/status-bar interactions semantically instead of by raw
      coordinates
- [ ] drive divider resize semantically using normalized split ratios
- [ ] keep focused pane interaction hottest without local prediction

### Testing

- [x] cover runtime-action decode at the protocol layer
- [x] cover server-side client view initialization
- [ ] connect a new screen and verify initial runtime/view bootstrap semantics
- [ ] verify changing viewed tab only affects that screen
- [ ] verify closing a shared tab remaps viewed tab deterministically
- [ ] verify different screens can keep different focused panes on one tab
- [ ] verify stale pane updates are rejected/refreshed via revision linkage
- [ ] verify normalized split resize reflects across different screen sizes
- [ ] verify focused-pane traffic is scheduled ahead of non-focused panes
- [ ] verify tab/status-bar semantic actions propagate shared runtime changes

### Deferred / TODO

- [ ] scroll/search coupling across screens
- [ ] prediction/latency-hiding work beyond v1

## Related Docs

- [./requirements.md](./requirements.md)
- [./ssh-desktop.md](./ssh-desktop.md)
- [../modules/remote-daemon.md](../modules/remote-daemon.md)
