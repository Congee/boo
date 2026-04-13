#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${OSTYPE:-}" != darwin* ]]; then
  echo "this script is for macOS only" >&2
  exit 1
fi

if [[ $# -lt 2 ]]; then
  cat <<'EOF' >&2
Usage:
  bash scripts/record-bench-scenario.sh <scenario> <output.mp4> [duration-sec]

Examples:
  bash scripts/record-bench-scenario.sh plain-cat /tmp/boo-plain.mp4 10
  bash scripts/record-bench-scenario.sh pager-less /tmp/boo-pager.mp4 15

Notes:
  - Starts a local Boo server and GUI client on app-targeted sockets.
  - Injects the scenario command over Boo's control socket.
  - Records the Boo window with scripts/record-macos-window.swift for a fixed duration.
  - Does not rely on the app being frontmost.
EOF
  exit 1
fi

SCENARIO="$1"
OUT_MP4="$2"
DURATION="${3:-10}"

SOCKET="${SOCKET:-/tmp/boo-prof.sock}"
GUI_TEST_SOCKET="${GUI_TEST_SOCKET:-/tmp/boo-gui-input.sock}"
GUI_TEST_STATUS="${GUI_TEST_STATUS:-/tmp/boo-gui-status.txt}"
READY_TIMEOUT="${READY_TIMEOUT:-20}"
BOO_SERVER_BIN="${BOO_SERVER_BIN:-target/debug/boo}"
BOO_CLIENT_BIN="${BOO_CLIENT_BIN:-target/debug/boo}"
BOO_CLIENT_IMPL="${BOO_CLIENT_IMPL:-}"
WINDOW_OWNER="${WINDOW_OWNER:-boo}"

SERVER_PID=""
CLIENT_PID=""

cleanup() {
  if [[ -n "$CLIENT_PID" ]]; then
    kill "$CLIENT_PID" >/dev/null 2>&1 || true
    wait "$CLIENT_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET" "$SOCKET.stream" "$GUI_TEST_SOCKET" "$GUI_TEST_STATUS"
}

trap cleanup EXIT

rm -f "$SOCKET" "$SOCKET.stream" "$GUI_TEST_SOCKET" "$GUI_TEST_STATUS"

COMMAND="$(bash scripts/run-terminal-bench.sh "$SCENARIO" --print-only | awk -F= '/^command=/{print substr($0,9)}')"
if [[ -z "$COMMAND" ]]; then
  echo "failed to resolve scenario command for: $SCENARIO" >&2
  exit 1
fi
WORKLOAD="${COMMAND}\r"

"$BOO_SERVER_BIN" server --socket "$SOCKET" >/tmp/boo-record-bench-server.log 2>&1 &
SERVER_PID=$!

CLIENT_ENV=(
  "BOO_GUI_TEST_SOCKET=$GUI_TEST_SOCKET"
  "BOO_GUI_TEST_STATUS_PATH=$GUI_TEST_STATUS"
)
if [[ -n "$BOO_CLIENT_IMPL" ]]; then
  CLIENT_ENV+=("BOO_TERMINAL_BODY_IMPL=$BOO_CLIENT_IMPL")
fi

env "${CLIENT_ENV[@]}" "$BOO_CLIENT_BIN" --socket "$SOCKET" >/tmp/boo-record-bench-client.log 2>&1 &
CLIENT_PID=$!

python3 scripts/ui-test-client.py --socket "$SOCKET" wait-ready --timeout "$READY_TIMEOUT" >/dev/null

STATUS=""
for _ in $(seq 1 $((READY_TIMEOUT * 10))); do
  if [[ -f "$GUI_TEST_STATUS" ]]; then
    STATUS="$(cat "$GUI_TEST_STATUS")"
  fi
  if [[ "$STATUS" == *"mode=attached"* && "$STATUS" == *"stream_ready=1"* ]]; then
    break
  fi
  sleep 0.1
done

if [[ "${STATUS:-}" != *"mode=attached"* || "${STATUS:-}" != *"stream_ready=1"* ]]; then
  echo "GUI client did not reach attached stream state: ${STATUS:-<none>}" >&2
  exit 1
fi

swift scripts/record-macos-window.swift "$WINDOW_OWNER" "$OUT_MP4" "$DURATION" >/dev/null 2>&1 &
RECORDER_PID=$!
sleep 0.5

python3 scripts/ui-test-client.py --socket "$SOCKET" request send-text "text=$WORKLOAD" >/dev/null

wait "$RECORDER_PID"

printf 'scenario=%s\n' "$SCENARIO"
printf 'output=%s\n' "$OUT_MP4"
