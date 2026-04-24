# Module Group: iOS Client

Primary location:

- `ios/`

Important files:

- `ios/Sources/ProtocolClient.swift`
- `ios/Sources/ClientWireState.swift`
- `ios/Sources/WireCodec.swift`
- `ios/Sources/Screens.swift`

## Role

The iOS app is a native SwiftUI client for Boo's native remote daemon.

It handles:

- Bonjour discovery
- optional Tailscale device discovery through the Tailscale API
- auth
- runtime-view bootstrap
- tab metadata and current viewer state
- trusted server identity pinning
- terminal state decoding and presentation

## Important Current Behavior

- browses `_boo._udp`
- can list tailnet devices when a Tailscale API access token is configured in Settings
- does not reuse the installed Tailscale app's authenticated connection state
- stores the Tailscale API access token in the iOS Keychain rather than plain app settings
- cannot call true `tailscale ping` through the installed Tailscale iOS app
- using `libtailscale` / `TailscaleKit` would mean running a second embedded Tailscale node inside Boo, which is a different architecture from reusing the official Tailscale app
- therefore current Tailscale dashboard metrics are app-level Boo service probes on the configured port, not Tailscale-native peer/path telemetry
- connects through resolved Network framework endpoints
- supports saved nodes and connection history
- bootstraps from runtime state rather than selecting a client-owned target
- does not render separate native runtime tab chrome; the Boo core statusbar is
  the visible tab-list UI
- if Bonjour browsing returns local-network authorization failure, the app now shows a direct error and an `Open iPad Settings` action instead of silently showing an empty discovery list
- real-device smoke coverage has verified the discovered-daemon connect-and-type
  path on both physical iPad and iPhone hardware
- emits native `Logger` rows and `OSSignposter` intervals for shared
  runtime-view latency events, including `remote.input`, `remote.focus_pane`,
  `remote.set_viewed_tab`, `remote.pane_update`, and `remote.render_apply`

## Verification

Primary automated lane:

- [../../scripts/test-ios-remote-view.sh](../../scripts/test-ios-remote-view.sh)
- [../../scripts/list-ios-devices.sh](../../scripts/list-ios-devices.sh)
- [../../scripts/check-ios-device-state.sh](../../scripts/check-ios-device-state.sh)
- [../../scripts/build-ios-device.sh](../../scripts/build-ios-device.sh)
- [../../scripts/install-ios-device.sh](../../scripts/install-ios-device.sh)
- [../../scripts/launch-ios-device.sh](../../scripts/launch-ios-device.sh)
- [../../scripts/deploy-ios-device.sh](../../scripts/deploy-ios-device.sh)
- [../../scripts/test-ios-ui.sh](../../scripts/test-ios-ui.sh)
- [../../scripts/verify-ios-signposts.sh](../../scripts/verify-ios-signposts.sh)

Primary high-level doc:

- [../../ios/README.md](../../ios/README.md)

## Change Risks

Changes here can affect:

- auth and handshake behavior
- runtime-view bootstrap and reconnect UX
- terminal decode correctness
- Bonjour discovery and endpoint handling
- Tailscale peer discovery and token handling
- XCUITest state setup for real-device validation

## Post-v1 Follow-up

- scroll/search/copy-mode semantics for multiple screens and viewport sizes
- baseline latency measurements before deciding whether local prediction is
  needed for focus/tab/status changes
- focused-pane transport QoS under load
- host-scoped reconnect and detached-view timeout UX
