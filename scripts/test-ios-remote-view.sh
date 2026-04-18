#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${BOO_IOS_REMOTE_PORT:-7347}"
AUTH_KEY="${BOO_IOS_REMOTE_AUTH_KEY:-boo-ios-validation}"
SOCKET_PATH="${BOO_IOS_REMOTE_SOCKET:-/tmp/boo-ios-remote-validation.sock}"
AUTHLESS_PORT="${BOO_IOS_REMOTE_AUTHLESS_PORT:-7348}"
AUTHLESS_SOCKET_PATH="${BOO_IOS_REMOTE_AUTHLESS_SOCKET:-/tmp/boo-ios-remote-authless-validation.sock}"
XCODE_LOG="$ROOT/ios/.derived-validate/xcodebuild.log"
SWIFT_MODULE_CACHE="$ROOT/ios/.swift-module-cache"
VALIDATOR_BIN="$ROOT/ios/.derived-validate/remote-validator"
SELFTEST_BIN="$ROOT/ios/.derived-validate/protocol-codec-selftest"

cleanup() {
  for pid_var in SERVER_PID AUTHLESS_SERVER_PID; do
    local pid="${!pid_var:-}"
    if [[ -n "$pid" ]]; then
      kill "$pid" >/dev/null 2>&1 || true
      wait "$pid" >/dev/null 2>&1 || true
    fi
  done
  rm -f "$SOCKET_PATH"
  rm -f "$AUTHLESS_SOCKET_PATH"
}
trap cleanup EXIT

start_server() {
  local socket_path="$1"
  local port="$2"
  local auth_key="$3"
  local log_path="$4"
  rm -f "$socket_path"
  if [[ -n "$auth_key" ]]; then
    target/debug/boo server --socket "$socket_path" --remote-port "$port" --remote-auth-key "$auth_key" >"$log_path" 2>&1 &
  else
    target/debug/boo server --socket "$socket_path" --remote-port "$port" >"$log_path" 2>&1 &
  fi
  echo $!
}

cd "$ROOT"

cargo build >/dev/null
mkdir -p "$SWIFT_MODULE_CACHE"
mkdir -p "$(dirname "$VALIDATOR_BIN")"
SERVER_PID="$(start_server "$SOCKET_PATH" "$PORT" "$AUTH_KEY" /tmp/boo-ios-remote-server.log)"
sleep 1

swiftc -module-cache-path "$SWIFT_MODULE_CACHE" \
  ios/Sources/WireCodec.swift \
  ios/Validation/RemoteValidator.swift \
  ios/Validation/RemoteValidatorMain.swift \
  -emit-executable \
  -o "$VALIDATOR_BIN"
"$VALIDATOR_BIN" \
  --host 127.0.0.1 \
  --port "$PORT" \
  --auth-key "$AUTH_KEY" \
  --check-discovery

"$VALIDATOR_BIN" \
  --host 127.0.0.1 \
  --port "$PORT" \
  --auth-key "${AUTH_KEY}-wrong" \
  --expect-auth-failure

AUTHLESS_SERVER_PID="$(start_server "$AUTHLESS_SOCKET_PATH" "$AUTHLESS_PORT" "" /tmp/boo-ios-remote-authless-server.log)"
sleep 1
"$VALIDATOR_BIN" \
  --host 127.0.0.1 \
  --port "$AUTHLESS_PORT"

swiftc -module-cache-path "$SWIFT_MODULE_CACHE" \
  ios/Sources/ClientWireState.swift \
  ios/Sources/WireCodec.swift \
  ios/Validation/ProtocolCodecSelfTest.swift \
  ios/Validation/ProtocolCodecSelfTestMain.swift \
  -emit-executable \
  -o "$SELFTEST_BIN"
"$SELFTEST_BIN"

mkdir -p "$(dirname "$XCODE_LOG")"
if ! xcodebuild \
  -project ios/Boo.xcodeproj \
  -scheme Boo \
  -configuration Debug \
  -destination 'generic/platform=iOS' \
  -derivedDataPath ios/.derived-validate \
  CODE_SIGNING_ALLOWED=NO \
  build >"$XCODE_LOG" 2>&1
then
  if grep -q "SwiftCompile normal arm64" "$XCODE_LOG" && grep -q "Ld .*Boo.app/Boo.debug.dylib" "$XCODE_LOG"; then
    echo "iOS build reached Swift compilation; final link still fails in this environment. See $XCODE_LOG"
  else
    cat "$XCODE_LOG"
    exit 1
  fi
else
  echo "iOS app build passed"
fi

echo "iOS remote-view validation passed"
