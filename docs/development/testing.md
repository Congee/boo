# Testing And Verification

boo uses a mix of unit tests, targeted scenario scripts, socket-level checks,
and selective manual validation.

## Baseline

```bash
cargo test
```

## Repo Scripts

Desktop and UI:

- `bash scripts/test-ui-snapshot.sh`
- `bash scripts/test-ui-scenarios.sh`
- `bash scripts/test-gui-client.sh`

Headless and server:

- `bash scripts/test-headless.sh`
- `bash scripts/test-headless-scenarios.sh`

Remote:

- `bash scripts/verify-remote-host.sh`
- `bash scripts/verify-remote-mac.sh`
- `bash scripts/test-ios-remote-view.sh`
- `bash scripts/list-ios-devices.sh`
- `bash scripts/check-ios-device-state.sh <device-id>`
- `bash scripts/build-ios-device.sh`
- `BOO_IOS_DEVICE_ID=<device-id> bash scripts/install-ios-device.sh`
- `BOO_IOS_DEVICE_ID=<device-id> bash scripts/launch-ios-device.sh`
- `BOO_IOS_DEVICE_ID=<device-id> bash scripts/deploy-ios-device.sh`
- `bash scripts/test-ios-ui.sh --destination 'id=<device-id>'`
- `bash scripts/test-ios-ui.sh --destination 'id=<device-id>' --only-testing 'BooUITests/BooAppLaunchTests/<testName>'`

Current real-device smoke baseline:

- `bash scripts/test-ios-ui.sh --destination 'id=<device-id>' --only-testing 'BooUITests/BooAppLaunchTests/testTappingDiscoveredDaemonConnectsAndTypes'`
- this path has passed on physical iPad and iPhone hardware and verifies
  discovered daemon -> terminal screen -> type -> echoed marker

## Testing Principles

- prefer deterministic socket/control checks when possible
- use visual/manual validation for UX and input behavior that cannot be proven
  well through protocol checks alone
- keep high-risk subsystem docs updated when behavior changes
- when adding third-party discovery paths such as Tailscale, separate
  peer-list validation from actual Boo service reachability; device discovery
  alone does not prove the remote daemon is listening
- real-device Bonjour discovery also depends on the iOS Local Network
  permission; if the app reports local-network authorization failure, that is
  a device permission issue, not a QUIC daemon failure
- if a real-device run fails before Boo launches, check signing/provisioning,
  locked-device state, Developer Mode / UI Automation, and developer-disk-image
  setup before debugging runtime protocol behavior
