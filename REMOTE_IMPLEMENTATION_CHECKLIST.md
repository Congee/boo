# Boo Remote Implementation Checklist

Date: April 17, 2026

This checklist turns [REMOTE_REQUIREMENTS.md](./REMOTE_REQUIREMENTS.md) into concrete implementation work.

It is organized so that:

- requirements and product decisions live in `REMOTE_REQUIREMENTS.md`
- implementation sequencing and deliverables live here

## Current Baseline

Already present in the repo:

- first-cut desktop SSH remote path via `boo --host`
- Boo-native TCP daemon used by the iOS client
- iOS client under [ios/](./ios)
- remote verification helpers:
  - [scripts/verify-remote-host.sh](./scripts/verify-remote-host.sh)
  - [scripts/verify-remote-mac.sh](./scripts/verify-remote-mac.sh)
  - [scripts/remote-stream-client.py](./scripts/remote-stream-client.py)

This checklist assumes those pieces remain the starting point.

## Verification Structure

Implementation work shall preserve the three verification layers defined in [REMOTE_REQUIREMENTS.md](./REMOTE_REQUIREMENTS.md):

1. Protocol-level verification
2. End-to-end transport verification
3. Manual UX verification

This means:

- protocol changes should come with protocol-level checks
- transport/bootstrap changes should come with end-to-end checks
- mobile and human-facing UX changes should identify which behaviors remain manual-only

## Build and Packaging Foundation

Goal:

- make Boo produce a canonical build artifact on both Linux and Darwin
- use Nix to reduce dependence on mutable remote checkout binaries

### 1. Flake Outputs

- [x] Add `packages.default` for `boo`
- [x] Add `apps.default` for `boo`
- [x] Add a basic `checks.default`
- [ ] Add a documented Nix run/build flow for developers and remote verification

### 2. Remote Darwin Builder

- [ ] Configure `example-mbp.local` as an `aarch64-darwin` remote Nix builder
- [x] Add a one-off helper for Darwin remote builds:
  - [scripts/nix-build-remote-darwin.sh](./scripts/nix-build-remote-darwin.sh)
- [ ] Prove Linux can request a Darwin build through that builder
  - Current status: the split `libghostty-vt` Nix package works on Linux, but Darwin still fails inside Ghostty's Apple SDK discovery path.
  - Keep direct host `cargo build` on `example-mbp.local` as the working remote-Mac build path until the Darwin Nix derivation is fixed.
- [ ] Decide the stable remote binary reference:
  - direct Nix store path
  - or a managed wrapper/symlink pointing at the current Nix build

### 3. Remote Bootstrap Integration

- [ ] Prefer a Nix-built remote Boo binary over mutable `target/debug/boo` when configured
- [ ] Verify SSH bootstrap works against the Nix-produced remote binary
- [ ] Keep the SSH transport logic independent from the Nix build/deploy mechanism

## Phase 1: Finish Desktop SSH Mode

Goal:

- make `boo --host <ssh-host>` solid for desktop-to-desktop usage
- keep using SSH plus forwarded Boo sockets as the near-term implementation

### 1. CLI and Config

- [x] Support `remote-binary` expansion for:
  - `~`
  - `$HOME`
- [x] Support the same expansion rules for:
  - `remote-workdir`
  - `remote-socket`
- [x] Normalize remote host config precedence:
  - CLI flag beats config file
  - explicit `--socket` beats generated host-specific socket
- [x] Improve `--help` text for remote flags to describe current SSH behavior clearly
- [x] Overhaul remote path expansion so `~` and `$HOME` handling is implemented as a deliberate remote-path resolution layer instead of ad hoc string rewriting during SSH bootstrap

### 2. Remote Bootstrap

- [x] Verify remote Boo binary existence before trying to start the server
- [x] Detect and report startup failures clearly:
  - [x] SSH failure
  - [x] missing binary
  - [x] missing workdir
  - [x] permission failure
  - [x] socket bind/start failure
- [x] Make remote bootstrap idempotent:
  - do not spawn duplicate remote servers unnecessarily
- [x] Record or expose the effective remote socket path in logs/status

### 3. SSH Tunnel Lifecycle

- [x] Reuse an existing SSH master connection when healthy
- [x] Detect stale local forwarded sockets and clean them safely
- [x] Detect broken master/tunnel state and rebuild it
- [x] Define desktop reconnect behavior for:
  - tunnel drop
  - remote server restart
  - client laptop sleep/resume
- [x] Surface remote connection state in the GUI or status output

### 4. Version and Compatibility

- [x] Add client/server version handshake for remote desktop mode
- [x] Detect incompatible local vs remote Boo versions before attach
- [x] Produce a clear actionable error for version mismatch
  - [x] restart a stale remote server on the configured socket when control version negotiation proves it is out of date

### 5. Verification

#### Protocol-Level

- [x] Add direct checks for:
  - version mismatch handling
  - [x] remote bootstrap error classification where protocol-visible

#### End-to-End

