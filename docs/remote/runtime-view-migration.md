# Runtime-View Migration Plan

## Goal

Remove the remote `session` abstraction from Boo's client/server contract and
make remote clients views onto a single authoritative server runtime.

Desired product behavior:

- one Boo server runtime owns tabs, panes, focus, status, and terminal content
- desktop GUI and iOS remote both observe that same runtime
- changes on one view are reflected on the other view
- remote protocol is runtime-centric, not session-attach-centric

## Current Reality

Today, remote `session` is not a `libghostty-vt` concept. It is Boo-owned.
In practice it is currently just the tab identity from `TabManager`.

Code evidence:

- `src/tabs.rs`
  - `Tab { id, tree, ... }`
  - `active_tab_id()` returns the active tab id
  - `find_index_by_tab_id()` finds a tab by that id
- `src/runtime_server.rs`
  - remote create creates a new tab
  - remote destroy removes a tab
  - remote attach targets a tab by `tab_id` internally

So today:

- `tab_id == tab.id` at the transport boundary
- `destroy session == close tab`
- `attach session == attach to one tab`

This is the source of conceptual drift: the real runtime is tabs/panes/focus,
but the remote transport still exposes an extra `session` layer on top.

## What Is Already Shared Correctly

The server already has a richer shared runtime model that is closer to the
correct long-term design:

- `control::UiRuntimeState`
- `control::UiPaneSnapshot`
- `control::UiPaneTerminalSnapshot`
- `control::UiAppearanceSnapshot`

Relevant code:

- `src/runtime_ui.rs`
- `src/control.rs`
- local GUI stream handling in `src/client_gui.rs`

This means Boo already has the foundation for a runtime-view protocol.
The main architectural problem is that iOS is still mostly wired through the
older attached-session terminal protocol.

## Legacy Session Audit

### Category A: Actually tab identity and should be renamed away from `session`

These uses do not represent an independent concept. They are tab ids.

- already migrated in the core Rust transport/runtime path to names like:
  - `active_tab_id`
  - `find_index_by_tab_id`
  - `tab_id_for_pane_id`
  - `RemoteTabInfo`
  - `RemoteTabListSummary`
  - `RemoteCreateSummary.tab_id`
- remaining work in this category is mostly client-side and compatibility-only
  decode surfaces, not the main runtime implementation

These should ultimately become tab/runtime terminology or disappear into the
runtime state model entirely.

### Category B: Legacy transport attachment state that has now been removed from live code

These uses used to track which runtime target a given stream client was
explicitly attached to, plus optional resume/lease metadata.

- removed from live Rust transport/runtime code:
  - `ClientAttachmentLease`
  - `RevivableRuntimeSubscription`
  - explicit remote `Attach` / `Detach` command handling
  - remote attach/resume CLI and direct-client RPCs
- removed from live iOS product code:
  - resume attachment persistence
  - explicit attach bootstrap

What remains is only:
- `ClientRuntimeView`
  - current visible tab id for this viewer stream
  - cached tab-list/runtime/appearance payloads
  - cached terminal full state and pane states
  - latest acknowledged input sequence

That remaining viewer cache is runtime-view transport plumbing, not a
user-visible attach/session model.

### Category C: Obsolete legacy-target-picking model that should be removed

These represent the wrong product abstraction and should be deleted, not merely
renamed.

- iOS or desktop bootstrap that treats a tab list as a pool of candidate
  targets to pick from
- heuristic target selection before runtime state is known
- host-scoped stored target choice as a product concept
- any assumption that a host presents a bag of attachable tabs that mobile
  should choose from before it has seen the runtime state

Examples:

- `ios/Sources/ProtocolClient.swift`
- `listSessions()` as a bootstrap tool in the old client model
- `ios/Sources/Screens.swift` historically carried attachment-driven terminal bootstrap
- `src/client_gui.rs` still refreshes from compatibility tab lists before a
  richer runtime-view model exists

## Target Architecture

### One runtime viewer per client

A remote client should connect to the Boo runtime, not attach through the
legacy session layer.

The runtime viewer should carry:

- runtime state
- pane state / terminal state
- appearance state
- focus state
- status state
- thin viewer-local cache state for incremental streaming

### Runtime state should contain tabs if tabs exist in the UI

The client does not need a separate first-class `RemoteTabInfo` resource that it
attaches to.

Instead, runtime state should include:

- tabs
  - tab id
  - title
  - active flag
  - pane count
- panes
  - pane id
  - geometry for the client's current viewport
  - focused flag
  - tab membership
- global runtime metadata
  - active tab id or index
  - focused pane id
  - pwd/status/search/etc.

### Terminal content should be pane-scoped

Terminal full state / delta should be associated with pane ids, not sessions.

### Client actions should be semantic runtime actions

Preferred actions:

- `focusPane(paneId)`
- `focusTab(tabId)`
- `closeTab(tabId)`
- `createTab(...)`
- `splitPane(paneId, direction)`
- `sendInput(toFocusedPane or paneId, bytes)`
- `resizeViewport(...)`
- `scrollPane(paneId, delta)`

Raw point coordinates should be optional fallback input, not the primary remote
API.

## Runtime Payload Contract

The runtime-view protocol should treat these payloads as first-class:

### `UiRuntimeState`

Authoritative shared runtime metadata for a client viewport:

- `active_tab: usize`
- `focused_pane: u64`
- `tabs: [UiTabSnapshot]`
  - `tab_id: u32`
  - `index: usize`
  - `active: bool`
  - `title: String`
  - `pane_count: usize`
