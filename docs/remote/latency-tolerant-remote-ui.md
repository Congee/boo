# Latency-Tolerant Remote UI Architecture

## Problem

Boo remote must remain usable when the network has high RTT, high jitter, or
short stalls. Recent iPad-on-LAN measurements showed `remote.heartbeat_rtt`
spikes in the hundreds of milliseconds. That is a link-quality fact Boo must
survive, not a number every UI interaction can wait on.

Therefore:

- heartbeat RTT is link-quality telemetry, not the primary user-perceived
  latency metric
- local intent feedback must not wait for heartbeat RTT
- server/runtime state remains authoritative for tabs, panes, terminal content,
  and layout truth
- clients may optimistically display safe view-local intent, then reconcile with
  server acknowledgements and revisions
- terminal text/content prediction is explicitly deferred until safer UI-level
  work and QoS are measured

## Research Findings Applied to Boo

### Mosh: synchronize state, not stale byte streams

Mosh models the terminal screen as synchronized state, speculatively echoes safe
input, and adapts output rate to network conditions. The relevant lesson for Boo
is to fast-forward clients toward the newest useful terminal/view state instead
of insisting every obsolete intermediate update reaches a slow client.

Boo action:

- coalesce superseded pane updates when newer state exists
- keep terminal content server-authored for now
- consider terminal text prediction only after action acknowledgements,
  optimistic view-local UI, and QoS prove insufficient

### THINC: real-time interaction updates outrank passive display work

THINC distinguishes real-time interaction updates from ordinary display traffic
and gives input-driven updates priority. Boo should treat a focused pane and the
state caused by recent user intent as a higher-priority stream than passive
visible panes.

Boo action:

- prioritize action acknowledgements and focused visible pane updates
- coalesce non-focused visible pane updates by pane
- keep starvation guards so visible non-focused panes do not freeze forever

### RFB/VNC: pull gives backpressure, pure pull costs RTT

RFB's framebuffer update request model makes client demand explicit, which is a
natural backpressure signal. Pure request/response would be too RTT-bound for
Boo, but the idea is useful: the server should know what the client has actually
accepted or rendered.

Boo action:

- keep push for active interaction
- add action/render acknowledgement metrics
- use queue-depth/render feedback to reduce passive update pressure under jitter

### RDPGFX and dynamic channels: acknowledge frames and separate concerns

RDP graphics protocols include frame acknowledgements and queue-depth feedback;
dynamic virtual channels provide separate logical channels. Boo already has
logical channel classification in the wire layer, but currently carries the
QUIC path over one bidirectional stream.

Boo action:

- add explicit action IDs and acknowledgements first
- add pane-aware scheduling on the current transport first
- evaluate multi-stream QUIC only after the simpler scheduling/reconciliation
  model is measured

### QUIC: multiplexing helps only when the app uses it deliberately

QUIC avoids TCP-style stream-level head-of-line blocking across independent
streams, but a single stream still serializes unrelated Boo frames. QUIC loss
recovery also rewards pacing instead of bursty writes.

Boo action:

- do not expect QUIC alone to solve bad Wi-Fi jitter
- pace bulk/passive pane output
- keep future stream split candidates explicit:
  - control/input/health
  - runtime metadata
  - focused pane
  - passive panes

### QUIC DATAGRAM: only for staleable, non-authoritative data

QUIC DATAGRAM is useful for data where retransmission is worse than loss. Boo's
authoritative terminal state should not move there first.

Boo action:

- do not send authoritative terminal content unreliably
- consider unreliable/staleable delivery later for transient cursor, hover,
  gesture-preview, or low-value visual hints

### Apple Network.framework: transport progress must not depend on UI work

`NWConnection` receives events on a queue chosen by the app. Boo's iOS
transport should not have heartbeat, read loop, frame decode, or protocol
progress owned by the SwiftUI MainActor, because screenshots, accessibility
snapshots, keyboard work, or layout can stall it.

Boo action:

- split the iOS client into an off-main transport/decoder owner and a MainActor
  view model
- MainActor publishes reduced state into SwiftUI only
- heartbeat/no-op/action ack progress must continue while UI is busy

### Remote desktop practice: adapt to jitter and degrade passive fidelity first

Remote display systems such as PCoIP emphasize jitter, bandwidth variation, and
adaptive behavior. For Boo's terminal UI, interaction fidelity matters more than
perfectly fresh passive panes under a bad link.

Boo action:

- degrade passive pane freshness before focused-pane/input responsiveness
- expose link quality separately from interaction latency
- show poor-link/recovering UI when needed rather than blocking local intent

## Target Architecture

### 1. Action acknowledgements and no-op baseline

Add a backward-compatible runtime-action envelope:

- `client_action_id: u64`
- `action: RuntimeAction`

The server must continue accepting legacy bare `RuntimeAction` payloads.

Add:

- `RuntimeAction::Noop { view_id }`
- action acknowledgement metadata in `UiRuntimeState` or equivalent runtime-view
  state
- metrics for no-op roundtrip, action acknowledgement, optimistic apply, and
  reconciliation

The no-op metric becomes Boo's minimal protocol roundtrip baseline. Heartbeat
RTT remains a health/link-quality signal.

### 2. iOS transport off MainActor

Split current iOS remote client responsibilities:

- off-main transport owner:
  - `NWConnection`
  - heartbeat send/ack/timeout
  - frame read/write
  - wire decode and validation
  - action/no-op timing
