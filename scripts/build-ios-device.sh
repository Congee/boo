#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEAM_ID="${BOO_IOS_TEAM_ID:-${1:-}}"
DEVICE_ID="${BOO_IOS_DEVICE_ID:-${2:-}}"
DERIVED_DATA="${BOO_IOS_DERIVED_DATA_PATH:-/tmp/boo-ios-derived}"
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

xcodebuild_clean_env() {
  env \
    -u DEVELOPER_DIR \
    -u SDKROOT \
    -u MACOSX_DEPLOYMENT_TARGET \
    -u IPHONEOS_DEPLOYMENT_TARGET \
    -u NIX_LDFLAGS \
    -u NIX_CFLAGS_COMPILE \
    -u NIX_CXXSTDLIB_COMPILE \
    -u CC \
    -u CXX \
    -u LD \
    -u AR \
    -u NM \
    -u RANLIB \
    -u LIBTOOL \
    -u LDPLUSPLUS \
    -u OTHER_LDFLAGS \
    -u OTHER_SWIFT_FLAGS \
    "$@"
}

cd "$ROOT"
xcodebuild_clean_env xcodebuild \
  -project ios/Boo.xcodeproj \
  -scheme Boo \
  -configuration "$CONFIGURATION" \
  -derivedDataPath "$DERIVED_DATA" \
  -destination "$DESTINATION" \
  -allowProvisioningUpdates \
  DEVELOPMENT_TEAM="$TEAM_ID" \
  build
