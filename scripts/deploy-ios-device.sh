#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEVICE_ID="${BOO_IOS_DEVICE_ID:-${1:-}}"

if [[ -z "$DEVICE_ID" ]]; then
  echo "usage: BOO_IOS_DEVICE_ID=<device-id> $0 [device-id]" >&2
  echo "tip: use scripts/list-ios-devices.sh to find a device identifier" >&2
  exit 2
fi

cd "$ROOT"
BOO_IOS_DEVICE_ID="$DEVICE_ID" bash scripts/check-ios-device-state.sh
bash scripts/build-ios-device.sh
BOO_IOS_DEVICE_ID="$DEVICE_ID" bash scripts/install-ios-device.sh
BOO_IOS_DEVICE_ID="$DEVICE_ID" bash scripts/launch-ios-device.sh
