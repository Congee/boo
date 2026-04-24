# Runtime-View Migration Status

## Status

The runtime-view migration is complete for the v1 redesign.

The shipping model is now:

- one shared server-owned runtime truth
- one server-owned view state per connected screen
- semantic `RuntimeAction` mutations for tab/focus/split/view operations
- pane-scoped terminal streaming with revision linkage
- detached views that stay recoverable until server idle timeout cleanup

## Goal

Boo remote clients are views/controllers of one authoritative server runtime.
The server owns tabs, panes, focus, status, and terminal content. Desktop GUI
and iOS should observe the same runtime updates instead of creating or selecting
client-owned lifecycle objects.

## Product Model

- The Boo runtime server owns tab and pane lifecycle.
- iOS bootstraps from runtime state / active tab metadata.
- iOS and desktop mutate the same shared runtime with semantic actions.
- iOS does not create or destroy tabs as recovery workaround.
- iOS does not render separate native runtime tab chrome; the Boo core
  statusbar remains the visible tab-list UI.
- closing iOS UI detaches the view first; the shared runtime remains alive until
  idle timeout cleanup.
- per-screen viewed tab, focused pane, visible panes, viewport, and attachment
  state are server-owned view bookkeeping, not product-owned runtime state.

## Runtime Payload Contract

`UiRuntimeState` is the bootstrap truth for remote clients. It carries:

- runtime revision
- view revision
- view id
- viewed tab id
- focused pane id
- tab metadata including explicit `tab -> pane_ids`
- visible pane geometry
- mouse/status/pwd metadata
- viewport metadata for per-screen geometry mapping

Tab-list payloads may still exist as compatibility metadata, but they are no
longer the primary target-selection model.

Terminal full state and deltas are pane-scoped and carry:

- `tab_id`
- `pane_id`
- `pane_revision`
- `runtime_revision`

Clients reject stale pane traffic and refresh from full state on view/runtime
revision boundaries instead of guessing.

## Semantic Action Contract

Runtime-view clients may send semantic runtime actions such as:

- focus tab / pane
- attach / detach a view
- create / close tab
- next / previous tab
- create split
- resize split by normalized ratio
- send input to the focused runtime pane
- send app key / mouse events
- resize viewport
- scroll pane/runtime view
- detach the viewer UI

These are now first-class runtime commands, not client-owned lifecycle
workarounds.

## Removed From iOS Product Flow

- legacy target bootstrap
- resume-target persistence
- host-scoped preferred target persistence
- client-side create-tab recovery
- client-side close-tab recovery
- terminal error-banner New Tab / Close Tab actions
- disconnect destroying server tabs

## Remaining Intentional Legacy Terms

Some uses of the word "session" are unrelated to the removed remote model:

- Swift `URLSession`
- XCTest screenshot attachment API (`XCTAttachment`)
- AVFoundation recording API (`startSession`)
- tmux protocol session events
- saved layouts in `src/layout.rs` and the `--layout` flag, which are startup
  layout presets rather than runtime lifecycle objects
- JSON compatibility assertions that old `session_id` / `attached_tab` fields
  are absent

These should not be confused with remote runtime-view architecture.

## Implemented Results

- per-screen runtime metadata is published per client view instead of as one
  shared active-tab snapshot
- pane streaming is filtered by visible-pane membership per screen
- each screen gets focused-pane-first publish ordering, even when two screens
  focus different panes in the same tab
- normalized split ratios are shared runtime truth; each screen maps them into
  its own geometry using its own viewport
- detached views survive UI close and are cleaned up only after idle timeout
- runtime-view latency tracing exists across Rust and iOS using the shared
  remote event schema documented in
  [implementation-checklist.md](./implementation-checklist.md)

## Post-v1 Follow-up

- scroll, search, and copy-mode behavior still need a dedicated design pass for
  multiple screens and different viewport sizes
- local prediction is intentionally not part of v1; use the tracing foundation
  to collect user-perceived latency baselines first, then decide whether to
  predict focus/tab/status changes
- focused-pane-first publishing exists, but transport QoS should be hardened
  under load with explicit coalescing and starvation checks for non-focused
  visible panes
- host-scoped reconnect UX needs continued refinement so a detached mobile view,
  a disconnected transport, and a closed shared runtime tab are clearly
  different user actions

## Acceptance Criteria

The remote lifecycle migration is complete when:

- iOS and desktop GUI render the same server runtime state
- desktop tab/focus changes propagate to iOS
- iOS input/scroll/focus actions affect the same server-owned runtime
- no iOS flow creates, destroys, persists, or chooses a client-owned lifecycle
  object
- remaining visible-tab state is demonstrably transport cache/viewer state only

This acceptance bar is now met for the v1 redesign pass.
