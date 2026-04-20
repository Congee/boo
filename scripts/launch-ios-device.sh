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

xcrun devicectl device process launch \
  --device "$DEVICE_ID" \
  --terminate-existing \
  "$BUNDLE_ID"
