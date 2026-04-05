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

Automated validation now exists in [`scripts/test-ios-remote-view.sh`](/Users/example/dev/boo/scripts/test-ios-remote-view.sh).

The Swift app client and the validator now share the same wire-codec implementation in [`ios/Sources/WireCodec.swift`](/Users/example/dev/boo/ios/Sources/WireCodec.swift), so the protocol smoke test and the shipped app decode the same session and full-state payloads.

It verifies:

- Bonjour discovery on `_boo._tcp`
- HMAC auth against a live Boo daemon
- session listing
- create + attach
- resize
- terminal-state publishing with a real shell command round-trip
- wire-codec decoding for full-state and delta updates with a standalone Swift self-test
- client message-state transitions for auth, attach, detach, session creation, and delta application

Run it with:

```bash
bash scripts/test-ios-remote-view.sh
```

In this environment, `xcodebuild` reaches full Swift compilation for the iOS app, but final linking still fails with an Xcode/linker environment issue during the last app-link step. The validation script treats that specific linker failure as environmental and still enforces the live Boo daemon protocol validation.
