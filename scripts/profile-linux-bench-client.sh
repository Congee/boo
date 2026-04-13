#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${OSTYPE:-}" == darwin* ]]; then
  echo "this script is intended for Linux benchmark runs" >&2
  exit 1
fi

SOCKET="${SOCKET:-/tmp/boo-prof.sock}"
GUI_TEST_SOCKET="${GUI_TEST_SOCKET:-/tmp/boo-gui-input.sock}"
GUI_TEST_STATUS="${GUI_TEST_STATUS:-/tmp/boo-gui-status.txt}"
OUT="${OUT:-/tmp/boo-client-perf.data}"
DURATION="${DURATION:-5}"
READY_TIMEOUT="${READY_TIMEOUT:-20}"
WORKLOAD="${WORKLOAD:-for i in {1..5}; do seq 1 10000; echo __SEP__; done\\r}"
SERVER_BIN="${SERVER_BIN:-target/profiling/boo}"
CLIENT_BIN="${CLIENT_BIN:-target/profiling/boo}"
CLIENT_IMPL="${CLIENT_IMPL:-}"
PROFILE_TARGET="${PROFILE_TARGET:-client}"
PROFILER="${PROFILER:-auto}"

usage() {
  cat <<'EOF'
Usage:
  scripts/profile-linux-bench-client.sh [--socket PATH] [--gui-test-socket PATH] [--out PATH]
                                        [--duration SEC] [--ready-timeout SEC] [--workload TEXT]
                                        [--server-bin PATH] [--client-bin PATH]
                                        [--profile-target client|server] [--profiler auto|perf|none]

Examples:
  scripts/profile-linux-bench-client.sh
  scripts/profile-linux-bench-client.sh --workload 'cat bench/generated/plain-32mb.txt\r'
  scripts/profile-linux-bench-client.sh --profiler perf --duration 8

Notes:
  - Starts one Boo server and one Boo GUI client on isolated sockets.
  - Waits for the control socket and GUI attachment state to become ready.
  - Injects the workload through the control socket.
  - On Linux, video recording is optional and not part of this profiling helper.
  - With --profiler perf, records a perf.data file for the selected process.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --socket)
      SOCKET="$2"
      shift 2
      ;;
    --gui-test-socket)
      GUI_TEST_SOCKET="$2"
      shift 2
      ;;
    --out)
      OUT="$2"
      shift 2
      ;;
    --duration)
      DURATION="$2"
      shift 2
      ;;
    --ready-timeout)
      READY_TIMEOUT="$2"
      shift 2
      ;;
    --workload)
      WORKLOAD="$2"
      shift 2
      ;;
    --server-bin)
      SERVER_BIN="$2"
      shift 2
      ;;
    --client-bin)
      CLIENT_BIN="$2"
      shift 2
      ;;
    --profile-target)
      PROFILE_TARGET="$2"
      shift 2
      ;;
    --profiler)
      PROFILER="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

case "$PROFILE_TARGET" in
  client|server)
    ;;
  *)
    echo "invalid --profile-target: $PROFILE_TARGET" >&2
    exit 1
    ;;
esac

case "$PROFILER" in
  auto)
    if command -v perf >/dev/null 2>&1; then
      PROFILER="perf"
    else
      PROFILER="none"
    fi
    ;;
  perf)
    if ! command -v perf >/dev/null 2>&1; then
      echo "perf is not installed or not in PATH" >&2
      exit 1
    fi
    ;;
  none)
    ;;
  *)
    echo "invalid --profiler: $PROFILER" >&2
    exit 1
    ;;
esac

if [[ ! -x "$SERVER_BIN" ]]; then
  echo "server binary is missing or not executable: $SERVER_BIN" >&2
  exit 1
fi

if [[ ! -x "$CLIENT_BIN" ]]; then
  echo "client binary is missing or not executable: $CLIENT_BIN" >&2
  exit 1
fi

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

rm -f "$SOCKET" "$SOCKET.stream" "$GUI_TEST_SOCKET" "$GUI_TEST_STATUS" "$OUT"

BOO_PROFILE=1 "$SERVER_BIN" server --socket "$SOCKET" >/tmp/boo-profile-linux-server.log 2>&1 &
SERVER_PID=$!

CLIENT_ENV=(
  "BOO_GUI_TEST_SOCKET=$GUI_TEST_SOCKET"
  "BOO_GUI_TEST_STATUS_PATH=$GUI_TEST_STATUS"
  "BOO_PROFILE=1"
)
if [[ -n "$CLIENT_IMPL" ]]; then
  CLIENT_ENV+=("BOO_TERMINAL_BODY_IMPL=$CLIENT_IMPL")
fi

env "${CLIENT_ENV[@]}" "$CLIENT_BIN" --socket "$SOCKET" >/tmp/boo-profile-linux-client.log 2>&1 &
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

PROFILE_PID="$CLIENT_PID"
if [[ "$PROFILE_TARGET" == "server" ]]; then
  PROFILE_PID="$SERVER_PID"
fi

python3 scripts/ui-test-client.py --socket "$SOCKET" request send-text "text=$WORKLOAD" >/dev/null

if [[ "$PROFILER" == "perf" ]]; then
  perf record -g -o "$OUT" -p "$PROFILE_PID" -- sleep "$DURATION"
  echo "perf data saved to $OUT"
else
  sleep "$DURATION"
  echo "no external profiler selected; built-in BOO_PROFILE logs were captured"
fi

echo "server log: /tmp/boo-profile-linux-server.log"
echo "client log: /tmp/boo-profile-linux-client.log"
