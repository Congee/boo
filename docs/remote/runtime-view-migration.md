# Runtime-View Migration Status

## Goal

Boo remote clients are views/controllers of one authoritative server runtime.
The server owns tabs, panes, focus, status, and terminal content. Desktop GUI
and iOS should observe the same runtime updates instead of creating or selecting
client-owned lifecycle objects.

## Current Product Model

- The Boo runtime server owns tab and pane lifecycle.
- iOS bootstraps from runtime state / active tab metadata.
- iOS does not create or destroy tabs as recovery.
- iOS disconnect closes only the viewer connection, not the server runtime tab.
- Per-client visible-tab ids are transport/viewer bookkeeping for efficient
  terminal streaming, not product state.

## Runtime Payload Contract

`UiRuntimeState` is the bootstrap truth for remote clients:

- active tab index / id
- focused pane id
- tab metadata
- visible pane geometry
- mouse/status/pwd metadata

Tab-list payloads may remain as compatibility metadata, but they are not a
client-owned target selection model.

Terminal full state and deltas should continue moving toward pane-scoped
payloads. Today some streaming caches are keyed by the viewer's visible tab;
that is a transport cache seam, not an independent lifecycle object.

## Semantic Action Contract

Runtime-view clients may send semantic runtime actions such as:

- focus tab / pane
- send input to the focused runtime pane
- send app key / mouse events
- resize viewport
- scroll pane/runtime view
- disconnect the viewer

Explicit create/close-tab messages are not part of iOS recovery. If tab
creation/destruction is exposed to remote clients in the future, it should be a
server-authorized runtime command matching desktop GUI semantics, not a
client-owned lifecycle workaround.

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
- tmux protocol session events
- saved layouts in `src/layout.rs` and the `--layout` flag, which are startup
  layout presets rather than runtime lifecycle objects

These should not be confused with remote runtime-view architecture.

## Remaining Technical Debt

- Keep reducing compatibility tab-list metadata in favor of first-class runtime
  snapshots everywhere.
- Move terminal full-state/delta streaming from visible-tab cache seams toward
  pane-scoped runtime-view payloads.
- Saved layout terminology now uses `src/layout.rs` and `--layout`; it is not
  the remote client lifecycle model.

## Acceptance Criteria

The remote lifecycle migration is complete when:

- iOS and desktop GUI render the same server runtime state
- desktop tab/focus changes propagate to iOS
- iOS input/scroll/focus actions affect the same server-owned runtime
- no iOS flow creates, destroys, persists, or chooses a client-owned lifecycle
  object
- remaining visible-tab state is demonstrably transport cache/viewer state only
