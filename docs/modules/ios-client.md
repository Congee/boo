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
- session listing and attach
- reconnect/resume metadata
- terminal state decoding and presentation

## Important Current Behavior

- browses `_boo._tcp`
- can list tailnet devices when a Tailscale API access token is configured in Settings
- does not reuse the installed Tailscale app's authenticated session
- stores the Tailscale API access token in the iOS Keychain rather than plain app settings
- connects through resolved Network framework endpoints
- supports saved nodes and connection history
- supports attachment resume and trusted server identity pinning

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

Primary high-level doc:

- [../../ios/README.md](../../ios/README.md)

## Change Risks

Changes here can affect:

- auth and handshake behavior
- resume/reconnect UX
- terminal decode correctness
- Bonjour discovery and endpoint handling
- Tailscale peer discovery and token handling
- XCUITest state setup for real-device validation
