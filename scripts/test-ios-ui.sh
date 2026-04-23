#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_PATH="${BOO_IOS_UI_TEST_SOCKET:-/tmp/boo-ios-ui-tests.sock}"
PORT="${BOO_IOS_UI_TEST_PORT:-}"
DESTINATION="${BOO_IOS_UI_TEST_DESTINATION:-platform=iOS Simulator,name=iPad mini (A17 Pro),OS=26.3.1}"
DERIVED_DATA="${BOO_IOS_UI_TEST_DERIVED_DATA:-/tmp/boo-ios-derived}"
HOST="${BOO_IOS_UI_TEST_HOST:-}"
TEAM_ID="${BOO_IOS_TEAM_ID:-}"
ONLY_TEST="${BOO_IOS_UI_TEST_ONLY:-}"
HOST_PORT_FILE="/tmp/boo-ios-ui-tests.env"
SKIP_DAEMON="${BOO_IOS_UI_TEST_SKIP_DAEMON:-0}"
XCODEBUILD_LOG="/tmp/boo-ios-ui-tests.xcodebuild.log"

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
  pkill -f "target/debug/boo server --socket ${SOCKET_PATH}" >/dev/null 2>&1 || true
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
  rm -f "$XCODEBUILD_LOG"
}
trap cleanup EXIT

cd "$ROOT"
pkill -f "target/debug/boo server --socket ${SOCKET_PATH}" >/dev/null 2>&1 || true
if [[ "$SKIP_DAEMON" != "1" ]]; then
  cat > "$HOST_PORT_FILE" <<EOF
BOO_UI_TEST_HOST=$HOST
BOO_UI_TEST_PORT=$PORT
EOF
fi
cargo build >/dev/null
if [[ "$SKIP_DAEMON" != "1" ]]; then
  rm -f "$SOCKET_PATH"
  RUST_LOG=info target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" --remote-bind-address "$BIND_ADDRESS" >/tmp/boo-ios-ui-tests.log 2>&1 &
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

run_xcodebuild() {
  set +e
  xcodebuild_clean_env "$@" 2>&1 | tee "$XCODEBUILD_LOG"
  local status=${PIPESTATUS[0]}
  set -e

  if [[ "$status" -ne 0 ]]; then
    if grep -Fq "Timed out while enabling automation mode" "$XCODEBUILD_LOG"; then
      cat >&2 <<'EOF'

real-device UI testing reached the test runner, but the device did not enter automation mode.
On the device, verify:
- Settings > Developer > Enable UI Automation is ON
- the device stays unlocked during the run

Then rerun scripts/test-ios-ui.sh.
EOF
    elif grep -Fq "The developer disk image could not be mounted on this device." "$XCODEBUILD_LOG"; then
      cat >&2 <<'EOF'

the target device is connected, but Xcode could not mount its developer disk image.
Open Xcode > Window > Devices and Simulators, select the device, and let Xcode finish
any required support-file / developer-disk-image setup before rerunning this script.
EOF
    fi
  fi

  return "$status"
}

if [[ "$DESTINATION" == *"platform=iOS Simulator"* ]]; then
  BOO_UI_TEST_HOST="$HOST" BOO_UI_TEST_PORT="$PORT" \
    run_xcodebuild xcodebuild \
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
    run_xcodebuild xcodebuild \
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
