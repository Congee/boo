# Boo Remote Requirements

Structured remote docs now live under [`docs/remote/`](./docs/remote). This
root file remains the detailed product contract; start with
[docs/remote/requirements.md](./docs/remote/requirements.md) if you want the
docs-tree entrypoint.

Date: April 17, 2026

This document defines the product requirements for remote Boo.

Remote Boo still has two important client modes:

- desktop-to-desktop remote Boo
- iOS/mobile remote Boo

But those modes should converge on one canonical Boo-native remote transport over time. The split in
this document is about client UX and rollout sequencing, not about keeping two permanent remote
products with incompatible contracts. Boo already has partial implementations for both, so the goal
here is to refine the direction rather than replace it.

## Current Repo State

Boo already has two remote foundations:

- SSH-backed remote desktop bootstrap and socket forwarding
  - see [REMOTE_SSH_INTEGRATION_PLAN.md](./REMOTE_SSH_INTEGRATION_PLAN.md) for the current SSH desktop milestone status and implementation notes
- a Boo-native remote daemon and iOS client
  - see [FEATURES.md](./FEATURES.md)
  - see [ios/](./ios)
  - see [scripts/test-ios-remote-view.sh](./scripts/test-ios-remote-view.sh)

The current repo already supports:

- `boo --host <ssh-host>` style desktop remote work in first-cut form
- `boo --headless --remote-port <port>` for a Boo-native TCP daemon
- a native SwiftUI iOS client that speaks the Boo remote wire protocol

The requirements below assume those foundations remain.

## Product Split

Remote Boo shall be treated as two concrete client modes sharing one long-term remote contract:

1. Desktop Remote Mode
   A desktop Boo client controls another desktop Boo instance, primarily over SSH.

2. Mobile Remote Mode
   The iOS app controls a remote Boo instance, with requirements shaped by mobile networking, touch input, sleep/resume, and constrained screen space.

There shall be one canonical protocol and session model for both modes, while transport rollout and
UX details can still differ during implementation.

## Design Principles

- Reuse Boo's existing session, pane, and terminal-state model.
- Keep the server authoritative for PTYs, tabs, splits, and terminal state.
- Preserve deterministic, socket-level verification paths.
- Prefer secure bootstrap and authentication by default.
- Do not expose raw control surfaces to hostile networks without explicit hardening.
- Avoid making the desktop SSH path wait on mobile-specific transport work.

## Verification Principles

Remote Boo shall be designed so its critical behaviors can be verified directly and deterministically.

The primary verification rules are:

- prefer protocol-level and socket-level checks over focus-sensitive GUI automation
- verify bootstrap, attach, and stream health without requiring manual observation when possible
- preserve app-targeted verification paths for both desktop and iOS modes
- treat visual/manual verification as necessary for touch UX and human-facing state, not as the primary proof of transport correctness

The remote product shall expose enough observable state to verify:

- bootstrap success
- connection health
- protocol compatibility
- session visibility
- attach/resume state
- transport degradation and reconnect behavior

The verification strategy shall include three layers:

1. Protocol-level verification
   - handshake
   - auth
   - capability/version negotiation
   - heartbeats
   - reconnect/resume metadata

2. End-to-end transport verification
   - bootstrap works
   - session list works
   - attach works
   - input and resize work
   - full-state and incremental updates work

3. Manual UX verification
   - touch interaction
   - keyboard ergonomics
   - mobile sleep/wake behavior
   - visible connection state and degraded-mode UX

## Mode 1: Desktop Remote Requirements

### Primary User Story

The user runs:

```bash
boo --host <ssh-host>
```

and gets a local native Boo UI attached to a remote Boo session owner.

### Transport

Desktop remote shall use SSH as the default transport and security boundary.

The current first-cut implementation remains:

- bootstrap the remote Boo server over SSH
- forward Boo's control socket
- forward Boo's `.stream` socket
- connect the existing local GUI/client to the forwarded local sockets

