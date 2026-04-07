#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${OSTYPE:-}" != darwin* ]]; then
  echo "this script is for macOS only" >&2
  exit 1
fi

TRACE_OUT="${TRACE_OUT:-/tmp/boo-time.trace}"
SOCKET="${SOCKET:-/tmp/boo-prof.sock}"
TEMPLATE="${TEMPLATE:-Time Profiler}"
TIME_LIMIT="${TIME_LIMIT:-10s}"
WORKLOAD="${WORKLOAD:-}"
READY_TIMEOUT="${READY_TIMEOUT:-20}"

usage() {
  cat <<'EOF'
Usage:
  scripts/profile-macos-instruments.sh [--trace PATH] [--socket PATH] [--template NAME] [--time-limit DUR] [--ready-timeout SEC] [--workload TEXT]

Examples:
  cargo build --profile profiling
  scripts/profile-macos-instruments.sh --workload $'cat ~/config.json\r'
  scripts/profile-macos-instruments.sh --template 'System Trace' --time-limit 15s

Notes:
  - Uses scripts/profiling-boo.sh so the profiling build can find libghostty-vt.dylib.
  - If --workload is set, the script waits for the control socket to report a populated
    UI snapshot and then injects the given terminal text through scripts/ui-test-client.py.
  - The output trace is overwritten by removing any existing trace path first.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --trace)
      TRACE_OUT="$2"
      shift 2
      ;;
    --socket)
      SOCKET="$2"
      shift 2
      ;;
    --template)
      TEMPLATE="$2"
      shift 2
      ;;
    --time-limit)
      TIME_LIMIT="$2"
      shift 2
      ;;
    --workload)
      WORKLOAD="$2"
      shift 2
      ;;
    --ready-timeout)
      READY_TIMEOUT="$2"
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

cleanup() {
  rm -f "$SOCKET" "$SOCKET.stream"
}

cleanup
trap cleanup EXIT

rm -rf "$TRACE_OUT"

xcrun xctrace record \
  --template "$TEMPLATE" \
  --output "$TRACE_OUT" \
  --time-limit "$TIME_LIMIT" \
  --launch -- \
  "$PWD/scripts/profiling-boo.sh" server --socket "$SOCKET" &
TRACE_PID=$!

if [[ -n "$WORKLOAD" ]]; then
  python3 scripts/ui-test-client.py --socket "$SOCKET" wait-ready --timeout "$READY_TIMEOUT" >/dev/null 2>&1 || true
  python3 scripts/ui-test-client.py --socket "$SOCKET" request send-text "text=$WORKLOAD" >/dev/null 2>&1 || true
fi

wait "$TRACE_PID"

echo "trace saved to $TRACE_OUT"
