#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_PATH="${BOO_IOS_UI_TEST_SOCKET:-/tmp/boo-ios-ui-tests.sock}"
PORT="${BOO_IOS_UI_TEST_PORT:-}"
DESTINATION="${BOO_IOS_UI_TEST_DESTINATION:-platform=iOS Simulator,name=iPad mini (A17 Pro),OS=26.3.1}"
DERIVED_DATA="${BOO_IOS_UI_TEST_DERIVED_DATA:-$ROOT/ios/.derived-uitests}"
HOST="${BOO_IOS_UI_TEST_HOST:-}"
TEAM_ID="${BOO_IOS_TEAM_ID:-}"
ONLY_TEST="${BOO_IOS_UI_TEST_ONLY:-}"
GENERATED_CONFIG="$ROOT/ios/BooUITests/GeneratedUITestConfig.swift"
HOST_PORT_FILE="/tmp/boo-ios-ui-tests.env"
SKIP_DAEMON="${BOO_IOS_UI_TEST_SKIP_DAEMON:-0}"

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

if [[ -z "$PORT" ]]; then
  PORT="$(
    python3 -c 'import socket, sys; bind = sys.argv[1]; s = socket.socket(); s.bind((bind, 0)); print(s.getsockname()[1]); s.close()' \
      "$BIND_ADDRESS"
  )"
fi

cleanup() {
  if [[ -n "${PORT:-}" ]]; then
    pgrep -f "dns-sd -R boo on .* (${PORT}) _boo._udp local ${PORT}" | while read -r pid; do
      kill "$pid" >/dev/null 2>&1 || true
    done
  fi
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH"
  rm -f "$HOST_PORT_FILE"
  cat > "$GENERATED_CONFIG" <<'EOF'
enum GeneratedUITestConfig {
    static let host: String? = nil
    static let port: UInt16 = 7351
}
EOF
}
trap cleanup EXIT

cd "$ROOT"
if [[ "$SKIP_DAEMON" == "1" ]]; then
cat > "$GENERATED_CONFIG" <<'EOF'
enum GeneratedUITestConfig {
    static let host: String? = nil
    static let port: UInt16 = 7337
}
EOF
else
cat > "$GENERATED_CONFIG" <<EOF
enum GeneratedUITestConfig {
    static let host: String? = $(printf '%s' "\"$HOST\"")
    static let port: UInt16 = $PORT
}
EOF
cat > "$HOST_PORT_FILE" <<EOF
BOO_UI_TEST_HOST=$HOST
BOO_UI_TEST_PORT=$PORT
EOF
fi
cargo build >/dev/null
if [[ "$SKIP_DAEMON" != "1" ]]; then
  rm -f "$SOCKET_PATH"
  target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" --remote-bind-address "$BIND_ADDRESS" >/tmp/boo-ios-ui-tests.log 2>&1 &
  SERVER_PID=$!
  sleep 1
  if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    cat /tmp/boo-ios-ui-tests.log >&2
    exit 1
  fi
fi

TEST_ARGS=()
if [[ -n "$ONLY_TEST" ]]; then
  TEST_ARGS+=("-only-testing:$ONLY_TEST")
fi

if [[ "$DESTINATION" == *"platform=iOS Simulator"* ]]; then
  BOO_UI_TEST_HOST="$HOST" BOO_UI_TEST_PORT="$PORT" \
    xcodebuild \
    -project ios/Boo.xcodeproj \
    -scheme Boo \
    -destination "$DESTINATION" \
    -derivedDataPath "$DERIVED_DATA" \
    "${TEST_ARGS[@]}" \
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
    "${TEST_ARGS[@]}" \
    test
fi