This matches Boo's current architecture and avoids rewriting the client around the TCP daemon.

However, this SSH-forwarded-socket design should be treated as the first milestone, not necessarily the final unified transport.

If Boo converges on one remote solution for both desktop and iOS, the likely long-term shape is:

- SSH for bootstrap, identity, and optional deployment convenience
- then upgrade to a Boo-native remote transport over one network connection

This is the same broad pattern used by `tssh` and `tsshd`:

- authenticate and bootstrap through SSH
- start a remote helper on the server
- exchange connection parameters and short-lived secrets over the SSH channel
- then move the session to a transport designed for better latency and reconnect behavior

### Functional Requirements

- The remote host shall run a compatible Boo server binary.
- Boo shall auto-bootstrap the remote server when needed.
- Boo shall reuse existing SSH connections when practical.
- Boo shall forward both:
  - control RPC socket
  - live `.stream` socket
- The local GUI shall behave like a native Boo client, not like a dumb terminal relay.
- The user shall be able to:
  - list sessions
  - create sessions
  - attach to sessions
  - resize panes/windows
  - send input
  - receive live terminal updates

### Configuration Requirements

Desktop remote shall support:

- `--host`
- `--socket`
- `remote-host`
- `remote-workdir`
- `remote-socket`
- `remote-binary`

The following must be handled correctly:

- host-specific local forwarded socket naming
- remote path expansion for `~` and `$HOME`
- remote binary not being present in `PATH`
- explicit local socket override via `--socket`
- version mismatch detection between local and remote Boo

### UX Requirements

- The common path shall not require the user to manually invoke `ssh`.
- Errors shall be host-oriented and actionable:
  - SSH failed
  - remote binary missing
  - remote socket unavailable
  - version mismatch
  - authentication failure
- The UI shall make it clear when the user is attached to a remote host.
- Reconnect behavior shall be defined for:
  - tunnel drop
  - server restart
  - laptop sleep/resume

### Security Requirements

- SSH host verification and authentication remain the default trust boundary.
- The remote Boo server should not need to listen on a public TCP port for desktop remote mode.
- Local forwarded sockets shall be cleaned up safely and deterministically.

### Verification Requirements

Desktop remote shall be verifiable through direct, non-visual checks:

- bootstrap succeeds
- forwarded local control socket works
- forwarded local `.stream` socket works
- `get-ui-snapshot` works through the tunnel
- session creation works through the tunnel
- live stream attach/update works through the tunnel

## Mode 2: Mobile Remote Requirements

### Primary User Story

The user opens Boo on iPhone or iPad and attaches to a remote Boo session owner to monitor and control a desktop system.

This shall work both:

- on a local network
- across less reliable networks when productized for remote-from-anywhere use

### Existing Foundation

The repo already includes:

- Boo-native TCP remote daemon support
- Bonjour advertisement on `_boo._tcp`
- HMAC challenge/response support
- a SwiftUI iOS client under [ios/](./ios)
- an automated validation script in [scripts/test-ios-remote-view.sh](./scripts/test-ios-remote-view.sh)

This is the correct foundation for mobile mode.

Mobile mode should not be reduced to "just run SSH inside the iOS app" unless that becomes a deliberate product direction later.

### Functional Requirements

- The iOS app shall browse and connect to compatible Boo servers.
- The user shall be able to:
  - discover hosts on LAN
  - connect to saved hosts manually
  - authenticate securely
  - list sessions
  - attach/detach
  - resize
  - send text and key input
  - observe terminal updates in real time
- The app shall maintain connection history and saved nodes.

### Mobile-Specific Requirements

- The app shall tolerate:
  - app background/foreground transitions
  - temporary connectivity loss
  - IP/network changes
  - device sleep/wake
- The app shall provide mobile-appropriate input UX:
  - software keyboard support
  - hardware keyboard support
  - touch gestures for navigation and selection
  - missing modifier/function key affordances
- The app shall provide a readable, responsive terminal presentation on small screens.

### Transport Requirements

