#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_ROOT="${BOO_TEST_CONFIG_ROOT:-/tmp/boo-ui-test}"
CONFIG_DIR="$CONFIG_ROOT/boo"
SOCKET_PATH="${BOO_TEST_SOCKET:-/tmp/boo-ui-test.sock}"
KEEP_RUNNING="${BOO_TEST_KEEP_RUNNING:-0}"

mkdir -p "$CONFIG_DIR"
cat > "$CONFIG_DIR/config.boo" <<EOF
control-socket = $SOCKET_PATH
EOF

rm -f "$SOCKET_PATH"

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
  XDG_CONFIG_HOME="$CONFIG_ROOT" cargo run
) &
BOO_PID=$!

for _ in $(seq 1 120); do
  if [[ -S "$SOCKET_PATH" ]]; then
    break
  fi
  sleep 0.1
done

if [[ ! -S "$SOCKET_PATH" ]]; then
  echo "control socket did not appear at $SOCKET_PATH" >&2
  exit 1
fi

SNAPSHOT=""
for _ in $(seq 1 120); do
  SNAPSHOT="$(echo '{"cmd":"get-ui-snapshot"}' | socat - "UNIX-CONNECT:$SOCKET_PATH")"
  if python3 -c 'import json,sys; data=json.load(sys.stdin); raise SystemExit(0 if data["snapshot"]["tabs"] else 1)' <<<"$SNAPSHOT" 2>/dev/null
  then
    break
  fi
  sleep 0.1
done

if ! python3 -c 'import json,sys; data=json.load(sys.stdin); raise SystemExit(0 if data["snapshot"]["tabs"] else 1)' <<<"$SNAPSHOT" 2>/dev/null
then
  echo "ui snapshot never populated" >&2
  exit 1
fi

printf '%s\n' "$SNAPSHOT"

if [[ "$KEEP_RUNNING" == "1" ]]; then
  echo "boo still running with pid $BOO_PID" >&2
  wait "$BOO_PID"
fi
