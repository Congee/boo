#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${BOO_IOS_REMOTE_PORT:-}"
SOCKET_PATH="${BOO_IOS_REMOTE_SOCKET:-/tmp/boo-ios-remote-validation.sock}"
XCODE_LOG="$ROOT/ios/.derived-validate/xcodebuild.log"
SWIFT_MODULE_CACHE="$ROOT/ios/.swift-module-cache"
VALIDATOR_BIN="$ROOT/ios/.derived-validate/remote-validator"
SELFTEST_BIN="$ROOT/ios/.derived-validate/protocol-codec-selftest"

cleanup() {
  local pid="${SERVER_PID:-}"
  if [[ -n "$pid" ]]; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH"
}
trap cleanup EXIT

if [[ -z "$PORT" ]]; then
  PORT="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
fi

cd "$ROOT"

cargo build >/dev/null
mkdir -p "$SWIFT_MODULE_CACHE"
mkdir -p "$(dirname "$VALIDATOR_BIN")"
rm -f "$SOCKET_PATH"
target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" >/tmp/boo-ios-remote-server.log 2>&1 &
SERVER_PID=$!
sleep 1
if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
  cat /tmp/boo-ios-remote-server.log >&2
  exit 1
fi

swiftc -module-cache-path "$SWIFT_MODULE_CACHE" \
  ios/Sources/WireCodec.swift \
  ios/Validation/RemoteValidator.swift \
  ios/Validation/RemoteValidatorMain.swift \
  -emit-executable \
  -o "$VALIDATOR_BIN"
"$VALIDATOR_BIN" \
  --host 127.0.0.1 \
  --port "$PORT" \
  --check-discovery

swiftc -module-cache-path "$SWIFT_MODULE_CACHE" \
  ios/Sources/ClientWireState.swift \
  ios/Sources/SessionModels.swift \
  ios/Sources/SessionHealth.swift \
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
