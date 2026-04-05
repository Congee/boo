# Boo iOS Remote Viewer

This directory contains the Boo iOS remote viewer app.

## Current Scope

- SwiftUI iOS app with bundle identifier `me.congee.boo`
- Connects to a compatible remote daemon using the existing GSP wire protocol
- Discovers Bonjour services on both `_boo._tcp` and `_ghostty._tcp`
- Supports:
  - manual connect
  - optional auth key
  - saved nodes
  - connection history
  - session list / attach / create
  - remote terminal cell-grid rendering

## Notes

- The iOS app is Boo-owned code. The older Ghostty iOS app was used only as a reference during implementation.
- The daemon/service side in the main Boo Rust app does not exist yet. This client currently targets a compatible remote daemon protocol.
- iOS local-network discovery requires `NSLocalNetworkUsageDescription` and `NSBonjourServices`; both are configured in the Xcode project.

## Verification

In this environment, the Swift sources compile through Xcode's Swift phase, but final linking via `xcodebuild` is currently blocked by an Xcode/linker environment issue that emits malformed linker arguments during the final app link step. The app project and sources are still structured for normal Xcode use on a local developer machine.