The current Boo-native TCP daemon is sufficient for LAN and development validation.

However, if Boo is meant to support remote-from-anywhere iOS access, it shall use the unified Boo-native transport defined below rather than relying on an exposed plain TCP port plus shared secret alone.

### Unified Transport Direction

If Boo chooses one remote solution that must work for both desktop and iOS, the canonical direction shall be:

1. SSH remains available as a bootstrap path, especially on desktop.
2. The canonical remote session protocol becomes a Boo-native network transport.
3. That Boo-native transport supports a single remote endpoint and a single client contract for:
   - desktop Boo
   - iOS Boo
4. SSH-forwarded Unix sockets become an implementation bridge for early desktop milestones, not the permanent product contract.

In practice, Boo shall use a transport model inspired by `trzsz-ssh` / `tsshd`:

- initial bootstrap/auth over SSH
- remote helper startup on the server
- protocol upgrade from SSH bootstrap to a Boo-native session transport
- reconnect and roaming support designed into the transport itself

That is a better fit for a unified desktop+iOS story than keeping the remote contract permanently tied to forwarded Unix sockets.

### Canonical Network Transport

The canonical Boo remote transport shall be:

- one Boo-native encrypted network connection per client session
- QUIC as the preferred live-session transport
- able to fall back to a TCP/TLS transport when UDP is unavailable or blocked

This means:

- QUIC is the preferred live-session path because it supports connection migration and better roaming behavior
- a TCP/TLS fallback is required for restricted environments
- desktop and iOS clients shall share the same application-level remote protocol regardless of the underlying transport

The protocol shall not require Unix socket forwarding as part of the permanent client contract.

### Bootstrap Requirements

Bootstrap shall work as follows:

- desktop clients may bootstrap through SSH
- the SSH bootstrap path shall:
  - authenticate the user
  - verify the remote host using SSH's trust model
  - start or locate the remote Boo helper/server
  - exchange the remote endpoint and short-lived connection credentials
  - upgrade the session from SSH bootstrap to the Boo-native network transport
- iOS clients shall connect directly to the Boo-native network transport and shall not depend on Unix socket forwarding

### Deployment Modes

Boo shall support two deployment modes for the same remote protocol:

1. LAN Mode
   - Bonjour discovery may be used
   - direct connection to the Boo-native remote endpoint is allowed

2. Internet Mode
   - the same Boo-native remote protocol is used
   - desktop clients may still use SSH bootstrap
   - iOS clients connect directly using the server's published endpoint

The protocol and client contract shall be the same in both modes. Discovery and bootstrap may differ.

### Security Requirements

The current HMAC challenge/response mechanism is useful, but it is not by itself a complete internet-facing product story.

For production-grade remote iOS access, the system shall define:

- server identity verification
- per-user authentication or equivalent strong credentials
- transport encryption
- host exposure guidance
- recovery and revocation story

### Reliability Requirements

Mobile mode and the unified remote transport shall use the following reconnect model:

- a short heartbeat timeout to detect path failure quickly
- automatic reconnect attempts after heartbeat failure
- connection migration support across sleep/wake and network switching
- a bounded reconnect window after which the session is considered lost
- explicit session resumption metadata so a client can reattach to the same remote session when reconnect succeeds
- user-visible state transitions for:
  - connected
  - reconnecting
  - degraded
  - disconnected

The product shall not depend on long default TCP timeouts to detect failure.

Mosh is the reference class for this kind of resilience. Boo does not need to copy Mosh exactly, but it should explicitly decide which mobile-network problems it does and does not solve.

The `tssh` / `tsshd` pair adds a more directly relevant reference for Boo because it keeps SSH in the story while still optimizing the live session transport. Boo should take explicit inspiration from these properties:

- connection migration
  - session survives client sleep/wake and network switching
- heartbeat-based failure detection
  - detect broken paths quickly without waiting for very long TCP timeouts
- bounded reconnect windows
  - support reconnect for a defined period, then fail decisively
