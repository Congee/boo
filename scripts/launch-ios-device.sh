#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEVICE_ID="${BOO_IOS_DEVICE_ID:-${1:-}}"
BUNDLE_ID="${BOO_IOS_BUNDLE_ID:-me.congee.boo}"

if [[ -z "$DEVICE_ID" ]]; then
  echo "usage: BOO_IOS_DEVICE_ID=<device-id> $0 [device-id]" >&2
  echo "tip: use scripts/list-ios-devices.sh to find a device identifier" >&2
  exit 2
fi

bash "$ROOT/scripts/check-ios-device-state.sh" "$DEVICE_ID"

LOG_FILE="$(mktemp -t boo-ios-launch.XXXXXX.log)"
cleanup() {
  rm -f "$LOG_FILE"
}
trap cleanup EXIT

set +e
xcrun devicectl device process launch \
  --device "$DEVICE_ID" \
  --terminate-existing \
  "$BUNDLE_ID" >"$LOG_FILE" 2>&1
status=$?
set -e

cat "$LOG_FILE"

if [[ $status -ne 0 ]]; then
  if grep -q "profile has not been explicitly trusted by the user" "$LOG_FILE"; then
    echo "" >&2
    echo "launch blocked by iOS trust policy." >&2
    echo "On the device, trust the Apple Development profile for this personal team," >&2
    echo "then retry launch. On recent iOS versions this is typically under:" >&2
    echo "Settings > General > VPN & Device Management" >&2
  fi
  exit "$status"
fi
