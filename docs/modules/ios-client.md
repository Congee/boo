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
- auth
- session listing and attach
- reconnect/resume metadata
- terminal state decoding and presentation

## Important Current Behavior

- browses `_boo._tcp`
- connects through resolved Network framework endpoints
- supports saved nodes and connection history
- supports attachment resume and trusted server identity pinning

## Verification

Primary automated lane:

- [../../scripts/test-ios-remote-view.sh](../../scripts/test-ios-remote-view.sh)

Primary high-level doc:

- [../../ios/README.md](../../ios/README.md)

## Change Risks

Changes here can affect:

- auth and handshake behavior
- resume/reconnect UX
- terminal decode correctness
- Bonjour discovery and endpoint handling
