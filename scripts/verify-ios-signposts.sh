#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOO_REPO_ROOT="$ROOT"
source "$ROOT/scripts/lib/vt-dylib-env.sh"

usage() {
  cat <<'EOF'
Usage: bash scripts/verify-ios-signposts.sh --device-id DEVICE_ID [options]

Records a real-device Instruments Logging trace while launching Boo with
UI-test auto-connect/input arguments, then exports OSLog/OSSignpost tables and
asserts that native Logger/signpost output contains the shared latency names.

Options:
  --device-id DEVICE_ID
  --team-id TEAM_ID
  --derived-data PATH
  --host HOST
  --port PORT
  --socket PATH
  --bundle-id BUNDLE_ID
  --output-dir PATH
  --time-limit DURATION        xctrace duration such as 18s, 1m
  --scenario NAME             named defaults: default, runtime-view-e2e
  --trace-actions ACTIONS      comma-separated: focus-pane,set-viewed-tab,input
  --trace-input-command TEXT   command sent after the terminal connects
  --vt-lib-dir PATH
  --skip-build                 use an existing app build in DerivedData
  --skip-install               use the app already installed on the device
  --skip-device-check          do not run the unlocked/developer-mode preflight
  -h, --help

Environment variable fallbacks remain supported:
  BOO_IOS_DEVICE_ID
  BOO_IOS_TEAM_ID
  BOO_IOS_DERIVED_DATA_PATH
  BOO_VT_LIB_DIR
EOF
}

require_arg() {
  if [[ $# -lt 2 ]]; then
    echo "Missing value for $1" >&2
    usage >&2
    exit 2
  fi
}

DEVICE_ID="${BOO_IOS_DEVICE_ID:-}"
TEAM_ID="${BOO_IOS_TEAM_ID:-}"
DERIVED_DATA="${BOO_IOS_DERIVED_DATA_PATH:-/tmp/boo-ios-derived}"
VT_LIB_DIR="${BOO_VT_LIB_DIR:-${VT_LIB_DIR:-}}"
HOST=""
PORT=""
SOCKET_PATH="/tmp/boo-ios-signpost-verify.sock"
BUNDLE_ID="me.congee.boo"
OUTPUT_DIR=""
TIME_LIMIT="18s"
SCENARIO="default"
TRACE_ACTIONS="focus-pane,set-viewed-tab,input"
TRACE_INPUT_COMMAND="echo BOO_SIGNPOST_VERIFY"
SKIP_BUILD=0
SKIP_INSTALL=0
SKIP_DEVICE_CHECK=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --device-id)
      require_arg "$@"
      DEVICE_ID="$2"
      shift 2
      ;;
    --team-id)
      require_arg "$@"
      TEAM_ID="$2"
      shift 2
      ;;
    --derived-data)
      require_arg "$@"
      DERIVED_DATA="$2"
      shift 2
      ;;
    --host)
      require_arg "$@"
      HOST="$2"
      shift 2
      ;;
    --port)
      require_arg "$@"
      PORT="$2"
      shift 2
      ;;
    --socket)
      require_arg "$@"
      SOCKET_PATH="$2"
      shift 2
      ;;
    --bundle-id)
      require_arg "$@"
      BUNDLE_ID="$2"
      shift 2
      ;;
    --output-dir)
      require_arg "$@"
      OUTPUT_DIR="$2"
      shift 2
      ;;
    --time-limit)
      require_arg "$@"
      TIME_LIMIT="$2"
      shift 2
      ;;
    --scenario)
      require_arg "$@"
      SCENARIO="$2"
      shift 2
      ;;
    --trace-actions)
      require_arg "$@"
      TRACE_ACTIONS="$2"
      shift 2
      ;;
    --trace-input-command)
      require_arg "$@"
      TRACE_INPUT_COMMAND="$2"
      shift 2
      ;;
    --vt-lib-dir)
      require_arg "$@"
      VT_LIB_DIR="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --skip-install)
      SKIP_INSTALL=1
      shift
      ;;
    --skip-device-check)
      SKIP_DEVICE_CHECK=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$DEVICE_ID" ]]; then
  echo "Missing --device-id" >&2
  echo "tip: use bash scripts/list-ios-devices.sh to find a device UDID" >&2
  exit 2