- transport selection by environment
  - faster transport by default
  - lower-latency option when desired
  - TCP fallback when UDP is blocked
- user-visible connection state
  - clear notification when reconnecting, degraded, or disconnected

These requirements matter for both iOS and desktop laptops that roam across networks.

### Timeout Policy

The remote transport shall define explicit time windows:

- auth challenge window:
  - 10 seconds on the native daemon before an unanswered HMAC challenge expires
- direct-client heartbeat window on the native daemon:
  - 20 seconds before a direct authenticated client is expired for missing heartbeats
- iOS heartbeat cadence:
  - send heartbeat every 5 seconds
- reconnect notification threshold:
  - surface degraded state after 8 seconds without a heartbeat acknowledgement
- reconnect deadline:
  - treat the connection as lost after 15 seconds without a heartbeat acknowledgement
  - after loss, iOS retries every 2 seconds for up to 5 attempts before declaring reconnect failure
- session revival window:
  - the server keeps resumable attachment state for 30 seconds after disconnect

The exact numbers may be tuned in implementation, but the product requirement is that all four windows exist and are explicit in the transport design.

### Verification Requirements

Mobile mode shall remain verifiable with deterministic automation where possible:

- discovery
- auth
- session listing
- attach
- resize
- terminal-state updates

Manual validation shall still be required for:

- touch UX
- keyboard accessory UX
- iOS permission prompts
- background/foreground behavior
- real-world poor-network behavior

## Shared Requirements Across Both Modes

These requirements apply regardless of transport.

### Session Model

- The server remains authoritative for sessions, tabs, splits, panes, and PTYs.
- Clients may attach and detach without destroying server-owned sessions.
- Session identity shall remain stable enough for reconnect/resume where supported.

### Protocol

- The protocol shall support:
  - session listing
  - attach/detach
  - resize
  - input
  - full-state bootstrap
  - incremental updates/deltas
- Control-style RPCs and live stream traffic shall remain separated, either logically or physically.

If Boo moves toward one canonical remote transport, the protocol should support multiplexed logical channels over one physical connection. At minimum, the connection should carry:

- control RPCs
- session and terminal-state stream updates
- input and resize events
- health/heartbeat traffic
- reconnect or resume metadata

This keeps the current Boo split between control and stream semantics while allowing a single network endpoint for both desktop and iOS.

For the first unified transport iteration, channel multiplexing shall use the existing Boo frame
header rather than introducing a second nested envelope format:

- keep the outer frame format:
  - 2-byte magic `GS`
  - 1-byte message type
  - 4-byte little-endian payload length
- logical channel membership is determined by message type family
- channel families are:
  - control:
    - `Auth`, `AuthChallenge`, `AuthOk`, `AuthFail`
    - `ListSessions`, `SessionList`
    - `Create`, `SessionCreated`
    - `Destroy`, `SessionExited`
    - `ErrorMsg`
  - session stream:
    - `Attach`, `Attached`
    - `Detach`, `Detached`
    - `FullState`, `Delta`, `ScrollData`
    - `UiRuntimeState`, `UiAppearance`, `UiPaneFullState`, `UiPaneDelta`
  - input/control-plane actions:
    - `Input`, `Key`, `Resize`
    - `ExecuteCommand`
    - `AppAction`, `AppKeyEvent`, `AppMouseEvent`, `FocusPane`
  - health:
    - `Heartbeat`, `HeartbeatAck`

This means the first unified transport keeps logical multiplexing without adding an explicit channel
id field. If later transports need independent congestion control or priorities per channel, Boo may
add a new envelope version then, but the first unified transport should not block on that.

### Current Native Handshake Wire Format

Until the unified transport introduces channel multiplexing, the native Boo daemon handshake shall
be documented as follows:

- every frame starts with:
  - 2-byte magic: `GS`
  - 1-byte message type
  - 4-byte little-endian payload length
