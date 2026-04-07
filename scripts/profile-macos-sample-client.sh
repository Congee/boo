#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${OSTYPE:-}" != darwin* ]]; then
  echo "this script is for macOS only" >&2
  exit 1
fi

SOCKET="${SOCKET:-/tmp/boo-prof.sock}"
GUI_TEST_SOCKET="${GUI_TEST_SOCKET:-/tmp/boo-gui-input.sock}"
OUT="${OUT:-/tmp/boo-client-sample.txt}"
DURATION="${DURATION:-5}"
INTERVAL_MS="${INTERVAL_MS:-1}"
READY_TIMEOUT="${READY_TIMEOUT:-20}"
WORKLOAD="${WORKLOAD:-$'for i in {1..5}; do seq 1 10000; echo __SEP__; done\r'}"
SERVER_BIN="${SERVER_BIN:-scripts/profiling-boo.sh}"
CLIENT_BIN="${CLIENT_BIN:-scripts/profiling-boo.sh}"

usage() {
  cat <<'EOF'
Usage:
  scripts/profile-macos-sample-client.sh [--socket PATH] [--gui-test-socket PATH] [--out PATH]
                                        [--duration SEC] [--interval-ms N] [--ready-timeout SEC]
                                        [--workload TEXT] [--server-bin PATH] [--client-bin PATH]

Examples:
  scripts/profile-macos-sample-client.sh
  scripts/profile-macos-sample-client.sh --workload $'cat ~/config.json\r'
  scripts/profile-macos-sample-client.sh --client-bin target/debug/boo

Notes:
  - Starts one Boo server and one Boo GUI client.
  - Waits for the control socket and GUI test socket to become ready.
  - Injects workload through the client-side GUI test socket, then samples the GUI process.
  - Always cleans up the launched processes and temporary sockets.
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
    --interval-ms)
      INTERVAL_MS="$2"
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
  rm -f "$SOCKET" "$SOCKET.stream" "$GUI_TEST_SOCKET"
}

trap cleanup EXIT

rm -f "$SOCKET" "$SOCKET.stream" "$GUI_TEST_SOCKET" "$OUT"

"$SERVER_BIN" server --socket "$SOCKET" >/tmp/boo-profile-client-server.log 2>&1 &
SERVER_PID=$!

BOO_GUI_TEST_SOCKET="$GUI_TEST_SOCKET" "$CLIENT_BIN" --socket "$SOCKET" >/tmp/boo-profile-client-gui.log 2>&1 &
CLIENT_PID=$!

python3 scripts/ui-test-client.py --socket "$SOCKET" wait-ready --timeout "$READY_TIMEOUT" >/dev/null

for _ in $(seq 1 $((READY_TIMEOUT * 10))); do
  if [[ -S "$GUI_TEST_SOCKET" ]]; then
    break
  fi
  sleep 0.1
done

if [[ ! -S "$GUI_TEST_SOCKET" ]]; then
  echo "GUI test socket did not become ready: $GUI_TEST_SOCKET" >&2
  exit 1
fi

python3 - <<'PY' "$GUI_TEST_SOCKET" "$WORKLOAD"
import socket, sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(sys.argv[1])
sock.sendall(f"text {sys.argv[2]}\n".encode())
sock.close()
PY

sample "$CLIENT_PID" "$DURATION" "$INTERVAL_MS" -file "$OUT"
echo "sample saved to $OUT"
