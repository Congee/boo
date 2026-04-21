#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_PATH="${BOO_IOS_UI_TEST_SOCKET:-/tmp/boo-ios-ui-tests.sock}"
PORT="${BOO_IOS_UI_TEST_PORT:-7351}"
DESTINATION="${BOO_IOS_UI_TEST_DESTINATION:-platform=iOS Simulator,name=iPad mini (A17 Pro),OS=26.3.1}"
DERIVED_DATA="${BOO_IOS_UI_TEST_DERIVED_DATA:-$ROOT/ios/.derived-uitests}"
HOST="${BOO_IOS_UI_TEST_HOST:-}"
TEAM_ID="${BOO_IOS_TEAM_ID:-}"
GENERATED_CONFIG="$ROOT/ios/BooUITests/GeneratedUITestConfig.swift"

if [[ -z "$HOST" ]]; then
  if [[ "$DESTINATION" == *"platform=iOS Simulator"* ]]; then
    HOST="127.0.0.1"
  else
    HOST="$(ifconfig en0 | awk '/inet / { print $2; exit }')"
  fi
fi

if [[ "$DESTINATION" == *"platform=iOS Simulator"* ]]; then
  BIND_ADDRESS="127.0.0.1"
else
  BIND_ADDRESS="0.0.0.0"
fi

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH"
  cat > "$GENERATED_CONFIG" <<'EOF'
enum GeneratedUITestConfig {
    static let host: String? = nil
    static let port: UInt16 = 7351
}
EOF
}
trap cleanup EXIT

cd "$ROOT"
cat > "$GENERATED_CONFIG" <<EOF
enum GeneratedUITestConfig {
    static let host: String? = $(printf '%s' "\"$HOST\"")
    static let port: UInt16 = $PORT
}
EOF
cargo build >/dev/null
rm -f "$SOCKET_PATH"
target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" --remote-bind-address "$BIND_ADDRESS" >/tmp/boo-ios-ui-tests.log 2>&1 &
SERVER_PID=$!
sleep 1

if [[ "$DESTINATION" == *"platform=iOS Simulator"* ]]; then
  BOO_UI_TEST_HOST="$HOST" BOO_UI_TEST_PORT="$PORT" \
    xcodebuild \
    -project ios/Boo.xcodeproj \
    -scheme Boo \
    -destination "$DESTINATION" \
    -derivedDataPath "$DERIVED_DATA" \
    test
else
  if [[ -z "$TEAM_ID" ]]; then
    TEAM_ID="$(defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier 2>/dev/null | sed -n 's/.*teamID = \([A-Z0-9]*\);/\1/p' | head -n 1)"
  fi
  if [[ -z "$TEAM_ID" ]]; then
    echo "Could not determine DEVELOPMENT_TEAM for device UI tests" >&2
    exit 2
  fi
  BOO_UI_TEST_HOST="$HOST" BOO_UI_TEST_PORT="$PORT" \
    xcodebuild \
    -project ios/Boo.xcodeproj \
    -scheme Boo \
    -destination "$DESTINATION" \
    -derivedDataPath "$DERIVED_DATA" \
    -allowProvisioningUpdates \
    DEVELOPMENT_TEAM="$TEAM_ID" \
    INFOPLIST_KEY_BOO_UI_TEST_HOST="$HOST" \
    INFOPLIST_KEY_BOO_UI_TEST_PORT="$PORT" \
    test
fi