- client auth bootstrap:
  - client sends `Auth` with an empty payload
  - auth-required server replies with `AuthChallenge` carrying 32 random bytes
  - authless server replies with `AuthOk`
  - auth-required client replies with `Auth` carrying `HMAC-SHA256(challenge, auth_key)`
- `AuthOk` payload layout:
  - `u16` protocol version
  - `u32` capability bits
  - `u16` build-id byte length followed by UTF-8 build-id bytes
  - `u16` server-instance-id byte length followed by UTF-8 bytes
  - `u16` server-identity-id byte length followed by UTF-8 bytes
- `Attach` payload layout:
  - `u32` session id
  - optional `u64` attachment id
  - optional `u64` resume token
- `Attached` payload layout:
  - `u32` session id
  - optional `u64` attachment id
  - optional `u64` resume token
- `Heartbeat` / `HeartbeatAck` payload:
  - opaque client token bytes echoed unchanged by the server

### Security Model

The canonical Boo-native remote protocol shall require:

- encrypted transport
- server identity verification
- replay-resistant session/bootstrap credentials
- per-user or per-device authentication stronger than a shared static HMAC secret alone

SSH remains an acceptable bootstrap trust model for desktop clients, but the Boo-native transport itself shall still provide its own secure session establishment story for direct clients such as iOS.

#### Transport encryption and identity pinning

The encrypted transport is TLS 1.2/1.3 (and QUIC, when that backend lands, via its built-in TLS 1.3). The server generates an ed25519 keypair and self-signed X.509 certificate on first start, persisted at `~/.config/boo/remote-daemon-identity/{key.pem,cert.pem}` with the private key at `0600`. There is no CA, no chain, and no user-provided cert material.

The daemon identity string is `base64url-nopad(sha256(SubjectPublicKeyInfo_DER))`. It is:

- a cryptographic pin anchor: presenting this identity over TLS means presenting the one cert whose SPKI hashes to that value
- stable across cert re-issuance from the same keypair
- algorithm-agnostic at the wire level (the string alone carries no algorithm tag, but the stored private key format does, so PQC migration replaces cert and key together and the identity string recomputes)

Clients bootstrap trust in one of two ways:

1. **SSH-bootstrapped** (desktop): SSH authenticates the user and verifies the host via `~/.ssh/known_hosts`. The client then asks the forwarded Boo control socket for `server_identity_id` and uses it as the SPKI pin for a direct TLS connection. No TOFU window: SSH already vouched.
2. **TOFU** (iOS and direct CLI use): the client opens TLS, the cert is accepted on first contact, the identity string is stored (e.g. `ConnectionStore.trustedServerIdentities`), and every subsequent connect verifies the same pin.

Third parties who deploy Boo daemons behind their own PKI may want a `--cert-path`/`--key-path` escape hatch so rustls uses the caller-provided cert instead of the generated self-signed one. This is a future addition; the pinning model above keeps working unchanged for clients that pin the deployed cert's SPKI.

The client-side verifier (`PinnedSpkiServerCertVerifier` in Rust, its iOS counterpart on the other side) ignores CA chain validation and hostname verification entirely. Cert expiry dates are not consulted either — a pin is an end-entity trust statement, not a chain-and-expiry delegation.

### End-User Versus Debug Diagnostics

Remote Boo shall separate user-facing connection state from debug/operator diagnostics.

End-user surfaces should show:

- remote host or endpoint label
- high-level connection state:
  - connected
  - bootstrapping
  - reconnecting
  - degraded
  - disconnected
- whether recovery is attempting to resume a specific session
- actionable failure text when recovery cannot proceed

Debug/operator surfaces may additionally show:

- protocol version
- capability bits
- build id
- daemon identity and instance ids
- per-daemon client and revive counts
- heartbeat RTT and age
- heartbeat expiry / overdue state
- attachment ids and resume-token presence
- auth challenge age / expiry

Raw tokens, auth keys, and other secret material shall not be displayed in either surface.

### Session Resume Rules

The protocol shall support resumable attachment semantics:

