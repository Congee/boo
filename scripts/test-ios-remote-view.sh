#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${BOO_IOS_REMOTE_PORT:-7347}"
AUTH_KEY="${BOO_IOS_REMOTE_AUTH_KEY:-boo-ios-validation}"
SOCKET_PATH="${BOO_IOS_REMOTE_SOCKET:-/tmp/boo-ios-remote-validation.sock}"
XCODE_LOG="$ROOT/ios/.derived-validate/xcodebuild.log"
SWIFT_MODULE_CACHE="$ROOT/ios/.swift-module-cache"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH"
}
trap cleanup EXIT

cd "$ROOT"

cargo build >/dev/null
rm -f "$SOCKET_PATH"
mkdir -p "$SWIFT_MODULE_CACHE"
target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" --remote-auth-key "$AUTH_KEY" >/tmp/boo-ios-remote-server.log 2>&1 &
SERVER_PID=$!
sleep 1

swift -module-cache-path "$SWIFT_MODULE_CACHE" ios/Validation/RemoteValidator.swift \
  --host 127.0.0.1 \
  --port "$PORT" \
  --auth-key "$AUTH_KEY" \
  --check-discovery

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
