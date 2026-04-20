#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEAM_ID="${BOO_IOS_TEAM_ID:-${1:-}}"
DEVICE_ID="${BOO_IOS_DEVICE_ID:-${2:-}}"
DERIVED_DATA="${BOO_IOS_DERIVED_DATA_PATH:-$ROOT/ios/.derived-device}"
CONFIGURATION="${BOO_IOS_CONFIGURATION:-Debug}"

discover_team_id() {
  defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier 2>/dev/null \
    | sed -n 's/.*teamID = \([A-Z0-9]*\);/\1/p' \
    | head -n 1
}

if [[ -z "$TEAM_ID" ]]; then
  TEAM_ID="$(discover_team_id)"
fi

if [[ -z "$TEAM_ID" ]]; then
  echo "usage: BOO_IOS_TEAM_ID=<team-id> $0 [team-id] [device-id]" >&2
  echo "tip: open Xcode once with your Apple ID signed in, or pass BOO_IOS_TEAM_ID explicitly" >&2
  exit 2
fi

DESTINATION="generic/platform=iOS"
if [[ -n "$DEVICE_ID" ]]; then
  DESTINATION="id=$DEVICE_ID"
fi

mkdir -p "$DERIVED_DATA"

cd "$ROOT"
xcodebuild \
  -project ios/Boo.xcodeproj \
  -scheme Boo \
  -configuration "$CONFIGURATION" \
  -derivedDataPath "$DERIVED_DATA" \
  -destination "$DESTINATION" \
  -allowProvisioningUpdates \
  DEVELOPMENT_TEAM="$TEAM_ID" \
  build
