# Boo iOS Remote Viewer

This directory contains the Boo iOS remote viewer app.

## Current Scope

- SwiftUI iOS app with bundle identifier `me.congee.boo`
- Connects to the Boo remote daemon using the existing GSP-compatible wire protocol
- Discovers Bonjour services on `_boo._udp`
- Lists Tailscale devices through the Tailscale API when configured in Settings
- Connects to discovered Bonjour services via the resolved Network framework endpoint instead of degrading them to a guessed `host:port`
- Supports:
  - manual connect
  - saved nodes
  - connection history
  - trusted daemon identity pinning per endpoint
  - runtime-first bootstrap from the daemon's current runtime state
  - runtime-state and tab metadata observation
  - terminal accessory keys for:
    - ctrl
    - alt/meta
    - command
    - tab
    - punctuation helpers used in terminal workflows
    - left/right arrows
    - press-and-hold repeat for repeatable keys
  - terminal drag gestures for:
    - terminal scroll-wheel style scrolling
    - left/right swipe navigation
  - remote terminal cell-grid rendering

## Notes

- The iOS app is Boo-owned code. The older Ghostty iOS app was used only as a reference during implementation.
- The Boo Rust app now exposes the matching TCP daemon when `remote-port` is configured, for example `boo --headless --remote-port 7337`.
- iOS local-network discovery requires `NSLocalNetworkUsageDescription` and `NSBonjourServices`; both are configured in the Xcode project.
- If Bonjour discovery reports that local network access is required, enable `boo` in `Settings > Privacy & Security > Local Network`.
- Tailscale discovery is separate from Bonjour. It lists devices in the same tailnet through the Tailscale API, then connects to them on the configured Boo port.
- The Tailscale section currently discovers devices, not Boo services. It assumes Boo is listening on the configured remote port, which defaults to `7337`.
- Tailscale discovery requires a Tailscale API access token configured in the iOS app Settings screen.
- The token is stored in the iOS Keychain. The Settings screen shows whether one is saved, but it does not re-display the secret value.
- The iOS app does not reuse the installed Tailscale app's login state for this feature.

## Verification

Automated validation now exists in [`scripts/test-ios-remote-view.sh`](/Users/example/dev/boo/scripts/test-ios-remote-view.sh).

The Swift app client and the validator now share the same wire-codec implementation in [`ios/Sources/WireCodec.swift`](/Users/example/dev/boo/ios/Sources/WireCodec.swift), so the protocol smoke test and the shipped app decode the same tab-list and full-state payloads.

It verifies:

- Bonjour discovery on `_boo._udp`
- runtime bootstrap and tab metadata observation
- runtime-state refresh after server-owned tab changes
- resize
- terminal-state publishing with a real shell command round-trip
- wire-codec decoding for full-state and delta updates with a standalone Swift self-test
- client message-state transitions for auth, runtime bootstrap, and delta application

The automated validation lane currently covers Bonjour. Tailscale peer discovery
is app-integrated, but it is not yet covered by an automated repo-side test
because it depends on a real tailnet API token and live peer inventory.

Run it with:

```bash
bash scripts/test-ios-remote-view.sh
```

The validation script currently completes the Rust daemon checks, the shared Swift protocol self-tests, and the iOS app build path in this repo environment.

The 2026-04-23 real-device smoke baseline also verifies the discovered-daemon
connect-and-type path on physical iPad and iPhone hardware with:

```bash
bash scripts/test-ios-ui.sh \
  --destination 'id=<device-id>' \
  --only-testing 'BooUITests/BooAppLaunchTests/testTappingDiscoveredDaemonConnectsAndTypes'
```

## Real Device Workflow

For a physical iPad or iPhone, the project needs a valid Apple development
signing identity and a non-empty development team.

Helpful commands:

```bash
bash scripts/list-ios-devices.sh
bash scripts/check-ios-device-state.sh <device-id>
bash scripts/build-ios-device.sh
BOO_IOS_DEVICE_ID=<device-id> bash scripts/build-ios-device.sh
BOO_IOS_DEVICE_ID=<device-id> bash scripts/install-ios-device.sh
BOO_IOS_DEVICE_ID=<device-id> bash scripts/launch-ios-device.sh
BOO_IOS_DEVICE_ID=<device-id> bash scripts/deploy-ios-device.sh
bash scripts/test-ios-ui.sh --destination 'id=<device-id>'
```

Notes:

- the build script uses `/tmp/boo-ios-derived` so it does not depend on
  Xcode's default `~/Library/Developer/Xcode/DerivedData` location
- the build script auto-discovers the first provisioning team known to Xcode;
  override with `BOO_IOS_TEAM_ID` if needed
- `check-ios-device-state.sh` reports whether the device is actually ready for
  install and launch
- if the attached device still reports Developer Mode disabled, installation
  will be blocked until the device finishes the full enable-and-reboot flow
- if the device is locked, `devicectl` cannot mount developer services for
  install or launch; unlock the device and retry
- on a personal-team signed build, the first launch can still be blocked until
  the device explicitly trusts the development profile in Settings; the launch
  script now points at that step when iOS rejects the app for trust reasons
- `scripts/test-ios-ui.sh` now runs XCUITests against either the simulator or a
  real attached device; on a real device it starts a local Boo daemon, writes a
  temporary UI-test host config for the test bundle, and exercises the visible
  connect/runtime-view terminal flow end-to-end
- `scripts/test-ios-ui.sh` also sanitizes inherited shell toolchain overrides
  such as `LD`, `CC`, `CXX`, `SDKROOT`, and `NIX_LDFLAGS` before invoking
  `xcodebuild`; this avoids host-shell linker leakage that can break real-device
  builds
- if a real-device UI test reaches the runner but fails with `Timed out while
  enabling automation mode`, verify `Settings > Developer > Enable UI Automation`
  on the device and keep the device unlocked during the run
- if `xcodebuild` reports `The developer disk image could not be mounted on this
  device`, open Xcode's Devices and Simulators window and let Xcode finish any
  required support-file / developer-disk-image setup before retrying
- Bonjour discovery on a real device still depends on the iPad or iPhone granting
  Local Network access to `boo`; otherwise the app now surfaces a direct error
  and an `Open iPad Settings` action instead of silently showing an empty list
- if a discovered-daemon UI test fails before a row appears, first check for
  stale Bonjour publishers or local-network permission before treating it as a
  runtime-view protocol failure

## Remaining Manual Validation

Automated validation covers the remote protocol, runtime-first bootstrap flow,
state updates, and the real-device discovered-daemon connect-and-type smoke
path. Manual validation is still reserved for client UX that depends on the real
iOS interaction model:

- keyboard accessory ergonomics on-device, especially held modifiers,
  command/control combos, and press-and-hold repeat behavior
- drag-scroll feel and left/right terminal movement
- background / foreground reconnect behavior as seen by the user
- iOS local-network permission prompts and discovery behavior in a real app install

Transport-state transitions that still need manual judgment in a real client:

- when degraded heartbeat state should feel visible but not alarming
- when reconnecting state should block input vs leave the last terminal visible
- how long an inactive runtime view should keep the last screen visible before
  the UI feels misleading

## Post-v1 Follow-up

- define how search, copy mode, and scrollback should behave for multiple
  screens with different viewport sizes
- add latency measurement before enabling client-side prediction for focus,
  tab, or status interactions
- harden transport scheduling so focused pane traffic remains low-latency while
  non-focused visible panes are coalesced without starvation
- refine reconnect UX around closing the mobile view, disconnecting the
  transport, and closing a shared runtime tab
