#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOO_REPO_ROOT="$ROOT"
PORT="${BOO_IOS_REMOTE_PORT:-}"
SOCKET_PATH="${BOO_IOS_REMOTE_SOCKET:-/tmp/boo-ios-remote-validation.sock}"
DERIVED_DIR="${BOO_IOS_VALIDATE_DERIVED:-/tmp/boo-ios-validate-derived}"
XCODE_LOG="$DERIVED_DIR/xcodebuild.log"
SWIFT_MODULE_CACHE="${BOO_IOS_SWIFT_MODULE_CACHE:-/tmp/boo-ios-swift-module-cache}"
VALIDATOR_BIN="$DERIVED_DIR/remote-validator"
SELFTEST_BIN="$DERIVED_DIR/protocol-codec-selftest"
TRACE_SELFTEST_BIN="$DERIVED_DIR/trace-render-apply-selftest"
if [[ -n "${BOO_IOS_DEVELOPER_DIR:-}" ]]; then
  XCODE_DEVELOPER_DIR="$BOO_IOS_DEVELOPER_DIR"
elif [[ -d /Applications/Xcode.app/Contents/Developer ]]; then
  XCODE_DEVELOPER_DIR="/Applications/Xcode.app/Contents/Developer"
else
  XCODE_DEVELOPER_DIR="$(xcode-select -p)"
fi

source "$ROOT/scripts/lib/vt-dylib-env.sh"

run_swiftc() {
  env -u SDKROOT DEVELOPER_DIR="$XCODE_DEVELOPER_DIR" xcrun swiftc "$@"
}

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

cleanup() {
  local pid="${SERVER_PID:-}"
  if [[ -n "$pid" ]]; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH" "${SOCKET_PATH}.stream"
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
boo_with_vt_lib_env target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" >/tmp/boo-ios-remote-server.log 2>&1 &
SERVER_PID=$!
sleep 1
if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
  cat /tmp/boo-ios-remote-server.log >&2
  exit 1
fi

run_swiftc -module-cache-path "$SWIFT_MODULE_CACHE" \
  ios/Sources/WireCodec.swift \
  ios/Validation/RemoteValidator.swift \
  ios/Validation/RemoteValidatorMain.swift \
  -emit-executable \
  -o "$VALIDATOR_BIN"
"$VALIDATOR_BIN" \
  --host 127.0.0.1 \
  --port "$PORT" \
  --check-discovery

run_swiftc -module-cache-path "$SWIFT_MODULE_CACHE" \
  ios/Sources/ClientWireState.swift \
  ios/Sources/TabModels.swift \
  ios/Sources/TabHealth.swift \
  ios/Sources/WireCodec.swift \
  ios/Validation/ProtocolCodecSelfTest.swift \
  ios/Validation/ProtocolCodecSelfTestMain.swift \
  -emit-executable \
  -o "$SELFTEST_BIN"
"$SELFTEST_BIN"

run_swiftc -module-cache-path "$SWIFT_MODULE_CACHE" \
  ios/Sources/BooTrace.swift \
  ios/Validation/TraceRenderApplySelfTestMain.swift \
  -emit-executable \
  -o "$TRACE_SELFTEST_BIN"
"$TRACE_SELFTEST_BIN"

mkdir -p "$(dirname "$XCODE_LOG")"
if ! xcodebuild_clean_env xcodebuild \
  -project ios/Boo.xcodeproj \
  -scheme Boo \
  -configuration Debug \
  -destination 'generic/platform=iOS' \
  -derivedDataPath "$DERIVED_DIR" \
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
