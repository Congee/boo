#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEVICE_ID="${BOO_IOS_DEVICE_ID:-${1:-}}"
DERIVED_DATA="${BOO_IOS_DERIVED_DATA_PATH:-$ROOT/ios/.derived-device}"
APP_PATH="${BOO_IOS_APP_PATH:-$DERIVED_DATA/Build/Products/Debug-iphoneos/Boo.app}"

if [[ -z "$DEVICE_ID" ]]; then
  echo "usage: BOO_IOS_DEVICE_ID=<device-id> $0 [device-id]" >&2
  echo "tip: use scripts/list-ios-devices.sh to find a device identifier" >&2
  exit 2
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "expected app bundle at $APP_PATH" >&2
  echo "build first with scripts/build-ios-device.sh" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash "$ROOT/scripts/check-ios-device-state.sh" "$DEVICE_ID"

xcrun devicectl device install app --device "$DEVICE_ID" "$APP_PATH"
