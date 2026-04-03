#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_ROOT="${BOO_TEST_CONFIG_ROOT:-/tmp/boo-ui-test}"
CONFIG_DIR="$CONFIG_ROOT/boo"
SOCKET_PATH="${BOO_TEST_SOCKET:-/tmp/boo-ui-test.sock}"
LOG_PATH="${BOO_TEST_LOG:-/tmp/boo-ui-test.log}"
KEEP_RUNNING="${BOO_TEST_KEEP_RUNNING:-0}"

mkdir -p "$CONFIG_DIR"
cat > "$CONFIG_DIR/config.boo" <<EOF
control-socket = $SOCKET_PATH
EOF

rm -f "$SOCKET_PATH" "$LOG_PATH"

cleanup() {
  if [[ -n "${BOO_PID:-}" ]] && kill -0 "$BOO_PID" 2>/dev/null; then
    kill "$BOO_PID" 2>/dev/null || true
    wait "$BOO_PID" 2>/dev/null || true
  fi
}

if [[ "$KEEP_RUNNING" != "1" ]]; then
  trap cleanup EXIT
fi

(
  cd "$ROOT_DIR"
  cargo build >/dev/null
)

(
  cd "$ROOT_DIR"
  XDG_CONFIG_HOME="$CONFIG_ROOT" target/debug/boo >"$LOG_PATH" 2>&1
) &
BOO_PID=$!

SNAPSHOT="$(python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" wait-ready)"
printf '%s\n' "$SNAPSHOT"

if [[ "$KEEP_RUNNING" == "1" ]]; then
  echo "boo still running with pid $BOO_PID" >&2
  wait "$BOO_PID"
fi