fi
case "$SCENARIO" in
  default)
    ;;
  runtime-view-e2e)
    TRACE_ACTIONS="${TRACE_ACTIONS:-focus-pane,set-viewed-tab,input}"
    if [[ "$TRACE_INPUT_COMMAND" == "echo BOO_SIGNPOST_VERIFY" ]]; then
      TRACE_INPUT_COMMAND="printf \'BOO_RV_E2E_IOS 🙂 測試 é\\n\'"
    fi
    ;;
  *)
    echo "Unknown --scenario: $SCENARIO" >&2
    usage >&2
    exit 2
    ;;
esac

TRACE_ACTIONS="${TRACE_ACTIONS//[[:space:]]/}"

trace_action_enabled() {
  local action="$1"
  [[ ",$TRACE_ACTIONS," == *",$action,"* ]]
}

if [[ -z "$HOST" ]]; then
  HOST="$(ifconfig en0 | awk '/inet / { print $2; exit }')"
fi
if [[ -z "$HOST" ]]; then
  echo "Could not determine host address for the iOS device; pass --host" >&2
  exit 2
fi

if [[ -z "$PORT" ]]; then
  PORT="$(
    python3 -c 'import socket; s = socket.socket(); s.bind(("0.0.0.0", 0)); print(s.getsockname()[1]); s.close()'
  )"
fi

if [[ -z "$OUTPUT_DIR" ]]; then
  OUTPUT_DIR="/tmp/boo-ios-signpost-verify.$(date +%Y%m%d-%H%M%S)"
fi
mkdir -p "$OUTPUT_DIR"

TRACE_PATH="$OUTPUT_DIR/boo-ios-signposts.trace"
XCTRACE_RECORD_LOG="$OUTPUT_DIR/xctrace-record.log"
SERVER_LOG="$OUTPUT_DIR/boo-server.log"
CARGO_BUILD_LOG="$OUTPUT_DIR/cargo-build.log"
TRACE_TOC_XML="$OUTPUT_DIR/trace-toc.xml"
OS_LOG_XML="$OUTPUT_DIR/os-log.xml"
OS_SIGNPOST_XML="$OUTPUT_DIR/os-signpost.xml"
OS_SIGNPOST_INTERVAL_XML="$OUTPUT_DIR/os-signpost-interval.xml"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  xcrun devicectl device process terminate \
    --device "$DEVICE_ID" \
    "$BUNDLE_ID" >/dev/null 2>&1 || true
  rm -f "$SOCKET_PATH" "${SOCKET_PATH}.stream"
}
trap cleanup EXIT

cd "$ROOT"
if [[ -n "$VT_LIB_DIR" ]]; then
  BOO_VT_LIB_DIR="$VT_LIB_DIR"
fi

if [[ "$SKIP_DEVICE_CHECK" != "1" ]]; then
  bash scripts/check-ios-device-state.sh "$DEVICE_ID"
fi

export BOO_IOS_DERIVED_DATA_PATH="$DERIVED_DATA"
if [[ "$SKIP_BUILD" != "1" ]]; then
  if [[ -n "$TEAM_ID" ]]; then
    bash scripts/build-ios-device.sh "$TEAM_ID" "$DEVICE_ID"
  else
    bash scripts/build-ios-device.sh "" "$DEVICE_ID"
  fi
fi

if [[ "$SKIP_INSTALL" != "1" ]]; then
  bash scripts/install-ios-device.sh "$DEVICE_ID"
fi

cargo build >"$CARGO_BUILD_LOG" 2>&1

rm -f "$SOCKET_PATH" "${SOCKET_PATH}.stream"
boo_with_vt_lib_env target/debug/boo \
  --trace-filter info \
  server \
  --socket "$SOCKET_PATH" \
  --remote-port "$PORT" \
  --remote-bind-address 0.0.0.0 \
  >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

sleep 1
if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
  cat "$SERVER_LOG" >&2
  exit 1
fi

rm -rf "$TRACE_PATH"
xcrun xctrace record \
  --template "Logging" \
  --device "$DEVICE_ID" \
  --time-limit "$TIME_LIMIT" \
  --output "$TRACE_PATH" \
  --no-prompt \
  --launch -- "$BUNDLE_ID" \
    -ApplePersistenceIgnoreState YES \
    --boo-ui-test-mode \
    --boo-ui-test-reset-storage \
    --boo-ui-test-node-name="Local Boo" \
    --boo-ui-test-host="$HOST" \
    --boo-ui-test-port="$PORT" \
    --boo-ui-test-auto-connect \
    --boo-ui-test-trace-actions="$TRACE_ACTIONS" \
    --boo-ui-test-trace-input-command="$TRACE_INPUT_COMMAND" \
  2>&1 | tee "$XCTRACE_RECORD_LOG"

