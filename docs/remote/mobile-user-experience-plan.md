# Mobile Remote Terminal UX Plan

Updated: 2026-04-26

## Scope and invariants

This plan turns the current iPad findings plus mobile-terminal research into
actionable Boo work. It is intentionally focused on Boo's remote terminal UI,
not on bundling unrelated terminal-app features.

Hard invariants:

- terminal content has one authoritative state: server/runtime memory
- clients may cache, render, and optimistically show safe view-local intent, but
  must reconcile against server revisions
- clients do not persist independent terminal truth; reconnect/resume means
  reattaching a view and pulling current state from the server
- terminal text prediction remains out of scope until the server-authored state
  path is deterministic under jitter

## 1. Deterministic iOS pane render-state fix

Problem: on a live iPad, typed text can disappear after a keypress and then
reappear, or not, after another key. This points to stale pane state, delta
application, focus-state mirroring, or SwiftUI publication ordering rather than
terminal ownership.

Concrete plan:

- [x] Reproduce with the live iPad lane against a long-running server:
  - start server with `--profiling --trace-filter info`
  - connect iPad to the server
  - type a short deterministic marker one byte/key at a time
  - capture screen recording or screenshots plus server/client logs
- [x] Add pane-state sequence instrumentation:
  - server: pane id, pane revision, runtime revision, view id, focused pane,
    update kind full/delta, latest input sequence
  - iOS: receive order, accepted/rejected revision, delta/full state, rendered
    pane id, focused-pane mirror update, SwiftUI publication revision
- [x] Add stale-state guards on iOS:
  - reject older pane revisions
  - reject same-revision deltas unless explicitly idempotent
  - never let a focused legacy/full-state mirror overwrite a newer pane-specific
    state for the same pane
- [x] Add deterministic recovery:
  - request or wait for full-state refresh after a rejected/missing delta
  - prefer full-state for focused pane after input until ordering is proven
  - keep render-ack tied to the actual rendered pane revision
- [x] Add focused unit/self-test coverage:
  - out-of-order full then delta
  - delta with missing base
  - legacy focused full-state older than pane-specific update
  - focus change while input updates are in flight
- [x] Verify on:
  - iOS simulator metrics lane
  - physical iPad metrics/signpost lane
  - desktop local client path through `cargo check`/existing remote validation

Acceptance:

- typed text never visually disappears after being rendered from an accepted
  server revision
- iOS logs explain every rejected pane update and every full-state recovery
- physical iPad smoke shows stable typed markers under repeated key input

## 2. iOS touch-first terminal gesture grammar

Problem: iOS currently has ad hoc interaction behavior. Pane focus, scroll,
selection, mouse passthrough, and zoom need a predictable grammar that does not
fight the system keyboard or terminal mouse modes.

Concrete plan:

- [x] Define the gesture map in docs and tests:
  - one-finger tap: focus pane and send terminal click when mouse mode requires
  - one-finger drag: text selection by default; mouse drag when explicit mouse
    passthrough mode is active
  - two-finger pan: terminal scroll/copy-mode scroll
  - two-finger tap: configurable quick action, default new tab or command palette
  - pinch: font size zoom
  - long press: selection/menu, with optional mouse passthrough toggle
- [x] Implement one complete gesture router in the iOS terminal surface:
  - pane-local UIKit gesture overlay on terminal canvases
  - explicit recognizers for one-finger tap, two-finger pan/tap, long press, and pinch zoom
  - no duplicate SwiftUI tap/drag focus paths from overlays/statusbar/body
- [x] Make gesture side effects server-semantic:
  - focus pane via runtime action
  - two-finger scroll maps to server terminal wheel events only for the focused pane
  - long press and two-finger tap open compose input instead of sending terminal mouse input
- [x] Add visual feedback:
  - focused pane highlight immediately
  - scroll/copy-mode indicator
  - mouse passthrough indicator
- [x] Verify with XCUITest gestures and physical iPad screenshots.
  - `compare-ios-simulator-ipad-metrics.sh --simulator-only` passes with render/input traces
  - `compare-ios-simulator-ipad-metrics.sh --ipad-only` passes with physical screenshot attachments and non-focused pane tap/focus coverage

Acceptance:

- tapping any visible pane focuses it deterministically
- scrolling does not steal focus
- long press/selection does not accidentally send terminal mouse input
- gesture behavior is documented and test-covered