- MainActor view model:
  - reduced runtime state publication
  - pane state publication into SwiftUI
  - optimistic UI overlay/reconciliation state

Transport tests should use deterministic protocol state, not sleeps, to verify
readiness and progress.

### 3. Safe optimistic UI

Optimistically apply only view-local visual intent:

- focus pane ring and focused-pane metadata display
- viewed tab / statusbar active tab display
- next/previous tab selection highlight
- split resize handle geometry while dragging
- pending-input / poor-link indicators

Every optimistic mutation is tagged with `client_action_id` and cleared when an
authoritative ack/revision arrives. If the authoritative state conflicts, roll
back to server state.

Do not predict arbitrary terminal text in this phase.

### 4. Pane-aware QoS and backpressure

Replace single latest-screen coalescing with per-client scheduling:

1. health/control/action acknowledgements
2. focused visible pane updates for that client
3. runtime/view metadata needed to reconcile UI
4. non-focused visible pane updates, coalesced by pane
5. background/passive work, if any

Add starvation protection for non-focused visible panes and feedback metrics for
queue depth, coalesced updates, render acknowledgements, and skipped passive
updates.

### 5. Future transport separation

After action acknowledgements, optimistic UI, off-main iOS transport, and
pane-aware QoS are measured, evaluate QUIC multi-stream transport. Do not make
multi-stream QUIC the first fix.

Candidate streams:

- control/input/health
- runtime metadata
- focused pane
- passive panes

Candidate unreliable delivery is limited to staleable transient UI, not terminal
authority.

## Actionable Checklist

### Measurement and acknowledgements

- [x] Add backward-compatible runtime-action envelope with `client_action_id`.
- [x] Continue accepting legacy bare `RuntimeAction` payloads.
- [x] Add `RuntimeAction::Noop { view_id }`.
- [x] Add action acknowledgement metadata to runtime-view state.
- [x] Add `remote.noop_roundtrip` metric.
- [x] Add `remote.action_ack` metric.
- [x] Add `remote.optimistic_apply` metric.
- [x] Add `remote.reconcile` metric.
- [x] Update simulator+iPad comparison output to list no-op/action metrics
      separately from `remote.heartbeat_rtt`.

### iOS transport isolation

- [ ] Move `NWConnection`, heartbeat, frame IO, and decode off MainActor.
- [ ] Keep MainActor responsible only for reduced state publication into SwiftUI.
- [ ] Verify heartbeat/no-op ack can progress while UI/AX work is busy.
- [ ] Replace sleep-based readiness checks with deterministic protocol-state
      waits.

### Safe optimistic UI

- [x] Optimistically apply focus-pane UI state immediately.
- [x] Optimistically apply viewed-tab/statusbar selection immediately.
- [ ] Optimistically apply split-resize handle geometry while dragging.
- [x] Tag optimistic focus/viewed-tab state with `client_action_id`.
- [x] Clear optimistic focus/viewed-tab state on matching server ack/revision.
- [x] Roll back optimistic focus/viewed-tab state on conflicting
      authoritative state.
- [x] Keep terminal text output server-authored.

### Pane-aware QoS and backpressure

- [ ] Replace single latest-screen coalescing with pane-aware scheduling.
- [ ] Prioritize health/control/action ack frames.
- [ ] Prioritize focused visible pane updates per client.
- [ ] Coalesce non-focused visible pane updates by pane.
- [ ] Add starvation guard for non-focused visible panes.
- [ ] Add queue-depth/render-ack feedback for passive pane throttling.
- [ ] Preserve per-client priority differences when clients view/focus different
      tabs or panes.

### Future transport separation

- [ ] Evaluate multi-stream QUIC only after action ack, off-main iOS transport,
      optimistic UI, and QoS metrics are available.
- [ ] Evaluate unreliable delivery only for staleable transient UI state.
- [ ] Keep authoritative terminal state on reliable state/delta paths.

## Validation Expectations

- Pure Rust tests cover runtime action envelope decode, legacy action decode,
  no-op ack state, and QoS scheduling order.
- Swift/iOS tests cover optimistic state apply/reconcile and off-main heartbeat
  or no-op progress while MainActor is busy.
- E2E metrics include simulator and iPad no-op/action/render measurements.
- High `remote.heartbeat_rtt` must not fail focus/tab/statusbar local feedback
  acceptance by itself.
- Tests must prefer deterministic state waits over sleeps/timeouts unless timing
  behavior itself is under test.

## References

- [Mosh paper](https://mosh.org/mosh-paper.pdf)
- [THINC: A Remote Display Architecture for Thin-Client Computing](https://www.cs.cornell.edu/courses/cs614/2005fa/papers/THINC.pdf)
- [RFC 9000: QUIC](https://www.rfc-editor.org/rfc/rfc9000)
- [RFC 9002: QUIC Loss Detection and Congestion Control](https://www.rfc-editor.org/rfc/rfc9002)
- [RFC 9221: QUIC DATAGRAM](https://www.rfc-editor.org/rfc/rfc9221)
- [RFC 6143: The Remote Framebuffer Protocol](https://www.rfc-editor.org/rfc/rfc6143)
- [Microsoft RDPGFX](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/)
- [Microsoft RDP dynamic virtual channels](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/3bd53020-9b64-4c9a-97fc-90a79e7e1e06)
- [Apple Network.framework NWConnection](https://developer.apple.com/documentation/network/nwconnection)
- [HP Anyware network requirements](https://anyware.hp.com/products/hp-anyware/2024.07/documentation/session-planning-guide/network-and-system-requirements)
