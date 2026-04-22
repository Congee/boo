# Remote Requirements

boo remote work currently spans two client modes:

- desktop-to-desktop remote Boo
- iOS/mobile remote Boo

Those modes are expected to converge on one canonical Boo-native transport over
time, even though rollout and UX differ today.

## Current Foundations

- SSH-backed desktop remote bootstrap and forwarding
- Boo-native TCP daemon
- SwiftUI iOS client

## Core Requirements

- keep the server authoritative for PTYs, tabs, splits, and terminal state
- preserve deterministic verification paths
- keep desktop SSH practical without blocking on mobile-specific work
- converge toward one shared remote protocol/session model
- do not rely on Boo-managed shared secrets for remote auth
- if key-based auth is added, verify public keys server-side in an
  `authorized_keys`-style model and use platform keychain or agent integration
  client-side rather than storing private keys inside Boo

## iOS Host Session Model

For iOS, the product direction is now:

- one host maps to one Boo session
- switching between hosts may switch sessions
- reconnecting to the same host should reopen that same host session rather
  than listing multiple candidate sessions and picking one heuristically

Two architectural models are worth keeping explicit:

1. Rejected current-style model:
   one host -> `listSessions()` -> choose a session heuristically -> attach
2. Desired host-scoped model:
   one host -> one canonical Boo session for iOS -> reopen or recover that same
   session until the user explicitly closes it

The first model is structurally wrong for iOS because it allows a visible
terminal that is not the canonical writable terminal for that host. Future
research should treat this as a host-session ownership problem, not as a mere
keyboard-focus bug.

## Tailscale Constraint

- Boo iOS currently uses the Tailscale API only for device discovery.
- Boo iOS cannot call true `tailscale ping` through the installed Tailscale iOS
  app.
- Using `libtailscale` / `TailscaleKit` would mean running a second embedded
  Tailscale node inside Boo rather than reusing the official Tailscale app.
- Without a supported API from the installed Tailscale app, Boo iOS cannot
  surface Tailscale-native direct-vs-DERP path telemetry.
- Until that changes, Tailscale dashboard metrics should be treated as
  application-level Boo service probes against the configured Boo port, roughly
  equivalent to an app-driven `nc -zv -u <tailscale-host> <boo-port>` check,
  not as authoritative Tailscale path RTT.

## Verification Layers

The remote product should preserve three verification layers:

1. protocol-level verification
2. end-to-end transport verification
3. manual UX verification

## Related Docs

- [./runtime-view-migration.md](./runtime-view-migration.md)
- [./ssh-desktop.md](./ssh-desktop.md)
- [./implementation-checklist.md](./implementation-checklist.md)
- [../modules/remote-daemon.md](../modules/remote-daemon.md)