- a remote session has a stable identity
- a client attachment has a resumable identity for a bounded time
- reconnecting clients may resume instead of creating a new session when the server still holds resumable state
- session resume shall not duplicate panes or create phantom attached clients

### Compatibility

- Client/server compatibility rules shall be explicit.
- At minimum, Boo shall detect and surface incompatible remote versions clearly.

### Observability

- Remote connection state shall be inspectable and debuggable.
- There shall be deterministic checks for:
  - control path health
  - stream path health
  - session visibility
  - attach state

## Existing Solutions And What Boo Should Learn From Them

### WezTerm

WezTerm's SSH domains are the closest model for desktop remote Boo:

- remote server/multiplexer on the far host
- bootstrap over SSH
- local native UI attached to the remote domain

Key lesson:

- Desktop remote Boo should feel like connecting to a domain, not like manually creating a tunnel.

### VS Code Remote SSH

VS Code installs a remote server and then runs all client/server traffic through authenticated SSH.

Key lesson:

- remote bootstrap, version management, and clear connectivity requirements matter as much as the core protocol

### Kitty and Ghostty

Kitty's `ssh` kitten and Ghostty's SSH shell integration focus on remote ergonomics:

- environment propagation
- terminfo handling
- connection reuse
- remote control forwarding

Key lesson:

- the "small integration details" around remote sessions matter a lot in practice

### tmux

tmux control mode exposes a simple programmatic remote-control surface while leaving transport out of scope.

Key lesson:

- Boo should keep its protocol inspectable and scriptable, not just GUI-oriented

### Mosh, Blink, and Termius

These are the clearest references for mobile expectations:

- roaming tolerance
- suspend/resume tolerance
- local/mobile UX
- saved hosts and quick reconnect

Key lesson:

- mobile remote access is a different problem from desktop SSH, especially on unreliable networks

### trzsz-ssh and tsshd

These are especially relevant because they combine SSH bootstrap with a transport designed for better live-session behavior.

Important ideas Boo should learn from:

- SSH can remain the trust/bootstrap layer without being the only session transport.
- A remote helper can be launched over SSH and then upgraded into a better live transport.
- Reconnect, roaming, and sleep/wake resilience should be first-class design goals.
- Transport choice may need to vary by environment:
  - QUIC-like default for throughput and migration
  - lower-latency mode where needed
  - TCP fallback when UDP is blocked
- Timeouts should be explicit and product-shaped:
  - heartbeat timeout
  - reconnect timeout
  - maximum keepalive/revival window

Boo does not need to copy QUIC or KCP specifically, but it should adopt the principle that a unified remote product needs transport-level resilience instead of relying only on a long-lived plain TCP stream.

## Recommended Roadmap

### Phase 1: Finish Desktop SSH Mode

- complete `boo --host`
- finish remote path expansion and error handling
- add version mismatch checks
- add reconnect/tunnel lifecycle polish
- verify on Linux ↔ macOS and desktop ↔ desktop flows

### Phase 2: Implement Unified Remote Transport

- implement the canonical Boo-native remote transport
- implement channel multiplexing for control, stream, input, and health
- implement the reconnect and resume model defined above
- implement heartbeat and timeout handling
- implement SSH bootstrap-to-native upgrade for desktop mode

### Phase 3: Harden Existing iOS Daemon Path

- implement LAN and internet deployment support for the canonical protocol
- improve auth and connection model where needed
- validate background/foreground and reconnect behavior
- improve mobile input ergonomics

### Phase 4: Unify Shared Remote Concepts

- explicit remote status surfaces in UI
- shared session identity and reconnect model
- shared compatibility/version handling
- shared test fixtures and protocol validation

## Non-Goals For The First SSH Milestone

The first desktop SSH milestone does not need to solve:

- mobile roaming transport
- internet-facing daemon exposure
- speculative local echo
- total transport unification between SSH desktop and iOS mobile modes

It only needs to validate the core desktop experience:

```bash
boo --host <host>
```

with reliable session control and live rendering through the existing Boo architecture.
