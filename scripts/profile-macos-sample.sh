#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${OSTYPE:-}" != darwin* ]]; then
  echo "this script is for macOS only" >&2
  exit 1
fi

SOCKET="${SOCKET:-/tmp/boo-prof.sock}"
OUT="${OUT:-/tmp/boo-sample.txt}"
DURATION="${DURATION:-5}"
INTERVAL_MS="${INTERVAL_MS:-1}"
WORKLOAD="${WORKLOAD:-$'for i in {1..20}; do cat ~/config.json; echo __SEP__; done\r'}"
READY_TIMEOUT="${READY_TIMEOUT:-20}"
BOO_BIN="${BOO_BIN:-target/debug/boo}"

usage() {
  cat <<'EOF'
Usage:
  scripts/profile-macos-sample.sh [--socket PATH] [--out PATH] [--duration SEC] [--interval-ms N] [--workload TEXT] [--boo-bin PATH]

Examples:
  scripts/profile-macos-sample.sh
  scripts/profile-macos-sample.sh --workload $'cat ~/config.json\r'
  scripts/profile-macos-sample.sh --boo-bin target/profiling/boo --duration 8

Notes:
  - Starts exactly one Boo server on the requested socket.
  - Waits until the control socket reports a populated UI snapshot.
  - Fails if readiness or workload injection fails.
  - Always cleans up the launched server and temporary sockets.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --socket)
      SOCKET="$2"
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
    --workload)
      WORKLOAD="$2"
      shift 2
      ;;
    --boo-bin)
      BOO_BIN="$2"
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

cleanup() {
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET" "$SOCKET.stream"
}

trap cleanup EXIT

rm -f "$SOCKET" "$SOCKET.stream" "$OUT"

"$BOO_BIN" server --socket "$SOCKET" >/tmp/boo-profile-server.log 2>&1 &
SERVER_PID=$!

python3 scripts/ui-test-client.py --socket "$SOCKET" wait-ready --timeout "$READY_TIMEOUT" >/dev/null
python3 scripts/ui-test-client.py --socket "$SOCKET" request send-text "text=$WORKLOAD" >/dev/null

sample "$SERVER_PID" "$DURATION" "$INTERVAL_MS" -file "$OUT"
echo "sample saved to $OUT"
