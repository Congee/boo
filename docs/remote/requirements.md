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
- converge toward one shared remote runtime-view protocol
- do not rely on Boo-managed shared secrets for remote auth
- if key-based auth is added, verify public keys server-side in an
  `authorized_keys`-style model and use platform keychain or agent integration
  client-side rather than storing private keys inside Boo

## Implemented Runtime-View Results

The completed redesign now provides:

- one shared runtime truth for:
  - tabs and pane trees
  - pane content/state
  - shared layout stored as normalized split ratios
  - semantic runtime mutations
- one server-owned per-screen view state for:
  - viewed tab
  - focused pane
  - visible panes
  - viewport size
  - attach/detach + idle-timeout lifecycle
- pane-scoped terminal streaming keyed by `tab_id -> pane_id`
- explicit runtime/view/pane revision linkage for stale-update rejection and
  refresh
- per-screen focused-pane-first delivery ordering

## iOS Host Runtime Model

For iOS, the product direction is now:

- one host maps to one canonical Boo runtime view
- reconnecting to the same host should reopen or recover that same host-owned
  runtime state rather than listing multiple candidate lifecycle targets
  heuristically
- the client should connect to runtime state, not pick from a host-local
  legacy lifecycle pool

Two architectural models are worth keeping explicit:

1. Rejected legacy model:
   one host -> legacy list-and-pick bootstrap -> choose a lifecycle target heuristically
2. Desired host-scoped model:
   one host -> one canonical Boo runtime view for iOS -> observe/control the
   same host-owned runtime state until the user explicitly disconnects

In the live product path, iOS bootstraps from runtime state and does not use the
old list-and-pick heuristic.

The first model is structurally wrong for iOS because it allows a visible
terminal that is not the canonical writable terminal for that host. Future
research should treat this as a host-runtime ownership problem, not as a mere
keyboard-focus bug.

With the redesign complete, iOS now attaches to a server-owned view, detaches on
UI disappearance, and may reattach to that view during the idle timeout window
without creating a replacement runtime tab.

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

The current implemented verification baseline for this redesign is:

- `cargo check -q`
- `cargo test -q runtime_server::tests::`
- `cargo test -q client_gui::tests::`

Additional 2026-04-23 real-device verification result:

- `BOO_IOS_UI_TEST_DESTINATION='id=<your device id>' BOO_IOS_UI_TEST_ONLY='BooUITests/BooUITests/testOpenLiveTabAndType' bash scripts/test-ios-ui.sh`
  now completes build, link, signing, install, and test-runner launch on the
  physical iPad after `xcodebuild` environment sanitization in the script
- the corrected focused test invocation
  `BOO_IOS_UI_TEST_ONLY='BooUITests/BooAppLaunchTests/testOpenLiveTabAndType'`
  now executes on-device and currently fails later in the connect flow with
  `Connection refused`
- that remaining issue is tracked as real-device workflow / transport follow-up
  rather than a missing runtime-view protocol redesign item

## Related Docs

- [./runtime-view-migration.md](./runtime-view-migration.md)
- [./ssh-desktop.md](./ssh-desktop.md)
- [./implementation-checklist.md](./implementation-checklist.md)
- [../modules/remote-daemon.md](../modules/remote-daemon.md)
