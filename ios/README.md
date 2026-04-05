# Boo iOS Remote Viewer

This directory contains the Boo iOS remote viewer app.

## Current Scope

- SwiftUI iOS app with bundle identifier `me.congee.boo`
- Connects to the Boo remote daemon using the existing GSP-compatible wire protocol
- Discovers Bonjour services on `_boo._tcp`
- Supports:
  - manual connect
  - optional auth key
  - saved nodes
  - connection history
  - session list / attach / create
  - remote terminal cell-grid rendering

## Notes

- The iOS app is Boo-owned code. The older Ghostty iOS app was used only as a reference during implementation.
- The Boo Rust app now exposes the matching TCP daemon when `remote-port` is configured, for example `boo --headless --remote-port 7337`.
- Optional challenge/response auth is enabled by configuring `remote-auth-key` on the Boo daemon side and entering the same key in the iOS client.
- iOS local-network discovery requires `NSLocalNetworkUsageDescription` and `NSBonjourServices`; both are configured in the Xcode project.

## Verification

In this environment, the Swift sources compile through Xcode's Swift phase, but final linking via `xcodebuild` is currently blocked by an Xcode/linker environment issue that emits malformed linker arguments during the final app link step. The app project and sources are still structured for normal Xcode use on a local developer machine.