xcrun xctrace export \
  --input "$TRACE_PATH" \
  --toc \
  --output "$TRACE_TOC_XML" >/dev/null

xcrun xctrace export \
  --input "$TRACE_PATH" \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="os-log"]' \
  --output "$OS_LOG_XML" >/dev/null

xcrun xctrace export \
  --input "$TRACE_PATH" \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="os-signpost"]' \
  --output "$OS_SIGNPOST_XML" >/dev/null

xcrun xctrace export \
  --input "$TRACE_PATH" \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="os-signpost-interval"]' \
  --output "$OS_SIGNPOST_INTERVAL_XML" >/dev/null

require_trace_text() {
  local file="$1"
  local pattern="$2"
  local description="$3"
  if ! grep -Fq "$pattern" "$file"; then
    echo "Missing $description ($pattern) in $file" >&2
    echo "Artifacts retained in $OUTPUT_DIR" >&2
    exit 1
  fi
}

require_trace_text "$OS_LOG_XML" "dev.boo.ios" "Boo Logger subsystem"
require_trace_text "$OS_LOG_XML" "latency" "Boo Logger category"
require_trace_text "$OS_LOG_XML" "remote.connect" "Logger remote.connect event"
require_trace_text "$OS_LOG_XML" "remote.pane_update" "Logger remote.pane_update event"
require_trace_text "$OS_LOG_XML" "remote.render_apply" "Logger remote.render_apply end event"

require_trace_text "$OS_SIGNPOST_XML" "remote.connect" "OSSignpost remote.connect event"
require_trace_text "$OS_SIGNPOST_XML" "remote.pane_update" "OSSignpost remote.pane_update event"
require_trace_text "$OS_SIGNPOST_XML" "remote.render_apply" "OSSignpost remote.render_apply end metadata"

require_trace_text "$OS_SIGNPOST_INTERVAL_XML" "remote.render_apply" "Instruments remote.render_apply end metadata"

if trace_action_enabled "focus-pane"; then
  require_trace_text "$OS_LOG_XML" "remote.focus_pane" "Logger remote.focus_pane begin event"
  require_trace_text "$OS_LOG_XML" "source_event= remote.focus_pane" "Logger remote.focus_pane render end"
  require_trace_text "$OS_SIGNPOST_XML" "remote.focus_pane" "OSSignpost remote.focus_pane interval events"
  require_trace_text "$OS_SIGNPOST_INTERVAL_XML" "remote.focus_pane" "Instruments remote.focus_pane signpost interval"
fi

if trace_action_enabled "set-viewed-tab"; then
  require_trace_text "$OS_LOG_XML" "remote.set_viewed_tab" "Logger remote.set_viewed_tab begin event"
  require_trace_text "$OS_LOG_XML" "source_event= remote.set_viewed_tab" "Logger remote.set_viewed_tab render end"
  require_trace_text "$OS_SIGNPOST_XML" "remote.set_viewed_tab" "OSSignpost remote.set_viewed_tab interval events"
  require_trace_text "$OS_SIGNPOST_INTERVAL_XML" "remote.set_viewed_tab" "Instruments remote.set_viewed_tab signpost interval"
fi

if trace_action_enabled "input"; then
  require_trace_text "$OS_LOG_XML" "remote.input" "Logger remote.input begin event"
  require_trace_text "$OS_LOG_XML" "source_event= remote.input" "Logger remote.input render end"
  require_trace_text "$OS_SIGNPOST_XML" "remote.input" "OSSignpost remote.input interval events"
  require_trace_text "$OS_SIGNPOST_INTERVAL_XML" "remote.input" "Instruments remote.input signpost interval"
fi

cat <<EOF
iOS signpost verification passed.

Artifacts:
  trace: $TRACE_PATH
  toc: $TRACE_TOC_XML
  os log: $OS_LOG_XML
  os signpost: $OS_SIGNPOST_XML
  os signpost intervals: $OS_SIGNPOST_INTERVAL_XML
  server log: $SERVER_LOG
EOF