## 3. Mobile keybar, sticky modifiers, and compose input

Problem: mobile terminals need fast access to Esc, Tab, Ctrl, Alt/Option, Cmd,
arrows, function keys, and safe text composition. Boo's current iOS input path is
too minimal for real terminal work.

Concrete plan:

- [x] Add a compact configurable keybar above the keyboard:
  - default keys: Esc, Tab, Ctrl, Alt/Option, Cmd, arrows, `/`, `-`, `~`, `|`
  - iPad layout can expose more keys than iPhone
  - hardware keyboard mode can collapse or hide the bar
- [x] Add sticky modifier semantics:
  - single tap = one-shot modifier
  - double tap = locked modifier
  - visible state for one-shot/locked modifiers
  - clear one-shot modifiers after the next key event
- [x] Add full function-key access:
  - Fn layer or horizontal keybar page for F1-F12
  - send terminal key specs through the existing remote key path
- [x] Add compose/draft input:
  - opens a native text editor overlay
  - supports autocorrect, dictation, and CJK composition
  - sends the final text as terminal input only on explicit Send
- [ ] Add user configuration later, after defaults stabilize:
  - key order
  - hidden keys
  - per-device presets

Acceptance:

- common terminal actions are reachable without an external keyboard
- sticky Ctrl/Alt/Cmd works for one-shot and locked sequences
- compose overlay can send CJK/dictated/multiline input without corrupting the
  terminal state

## 4. Connection health and remote debug HUD

Problem: high RTT and jitter are unavoidable on iPad/Wi-Fi. Users need to know
whether lag is network, server, render, or app scheduling, and developers need
the same facts without digging through traces.

Concrete plan:

- [x] Add a small optional connection health HUD:
  - connection status
  - heartbeat RTT p50/p95/latest
  - action-ack RTT latest
  - render-ack lag/latest pane revision
  - stale view age
  - reconnect count
- [x] Add developer detail mode:
  - server instance id
  - view id
  - viewed tab/focused pane
  - last pane revision per visible pane
  - queue depth/coalescing/starvation counters
- [x] Keep HUD data passive:
  - no terminal content prediction
  - no client-authored runtime truth
  - all state reconciles with server diagnostics
- [x] Export health snapshots with existing metrics artifacts.
- [x] Show user-facing banners only for actionable states:
  - reconnecting
  - detached view
  - expired view/server has no tabs
  - high jitter/degraded link

Acceptance:

- when iPad latency spikes, the UI can show whether RTT/action ack/render ack is
  the bottleneck
- metrics artifacts include the same health fields visible in the HUD
- debug HUD does not disturb terminal input or rendering cadence

## 5. Reattach/resume UX from server memory

Problem: users can exit a shell, background the app, lose the connection, or
reopen Boo and see confusing states such as "no active tab". Boo does not need
client-side terminal persistence; it needs clear reattachment to the
server-owned runtime state.

Concrete plan:

- [x] Define user-facing states:
  - connecting
  - attached to live view
  - detached view can reattach
  - server reachable but view expired
  - server reachable but has zero tabs
  - viewed tab exited
  - server unreachable
- [x] On reconnect, pull current server runtime state:
  - server tabs/panes/layout
  - existing runtime views, if still alive
  - latest full pane states for visible panes
  - server diagnostics for why previous view cannot attach
- [x] Offer explicit actions:
  - Reattach previous view
  - Open first/new tab
  - Pick an existing tab
  - Disconnect
- [x] Keep passive connections passive:
  - do not create tabs just by connecting
  - only explicit interactive actions create new runtime state
- [x] Add deterministic tests:
  - app reconnects while server still has the tab via `testReconnectAndTypeAgainAfterBackNavigation` and `testFastSwipeBackAndReconnectStress`
  - app reconnects after view timeout but tab still exists via runtime-view health reducer coverage
  - app reconnects after shell/tab exited via active-tab health reducer coverage
  - app reconnects to server with zero tabs via expired/no-active-tab reducer coverage
  - two clients reconnect/view different tabs through the macOS+iPad two-client runtime-view smoke lane

Acceptance:

- reconnect never depends on client-persisted terminal content
- every empty/detached/no-active-tab state has a clear reason and action
- clients can pull enough full state from server memory to render immediately