- `visible_panes: [UiPaneSnapshot]`
  - `pane_id: u64`
  - `focused: bool`
  - `frame`
  - split metadata
- `mouse_selection`
- `status_bar`
- `pwd`

This payload is the bootstrap truth for remote clients. Tab-list payloads are
secondary compatibility metadata.

### `UiAppearanceSnapshot`

Authoritative appearance for the current runtime view:

- font families / size
- opacity settings
- terminal colors
- cursor style / blink settings
- tab colors

### Pane terminal payloads

Pane terminal content remains streamed as full-state / delta data. Today this
is still cached per viewer around the tab currently visible in that viewer; the
long-term shape is:

- full state for visible pane terminals
- delta updates for pane terminals
- pane identity carried by pane snapshots/runtime state

## Semantic Action Contract

Preferred runtime-view actions:

- `listTabs`
  - compatibility metadata refresh only
- `focusTab(tabId)`
- `focusPane(paneId)`
- `createTab(cols, rows)`
- `closeTab(tabId)`
- `sendInput(bytes)` to the focused pane in the current runtime view
- `sendAppKey(event)` to the focused pane/runtime
- `sendAppMouse(event)` to the focused pane/runtime
- `resizeViewport(cols, rows)`
- `scrollPane(paneId, delta)` via app mouse wheel events

Temporary viewer-bookkeeping state that still exists:

- current visible tab id for each viewer stream
- per-viewer cached full state / pane state

These are no longer product-level session selection. They only exist to keep
incremental terminal streaming efficient while runtime-view delivery still
tracks what each viewer is showing.

## Why not pure coordinate-driven interaction?

A pure "send tap coordinates and let the server infer everything" model is
possible, but not ideal as the main design:

- server remains authoritative, which is good
- but focus changes would always require a visible round trip
- perceived latency would be worse
- clients already receive pane geometry, so they can send semantic targets like
  `focusPane(paneId)` while the server still validates them

Preferred compromise:

- server computes layout for the client's viewport
- client mirrors that geometry
- client sends semantic intent using pane ids/tab ids
- server remains authoritative and can correct drift

## Migration Strategy

### Phase 1: Naming audit and boundary cleanup

- stop using `session` in new product-facing logic
- document where `session` currently means `tab id`
- isolate transport-only attachment state

Phase 1 output should be:

- no new code uses `session` to mean tab identity
- internal transport state is split into:
  - runtime viewer
  - viewer-local cache state
  - compatibility wrappers only at the boundary
- diagnostics can still expose legacy field names for compatibility, but server
  state should stop mixing them together in one struct

### Phase 2: Runtime-first client bootstrap

- iOS should bootstrap from runtime state, not by picking from a tab list
- local GUI remote path should prefer runtime state too
- tab-list logic becomes compatibility-only metadata, not the source of
  bootstrap truth

Phase 2 protocol shape:

- client connects and authenticates
- server publishes runtime state first-class:
  - active tab
  - focused pane
  - visible pane geometry
  - tab metadata
- tab list may still be published for compatibility, but is not the bootstrap
  driver
- client does not pick from a session pool
- if a client needs a current transport target, it derives it from runtime
  state rather than a list/attach heuristic

Current implementation progress:

- local GUI remote bootstrap now derives its initial target from
  `UiRuntimeState.active_tab` and `UiTabSnapshot.tab_id`
- the runtime server now sends `UiRuntimeState` together with `TabList` during
  remote tab-list requests so clients can bootstrap from runtime state
- the runtime server also publishes runtime state and appearance immediately on
  remote connect, before any explicit target-picking round trip
- iOS has canonical runtime-state decoding and uses the active runtime tab as a
  bootstrap fallback before creating a new tab
- iOS no longer persists a host-scoped preferred tab as a product-level choice;
  reconnect bootstrap now prefers current runtime state
- iOS no longer persists resume-attachment metadata as product state
- remaining visible-tab state is viewer bookkeeping for transport caching, not a
  product-level "session selection" concept

### Phase 3: Pane/runtime action model

- send focus/input/scroll actions against pane or tab ids
- stop using per-viewer tab targeting as the primary interaction model

### Phase 4: Remove session protocol surfaces

When runtime-view tests are green:

- remove `listSessions`
- remove session-list payloads
- remove session attach selection from clients
- rename remaining transport internals to runtime/tab terms or delete them

## Remaining Protocol Surfaces To Remove

These are the main seams where the old session-shaped transport still appears
as historical or migration-only code:

- historical protocol documentation
  - legacy numeric opcodes still carry the old list/attach flow on the wire,
    even though product-facing code is now tab/runtime-oriented
- one-way migration helpers
  - iOS still rewrites legacy persisted `sessionId` / `hostSessions` data into
    canonical `tabId` / `hostTabs` storage on load
- client bootstrap
  - iOS still has compatibility tab-list handling, but production bootstrap is
    now tab-native rather than `listSessions()`-driven
  - local GUI remote stream handling still starts from compatibility tab lists
    instead of first-class runtime snapshots

The next protocol step is to keep shrinking the remaining historical transport
surface while shifting internals and client bootstrap to:

- runtime viewer
- semantic tab/pane actions
- compatibility wrappers only at the wire boundary

## Acceptance Criteria

The migration is successful when:

- desktop GUI and iOS both render the same runtime state
- desktop tab close is reflected on iOS as one fewer tab
- desktop focus changes are reflected on iOS
- iOS pane focus/input/scroll are reflected in desktop view
- no client bootstraps through `listSessions`
- `session` no longer exists as a product-level remote abstraction