- [x] Keep the current direct checks green:
  - control snapshot through forwarded socket
  - `.stream` session listing through forwarded socket
  - remote `new-session`
- [x] Keep the direct Mac sidecar flow green by syncing the repo before build:
  - [scripts/sync-remote-mac.sh](./scripts/sync-remote-mac.sh)
  - [scripts/verify-remote-mac.sh](./scripts/verify-remote-mac.sh)
- [x] Add dedicated automated checks for:
  - remote path expansion
  - stale tunnel recovery
- [x] Keep `scripts/verify-remote-mac.sh` working as the sidecar verifier

#### Manual UX

- [x] Confirm user-visible remote state remains understandable when:
  - bootstrap fails
  - attach succeeds
  - the tunnel is rebuilding

Exit criteria for Phase 1:

- `boo --host <ssh-host>` is reliable for desktop use
- remote startup, attach, and session control work without ad hoc operator steps
- failures are clear enough that users can self-diagnose common setup issues

## Phase 2: Implement Unified Remote Transport

Goal:

- create the canonical Boo remote transport shared by desktop and iOS
- keep SSH as an optional bootstrap path, not the permanent transport contract

### 1. Handshake and Versioning

- [ ] Define and implement a protocol handshake that includes:
  - [x] protocol version
  - [x] server version/build identifier
  - [x] transport capabilities
  - [x] reconnect/resume capabilities
- [ ] Reject incompatible peers early and clearly
  - [x] iOS client rejects malformed or unsupported `AuthOk` metadata before issuing protocol side effects
  - [x] desktop/developer direct probing uses the same Rust `AuthOk` validation rules before reporting success
  - [ ] desktop and all future direct clients enforce the same rejection rules
- [ ] Add upgrade path from SSH bootstrap to canonical Boo transport

### 2. Connection Model

- [ ] Implement one encrypted Boo-native connection per attached client
- [ ] Keep logical multiplexing for:
  - control RPCs
  - terminal/session stream
  - input/resize
  - health/heartbeat
  - resume metadata
- [ ] Preserve the existing control/stream semantics at the protocol level, even if they no longer require two physical sockets

### 3. Transport Backends

- [ ] Implement preferred live transport with connection-migration support
- [ ] Implement TCP/TLS fallback when UDP is unavailable or blocked
- [ ] Make both transports speak the same application protocol
- [ ] Expose negotiated transport details for debugging
  - [x] iOS client surfaces negotiated protocol/capability/build/instance metadata
  - [x] iOS client surfaces heartbeat RTT in the debug summary
  - [x] iOS client surfaces degraded/lost transport state in the UI

### 4. Resume and Reconnect

- [x] Add client attachment identity separate from session identity
- [x] Add resumable attach tokens/metadata
- [x] Support reconnect to the same attachment within the allowed revive window
- [x] Prevent duplicate or phantom attachments after reconnect
- [x] Carry enough state for clients to resume without a full destructive reset when possible

### 5. Heartbeats and Timeouts

- [ ] Implement active heartbeat traffic
  - [x] direct iOS client sends periodic heartbeat frames
  - [x] server replies with heartbeat acknowledgements
- [x] Implement heartbeat failure detection
- [x] Implement reconnect notification threshold
- [x] Implement reconnect deadline
- [x] Implement longer server-side session revival window
- [x] Surface connection state transitions to clients

### 6. Security

- [ ] Replace shared-secret-only assumptions with stronger direct-client auth
- [ ] Add server identity verification for direct Boo transport clients
  - [x] persist and warn on daemon instance-id changes for known iOS endpoints
  - [x] allow users to trust the current daemon instance for a known iOS endpoint
- [ ] Ensure bootstrap credentials are replay-resistant and bounded
  - [x] expire outstanding HMAC auth challenges on the server
  - [x] close unauthenticated direct-client connections after a failed HMAC response
  - [x] close idle or challenge-expired unauthenticated direct-client connections after bounded timeout
- [x] Ensure resumed connections cannot hijack unrelated sessions
  - [x] refuse automatic iOS resume when a known endpoint presents a different daemon identity
  - [x] require a server-issued resume token in addition to attachment identity

### 7. Verification

#### Protocol-Level

- [ ] Add tests for:
  - [x] handshake
  - [x] capability negotiation
  - [x] reconnect/resume attachment restore primitives
  - [x] resume-token rejection and recovery
  - multiplexed channels
  - [x] heartbeat request/ack round-trip
  - heartbeat loss and recovery
  - transport fallback

#### End-to-End

- [ ] Add end-to-end tests for:
  - SSH bootstrap then protocol upgrade
  - [x] direct client connect on the current native iOS path
  - version mismatch
  - [x] resume after temporary drop within the revive window
  - [x] local native-daemon diagnostics via [scripts/test-remote-daemon-diagnostics.sh](./scripts/test-remote-daemon-diagnostics.sh)

#### Manual UX

- [ ] Identify which transport-state transitions must still be judged manually in real clients

Exit criteria for Phase 2:

- desktop and iOS can speak one canonical remote protocol
- SSH is optional bootstrap, not the remote contract
- reconnect and resume are part of the transport, not bolt-on behavior

