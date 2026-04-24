#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEAM_ID="${BOO_IOS_TEAM_ID:-}"
DEVICE_ID="${BOO_IOS_DEVICE_ID:-}"
DERIVED_DATA="${BOO_IOS_DERIVED_DATA_PATH:-/tmp/boo-ios-derived}"
CONFIGURATION="${BOO_IOS_CONFIGURATION:-Debug}"

usage() {
  cat <<'EOF'
Usage: bash scripts/build-ios-device.sh [options]

Options:
  --team-id TEAM_ID
  --device-id UDID
  --derived-data PATH
  --configuration NAME
  -h, --help

Environment variable fallbacks remain supported:
  BOO_IOS_TEAM_ID
  BOO_IOS_DEVICE_ID
  BOO_IOS_DERIVED_DATA_PATH
  BOO_IOS_CONFIGURATION
EOF
}

require_arg() {
  if [[ $# -lt 2 ]]; then
    echo "Missing value for $1" >&2
    usage >&2
    exit 2
  fi
}

POSITIONAL=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --team-id)
      require_arg "$@"; TEAM_ID="$2"; shift 2 ;;
    --device-id)
      require_arg "$@"; DEVICE_ID="$2"; shift 2 ;;
    --derived-data)
      require_arg "$@"; DERIVED_DATA="$2"; shift 2 ;;
    --configuration)
      require_arg "$@"; CONFIGURATION="$2"; shift 2 ;;
    -h|--help)
      usage; exit 0 ;;
    --)
      shift
      POSITIONAL+=("$@")
      break
      ;;
    -*)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      POSITIONAL+=("$1")
      shift
      ;;
  esac
done

if [[ ${#POSITIONAL[@]} -gt 0 && -z "$TEAM_ID" ]]; then
  TEAM_ID="${POSITIONAL[0]}"
fi
if [[ ${#POSITIONAL[@]} -gt 1 && -z "$DEVICE_ID" ]]; then
  DEVICE_ID="${POSITIONAL[1]}"
fi
if [[ ${#POSITIONAL[@]} -gt 2 ]]; then
  echo "Unexpected positional arguments: ${POSITIONAL[*]:2}" >&2
  usage >&2
  exit 2
fi

discover_team_id() {
  defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier 2>/dev/null \
    | sed -n 's/.*teamID = \([A-Z0-9]*\);/\1/p' \
    | head -n 1
}

if [[ -z "$TEAM_ID" ]]; then
  TEAM_ID="$(discover_team_id)"
fi

if [[ -z "$TEAM_ID" ]]; then
  usage >&2
  echo "tip: open Xcode once with your Apple ID signed in, or pass --team-id explicitly" >&2
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
