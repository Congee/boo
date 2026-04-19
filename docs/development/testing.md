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

## Testing Principles

- prefer deterministic socket/control checks when possible
- use visual/manual validation for UX and input behavior that cannot be proven
  well through protocol checks alone
- keep high-risk subsystem docs updated when behavior changes