## Phase 3: Harden iOS Integration Against the Unified Transport

Goal:

- move the iOS app from today’s daemon assumptions to the canonical transport model

### 1. Transport Integration

- [ ] Update iOS client handshake to the canonical protocol
  - [x] protocol version / capability / build-id handshake decoding
  - [x] incompatible-handshake rejection in the production client
  - [x] attachment identity propagation
  - [x] resume token / reconnect metadata
- [ ] Support direct connection to the canonical remote endpoint
  - [x] auth-protected direct connect
  - [x] wrong-key direct-auth rejection validation
  - [x] authless direct connect
- [ ] Support reconnect/resume after:
  - [x] app background/foreground
  - [x] device sleep/wake via the same active-scene reconnect path
  - [x] network change with bounded client retries

### 2. UX and Input

- [ ] Review keyboard UX against remote-control needs:
  - escape
  - ctrl/meta/alt
  - arrows/function keys
- [ ] Add touch/gesture behaviors for:
  - scrolling
  - selection/navigation
  - session switching if needed
- [ ] Surface connection state:
  - [x] connected
  - [x] reconnecting
  - [x] degraded
  - [x] disconnected

### 3. Deployment Modes

- [ ] Preserve LAN discovery via Bonjour where useful
- [ ] Support manual endpoint entry for internet mode
- [ ] Clarify saved-host model for:
  - LAN-discovered hosts
  - directly entered hosts
  - auth material

### 4. Verification

#### Protocol-Level

- [ ] Extend protocol validation to cover the canonical iOS handshake and resume flow
  - [x] initial handshake with protocol version and capability decoding
  - [x] heartbeat acknowledgement echo validation
  - [x] resume flow

#### End-to-End

- [ ] Extend `scripts/test-ios-remote-view.sh` for the canonical protocol
  - [x] auth/list/create/attach/input/state-update validation on the current protocol
  - [x] wrong-key auth rejection validation
  - [x] authless direct-connect validation
  - [x] resume/reconnect validation
- [x] Add reconnect/resume validation for the iOS client

#### Manual UX

- [ ] Keep manual validation focused on:
  - touch UX
  - keyboard accessory UX
  - background/foreground behavior
  - iOS permissions

Exit criteria for Phase 3:

- the iOS app is a first-class client of the same remote transport as desktop Boo

## Phase 4: Shared Product Polish

Goal:

- make remote Boo feel coherent regardless of client type

### 1. UI Surfaces

- [ ] Show remote host/connection state in desktop Boo
- [ ] Show transport/debug state in logs or developer UI
  - [x] add an opt-in desktop fallback-status debug summary sourced from `get-remote-clients`
  - [x] log native remote server startup metadata including protocol/capability/auth/identity details
- [ ] Surface resumable/disconnected state cleanly
  - [x] desktop fallback status distinguishes resumable recovery by active session id

### 2. Observability

- [ ] Add structured remote transport logging
  - [x] log remote connect/auth/attach/revive/disconnect lifecycle events on the Rust server
  - [x] log native remote server startup metadata for direct TCP and local-stream daemons
- [x] Add per-connection and per-session diagnostic info
  - [x] expose remote client and revivable-attachment diagnostics over the control socket and `boo remote-clients`
  - [x] include auth/heartbeat age and revive-window expiry diagnostics in `boo remote-clients`
  - [x] include daemon metadata and per-client transport routing info in `boo remote-clients`
- [ ] Add protocol/transport metrics useful for latency and reconnect debugging
  - [x] surface iOS client connect/auth/list/attach timing and heartbeat RTT in the debug summary
  - [x] expose per-daemon client/attachment counts in `boo remote-clients`

### 3. Documentation

- [ ] Update `FEATURES.md` remote sections to match the canonical transport
- [ ] Keep `REMOTE_REQUIREMENTS.md` as the product contract
- [ ] Update `REMOTE_SSH_INTEGRATION_PLAN.md` to position SSH mode as:
  - first milestone
  - bootstrap path
  - not the final unified transport contract

## Open Implementation Questions

These are implementation questions, not product-direction questions:

- [ ] exact wire format for the new unified handshake
- [ ] exact channel framing for multiplexing
- [ ] whether the preferred live transport should be QUIC directly or a comparable migration-capable design
- [ ] exact timeout values for:
  - heartbeat
  - reconnect notification
  - reconnect deadline
  - session revival window
- [ ] which status/diagnostic details are exposed to end users vs debug-only tooling

## Recommended Immediate Next Steps

1. Finish Phase 1 items that unblock day-to-day desktop use:
   - remote path expansion
   - version mismatch detection
   - tunnel lifecycle recovery

2. Start the Phase 2 protocol skeleton:
   - handshake struct
   - capability/version negotiation
   - logical channel framing
   - resume token model

3. Keep using:
   - `scripts/verify-remote-host.sh`
   - `scripts/verify-remote-mac.sh`
   as the practical remote verification lane while the unified transport is being built.
