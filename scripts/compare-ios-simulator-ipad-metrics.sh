#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOO_REPO_ROOT="$ROOT"
source "$ROOT/scripts/lib/vt-dylib-env.sh"

usage() {
  cat <<'USAGE'
Usage: bash scripts/compare-ios-simulator-ipad-metrics.sh [options]

Runs the runtime-view iOS UI scenario on an iOS Simulator and, unless skipped,
on a physical iPad/iOS device. It compares simulator-loopback metrics with
real-device LAN metrics without changing router/network settings.

Options:
  --ios-device-id UDID       physical iOS device UDID for the iPad lane
  --team-id TEAM_ID          development team for physical-device signing
  --sim-destination DEST     xcodebuild simulator destination
  --derived-data PATH        derived data root (default: /tmp/boo-ios-compare-derived)
  --output-dir PATH          artifact directory (default: bench/generated/ios-sim-vs-ipad/run-<timestamp>)
  --host HOST                host/IP advertised to the iPad lane (default: en0 IPv4)
  --port PORT                remote daemon port; reused sequentially if supplied
  --time-limit DURATION      iPad xctrace duration (default: 24s)
  --headed-simulator         try to open Simulator.app before the simulator lane
  --simulator-live-session   boot/install/launch the simulator app and keep the server running
  --simulator-live-seconds N keep the live simulator session open for N seconds (default: 300)
  --export-simulator-attachments
                              export XCUITest attachments from result.xcresult (default)
  --no-export-simulator-attachments
                              skip XCUITest attachment export
  --simulator-only           skip the physical-device lane
  --ipad-only                skip the simulator lane
  --vt-lib-dir PATH
  -h, --help
USAGE
}

require_arg() {
  if [[ $# -lt 2 ]]; then
    echo "Missing value for $1" >&2
    usage >&2
    exit 2
  fi
}

IOS_DEVICE_ID=""
TEAM_ID=""
SIM_DESTINATION="platform=iOS Simulator,name=iPad mini (A17 Pro),OS=26.3.1"
DERIVED_DATA="/tmp/boo-ios-compare-derived"
OUTPUT_DIR=""
HOST=""
PORT=""
TIME_LIMIT="24s"
HEADED_SIMULATOR=0
SIMULATOR_LIVE_SESSION=0
SIMULATOR_LIVE_SECONDS=300
EXPORT_SIMULATOR_ATTACHMENTS=1
SIMULATOR_ONLY=0
IPAD_ONLY=0
VT_LIB_DIR="${BOO_VT_LIB_DIR:-${VT_LIB_DIR:-}}"
ONLY_TEST="BooUITests/BooAppLaunchTests/testRuntimeViewThreePaneScreenshotAndTapFocus"
TRACE_INPUT_COMMAND="printf 'BOO_RV_COMPARE 🙂 測試 é\\n'"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ios-device-id)
      require_arg "$@"; IOS_DEVICE_ID="$2"; shift 2 ;;
    --team-id)
      require_arg "$@"; TEAM_ID="$2"; shift 2 ;;
    --sim-destination)
      require_arg "$@"; SIM_DESTINATION="$2"; shift 2 ;;
    --derived-data)
      require_arg "$@"; DERIVED_DATA="$2"; shift 2 ;;
    --output-dir)
      require_arg "$@"; OUTPUT_DIR="$2"; shift 2 ;;
    --host)
      require_arg "$@"; HOST="$2"; shift 2 ;;
    --port)
      require_arg "$@"; PORT="$2"; shift 2 ;;
    --time-limit)
      require_arg "$@"; TIME_LIMIT="$2"; shift 2 ;;
    --headed-simulator)
      HEADED_SIMULATOR=1; shift ;;
    --simulator-live-session)
      SIMULATOR_LIVE_SESSION=1; SIMULATOR_ONLY=1; shift ;;
    --simulator-live-seconds)
      require_arg "$@"; SIMULATOR_LIVE_SECONDS="$2"; shift 2 ;;
    --export-simulator-attachments)
      EXPORT_SIMULATOR_ATTACHMENTS=1; shift ;;
    --no-export-simulator-attachments)
      EXPORT_SIMULATOR_ATTACHMENTS=0; shift ;;
    --simulator-only)
      SIMULATOR_ONLY=1; shift ;;
    --ipad-only)
      IPAD_ONLY=1; shift ;;
    --vt-lib-dir)
      require_arg "$@"; VT_LIB_DIR="$2"; shift 2 ;;
    -h|--help)
      usage; exit 0 ;;
    --)
      shift; break ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2 ;;
  esac
done

if [[ "$SIMULATOR_ONLY" == "1" && "$IPAD_ONLY" == "1" ]]; then
  echo "--simulator-only and --ipad-only are mutually exclusive" >&2
  exit 2
fi
if [[ "$SIMULATOR_ONLY" != "1" && -z "$IOS_DEVICE_ID" ]]; then
  echo "Missing --ios-device-id (or pass --simulator-only)" >&2
  exit 2
fi
if [[ -z "$HOST" && "$SIMULATOR_ONLY" != "1" ]]; then
  HOST="$(ifconfig en0 | awk '/inet / { print $2; exit }')"
fi
if [[ -z "$HOST" && "$SIMULATOR_ONLY" != "1" ]]; then
  echo "Could not determine host address for the iPad lane; pass --host" >&2
  exit 2
fi
if [[ -z "$OUTPUT_DIR" ]]; then
  OUTPUT_DIR="$ROOT/bench/generated/ios-sim-vs-ipad/run-$(date +%Y%m%d-%H%M%S)"
elif [[ "$OUTPUT_DIR" != /* ]]; then
  OUTPUT_DIR="$ROOT/$OUTPUT_DIR"
fi
mkdir -p "$OUTPUT_DIR"

SOCKET_PATH="$OUTPUT_DIR/boo-ios-compare.sock"
SERVER_PID=""
CURRENT_SERVER_LOG=""

cleanup() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH" "${SOCKET_PATH}.stream"
}
trap cleanup EXIT

choose_port() {
  local bind_address="$1"
  if [[ -n "$PORT" ]]; then
    printf '%s\n' "$PORT"
    return
  fi
  python3 -c 'import socket, sys; s=socket.socket(); s.bind((sys.argv[1], 0)); print(s.getsockname()[1]); s.close()' "$bind_address"
}

start_server() {
  local bind_address="$1"
  local port="$2"
  local log_path="$3"
  cleanup
  CURRENT_SERVER_LOG="$log_path"
  rm -f "$SOCKET_PATH" "${SOCKET_PATH}.stream"
  (
    cd "$ROOT"
    if [[ -n "$VT_LIB_DIR" ]]; then
      BOO_VT_LIB_DIR="$VT_LIB_DIR"
    fi
    boo_with_vt_lib_env target/debug/boo \
      --profiling \
      --trace-filter info \
      server \
      --socket "$SOCKET_PATH" \
      --remote-port "$port" \
      --remote-bind-address "$bind_address" \
      >"$log_path" 2>&1
  ) &
  SERVER_PID=$!
}

wait_server_ready() {
  local log_path="$1"
  if ! python3 "$ROOT/scripts/ui-test-client.py" --socket "$SOCKET_PATH" wait-ready --timeout 30 >"${log_path%.log}-ready.json"; then
    echo "Boo server did not become ready; server log follows:" >&2
    cat "$log_path" >&2 || true
    return 1
  fi
}

stop_server() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  SERVER_PID=""
  rm -f "$SOCKET_PATH" "${SOCKET_PATH}.stream"
}

simulator_name_from_destination() {
  python3 - "$SIM_DESTINATION" <<'PY'
import re, sys
match = re.search(r'(?:^|,)name=([^,]+)', sys.argv[1])
print(match.group(1) if match else '')
PY
}

simulator_udid_from_destination() {
  local sim_name="$1"
  local devices_json
  devices_json="$(xcrun simctl list devices -j)"
  python3 - "$sim_name" "$devices_json" <<'PY'
import json, sys
wanted = sys.argv[1]
data = json.loads(sys.argv[2])
devices = []
for runtimes in data.get('devices', {}).values():
    for device in runtimes:
        if device.get('isAvailable', True):
            devices.append(device)
if wanted:
    for device in devices:
        if device.get('name') == wanted:
            print(device.get('udid', ''))
            raise SystemExit(0)
if devices:
    print(devices[0].get('udid', ''))
PY
}

preflight_simulator() {
  local output_dir="$1"
  local stdout_path="$output_dir/simctl-list-devices.json"
  local stderr_path="$output_dir/simctl-list-devices.stderr"

  if python3 - "$stdout_path" "$stderr_path" <<'PY'
from pathlib import Path
import subprocess
import sys

stdout_path = Path(sys.argv[1])
stderr_path = Path(sys.argv[2])
try:
    result = subprocess.run(
        ["xcrun", "simctl", "list", "devices", "-j"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=30,
        check=False,
    )
except subprocess.TimeoutExpired as exc:
    stdout_path.write_bytes(exc.stdout or b"")
    stderr_path.write_bytes((exc.stderr or b"") + b"\nsimctl preflight timed out after 30s\n")
    raise SystemExit(124)

stdout_path.write_bytes(result.stdout)
stderr_path.write_bytes(result.stderr)
raise SystemExit(result.returncode)
PY
  then
    return 0
  fi

  cat >&2 <<EOF_PREFLIGHT
CoreSimulator is unhealthy before the Boo simulator lane starts.

'xcrun simctl list devices -j' failed, so xcodebuild/ibtool cannot discover
simulator runtimes either. This is not caused by the Boo server: the server is
not required for this preflight.

Artifacts:
  stdout: $stdout_path
  stderr: $stderr_path

Manual-only local repair from a normal Terminal with Full Disk Access.
This script prints these commands for diagnosis; it does not run them:
  killall Xcode Simulator 2>/dev/null || true
  xcrun simctl list devices

If simctl still reports CoreSimulatorService/simdiskimaged failures, repair the
system CoreSimulator runtime image state from a normal Terminal. Common repairs
for current Xcode/CoreSimulator image-state corruption are:
  xcodebuild -runFirstLaunch -checkForNewerComponents
  sudo rm /Library/Developer/CoreSimulator/Images/images.plist
  sudo rm -rf /Library/Developer/CoreSimulator/Cryptex
  reboot

After reboot, verify:
  xcrun simctl list devices
  xcrun simctl list runtimes

EOF_PREFLIGHT
  cat "$stderr_path" >&2 || true
  return 1
}

open_simulator_headed() {
  local udid="$1"
  local app_path
  app_path="$(xcode-select -p)/Applications/Simulator.app"

  if open -b com.apple.iphonesimulator --args -CurrentDeviceUDID "$udid" >/dev/null 2>&1; then
    return 0
  fi
  if [[ -d "$app_path" ]] && open "$app_path" --args -CurrentDeviceUDID "$udid" >/dev/null 2>&1; then
    return 0
  fi

  cat >&2 <<EOF_WARN
warning: could not open Simulator.app for headed mode; continuing with xcodebuild.
warning: visual evidence will still be exported from the UI test result bundle when available.
EOF_WARN
  return 1
}

export_result_attachments() {
  local result_bundle="$1"
  local attachments_dir="$2"

  if [[ "$EXPORT_SIMULATOR_ATTACHMENTS" != "1" ]]; then
    return 0
  fi
  if [[ ! -d "$result_bundle" ]]; then
    return 0
  fi

  rm -rf "$attachments_dir"
  mkdir -p "$attachments_dir"
  if xcrun xcresulttool export attachments \
    --path "$result_bundle" \
    --output-path "$attachments_dir" \
    >"$attachments_dir/export.log" 2>&1; then
    return 0
  fi

  echo "warning: failed to export simulator XCUITest attachments from $result_bundle" >&2
  cat "$attachments_dir/export.log" >&2 || true
  return 0
}

run_simulator_live_session() {
  local sim_udid="$1"
  local sim_port="$2"
  local sim_dir="$3"
  local server_log="$4"
  local app_path="$DERIVED_DATA/simulator/Build/Products/Debug-iphonesimulator/Boo.app"

  xcodebuild \
    -project ios/Boo.xcodeproj \
    -scheme Boo \
    -destination "$SIM_DESTINATION" \
    -derivedDataPath "$DERIVED_DATA/simulator" \
    -parallel-testing-enabled NO \
    build \
    >"$sim_dir/xcodebuild-live.log" 2>&1

  xcrun simctl install "$sim_udid" "$app_path"
  wait_server_ready "$server_log"
  xcrun simctl launch --terminate-running-process "$sim_udid" me.congee.boo \
    -ApplePersistenceIgnoreState YES \
    --boo-ui-test-mode \
    --boo-ui-test-reset-storage \
    --boo-ui-test-node-name="Local Boo" \
    --boo-ui-test-host=127.0.0.1 \
    --boo-ui-test-port="$sim_port" \
    --boo-ui-test-auto-connect \
    >"$sim_dir/simctl-launch.log" 2>&1

  cat <<EOF_LIVE
iOS simulator Boo live session is running:
  server socket: $SOCKET_PATH
  remote target: 127.0.0.1:$sim_port
  app: me.congee.boo
  server log: $sim_dir/boo-server.log
  app launch log: $sim_dir/simctl-launch.log
  duration: ${SIMULATOR_LIVE_SECONDS}s

Press Ctrl-C in this terminal to stop early and clean up.
EOF_LIVE
  sleep "$SIMULATOR_LIVE_SECONDS"
}

write_metrics() {
  local source_name="$1"
  local input_kind="$2"
  local input_path="$3"
  local output_path="$4"
  local status="${5:-0}"
  python3 - "$source_name" "$input_kind" "$input_path" "$output_path" "$status" <<'PY'
from __future__ import annotations
import json, re, sys
from pathlib import Path

source_name, input_kind, input_path, output_path, status = sys.argv[1:]
path = Path(input_path)
text = path.read_text(encoding='utf-8', errors='ignore') if path.exists() else ''
events = [
    'remote.connect',
    'remote.runtime_action',
    'remote.noop_roundtrip',
    'remote.action_ack',
    'remote.optimistic_apply',
    'remote.reconcile',
    'remote.focus_pane',
    'remote.set_viewed_tab',
    'remote.resize_split',
    'remote.input',
    'remote.pane_update',
    'remote.render_apply',
    'remote.render_ack',
    'remote.heartbeat_rtt',
]

def percentile(values, p):
    if not values:
        return None
    ordered = sorted(values)
    idx = int(round((len(ordered) - 1) * p))
    return round(ordered[idx], 3)

def summarize(values):
    return {
        'count': len(values),
        'min': round(min(values), 3) if values else None,
        'p50': percentile(values, 0.50),
        'p90': percentile(values, 0.90),
        'p95': percentile(values, 0.95),
        'p99': percentile(values, 0.99),
        'max': round(max(values), 3) if values else None,
        'avg': round(sum(values) / len(values), 3) if values else None,
    }

records = []
if input_kind == 'xml':
    records = re.findall(r'fmt="([^"]*remote\.[^"]*)"', text)
else:
    records = [line for line in text.splitlines() if 'remote.' in line]

summary = {}
for event in events:
    event_records = [record for record in records if event in record]
    values = []
    for record in event_records:
        match = re.search(r'elapsed_ms=([0-9]+(?:\.[0-9]+)?)', record)
        if match:
            values.append(float(match.group(1)))
    stats = summarize(values)
    stats['log_count'] = len(event_records)
    summary[event] = stats

Path(output_path).write_text(json.dumps({
    'source': source_name,
    'status': int(status),
    'input_kind': input_kind,
    'input_path': str(path),
    'events': summary,
}, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
PY
}

write_comparison() {
  local output_path="$1"
  shift
  python3 - "$output_path" "$@" <<'PY'
from __future__ import annotations
import json, sys
from pathlib import Path

out = Path(sys.argv[1])
entries = []
for label, metrics_path in zip(sys.argv[2::2], sys.argv[3::2]):
    path = Path(metrics_path)
    if path.exists():
        entries.append((label, json.loads(path.read_text(encoding='utf-8'))))

events = [
    'remote.heartbeat_rtt',
    'remote.noop_roundtrip',
    'remote.action_ack',
    'remote.optimistic_apply',
    'remote.reconcile',
    'remote.input',
    'remote.pane_update',
    'remote.render_apply',
    'remote.render_ack',
    'remote.focus_pane',
    'remote.set_viewed_tab',
]
lines = [
    '# iPad vs iOS Simulator Runtime-View Metrics',
    '',
    'Simulator uses loopback (`127.0.0.1`) on the Mac. iPad uses the LAN path to the same Mac server binary. Each lane gets a fresh server process so runtime state does not leak across runs. Large gaps between the two isolate network/radio/device scheduling from the shared Boo protocol path.',
    '',
    '| lane | status | event | log count | samples | avg ms | p50 ms | p90 ms | p95 ms | p99 ms | max ms |',
    '| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
]
for label, data in entries:
    for event in events:
        stats = data.get('events', {}).get(event, {})
        lines.append(
            f"| {label} | {data.get('status')} | {event} | {stats.get('log_count')} | {stats.get('count')} | {stats.get('avg')} | {stats.get('p50')} | {stats.get('p90')} | {stats.get('p95')} | {stats.get('p99')} | {stats.get('max')} |"
        )
lines.extend(['', '## Artifacts', ''])
for label, data in entries:
    lines.append(f"- {label}: `{data.get('input_path')}`")
out.write_text('\n'.join(lines) + '\n', encoding='utf-8')
PY
}

cd "$ROOT"
if [[ -n "$VT_LIB_DIR" ]]; then
  BOO_VT_LIB_DIR="$VT_LIB_DIR"
fi
cargo build >/dev/null

SIM_METRICS=""
IPAD_METRICS=""
SIM_STATUS=0
IPAD_STATUS=0

if [[ "$IPAD_ONLY" != "1" ]]; then
  SIM_DIR="$OUTPUT_DIR/simulator"
  mkdir -p "$SIM_DIR"
  SIM_PORT="$(choose_port 127.0.0.1)"
  start_server 127.0.0.1 "$SIM_PORT" "$SIM_DIR/boo-server.log"
  preflight_simulator "$SIM_DIR"
  SIM_NAME="$(simulator_name_from_destination)"
  SIM_UDID="$(simulator_udid_from_destination "$SIM_NAME")"
  if [[ -z "$SIM_UDID" ]]; then
    echo "Could not find simulator for destination: $SIM_DESTINATION" >&2
    exit 1
  fi
  xcrun simctl boot "$SIM_UDID" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$SIM_UDID" -b >/dev/null
  if [[ "$HEADED_SIMULATOR" == "1" ]]; then
    open_simulator_headed "$SIM_UDID" || true
  fi
  if [[ "$SIMULATOR_LIVE_SESSION" == "1" ]]; then
    run_simulator_live_session "$SIM_UDID" "$SIM_PORT" "$SIM_DIR" "$SIM_DIR/boo-server.log"
  fi
  SIM_START="$(date '+%Y-%m-%d %H:%M:%S')"
  wait_server_ready "$SIM_DIR/boo-server.log"
  set +e
  bash "$ROOT/scripts/test-ios-ui.sh" \
    --socket "$SOCKET_PATH" \
    --port "$SIM_PORT" \
    --host 127.0.0.1 \
    --destination "$SIM_DESTINATION" \
    --derived-data "$DERIVED_DATA/simulator" \
    --result-bundle "$SIM_DIR/result.xcresult" \
    --skip-daemon \
    --only-testing "$ONLY_TEST" \
    >"$SIM_DIR/xcodebuild.log" 2>&1
  SIM_STATUS=$?
  set -e
  export_result_attachments "$SIM_DIR/result.xcresult" "$SIM_DIR/attachments"
  SIM_END="$(date '+%Y-%m-%d %H:%M:%S')"
  xcrun simctl boot "$SIM_UDID" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$SIM_UDID" -b >/dev/null
  if xcrun simctl spawn "$SIM_UDID" log show \
    --style compact \
    --info \
    --debug \
    --predicate 'subsystem == "dev.boo.ios" AND category == "latency"' \
    --start "$SIM_START" \
    --end "$SIM_END" \
    >"$SIM_DIR/os-log.txt" 2>"$SIM_DIR/log-show.stderr"; then
    write_metrics simulator text "$SIM_DIR/os-log.txt" "$SIM_DIR/metrics.json" "$SIM_STATUS"
  else
    echo "warning: simulator OSLog collection failed; falling back to server trace log" >&2
    write_metrics simulator-server-fallback text "$SIM_DIR/boo-server.log" "$SIM_DIR/metrics.json" "$SIM_STATUS"
  fi
  SIM_METRICS="$SIM_DIR/metrics.json"
  stop_server
fi

if [[ "$SIMULATOR_ONLY" != "1" ]]; then
  IPAD_DIR="$OUTPUT_DIR/ipad"
  mkdir -p "$IPAD_DIR"
  IPAD_PORT="$(choose_port 0.0.0.0)"
  start_server 0.0.0.0 "$IPAD_PORT" "$IPAD_DIR/boo-server.log"
  IPAD_ARGS=(
    --device-id "$IOS_DEVICE_ID"
    --output-dir "$IPAD_DIR"
    --scenario runtime-view-e2e
    --trace-actions runtime-view-e2e,input
    --trace-input-command "$TRACE_INPUT_COMMAND"
    --host "$HOST"
    --port "$IPAD_PORT"
    --socket "$SOCKET_PATH"
    --time-limit "$TIME_LIMIT"
    --use-existing-server
  )
  if [[ -n "$TEAM_ID" ]]; then
    IPAD_ARGS+=(--team-id "$TEAM_ID")
  fi
  if [[ -n "$VT_LIB_DIR" ]]; then
    IPAD_ARGS+=(--vt-lib-dir "$VT_LIB_DIR")
  fi
  wait_server_ready "$IPAD_DIR/boo-server.log"
  set +e
  bash "$ROOT/scripts/verify-ios-signposts.sh" "${IPAD_ARGS[@]}" >"$IPAD_DIR/verify-ios-signposts.log" 2>&1
  IPAD_STATUS=$?
  set -e
  write_metrics ipad xml "$IPAD_DIR/os-log.xml" "$IPAD_DIR/metrics.json" "$IPAD_STATUS"
  IPAD_METRICS="$IPAD_DIR/metrics.json"
  stop_server
fi

COMPARISON_ARGS=()
if [[ -n "$SIM_METRICS" ]]; then
  COMPARISON_ARGS+=(simulator "$SIM_METRICS")
fi
if [[ -n "$IPAD_METRICS" ]]; then
  COMPARISON_ARGS+=(ipad "$IPAD_METRICS")
fi
write_comparison "$OUTPUT_DIR/comparison.md" "${COMPARISON_ARGS[@]}"

cat <<EOF_SUMMARY
iOS simulator vs iPad metrics artifacts:
  output: $OUTPUT_DIR
  comparison: $OUTPUT_DIR/comparison.md
EOF_SUMMARY
if [[ -n "$SIM_METRICS" ]]; then
  echo "  simulator metrics: $SIM_METRICS"
  if [[ -d "$OUTPUT_DIR/simulator/attachments" ]]; then
    echo "  simulator attachments: $OUTPUT_DIR/simulator/attachments"
  fi
fi
if [[ -n "$IPAD_METRICS" ]]; then
  echo "  iPad metrics: $IPAD_METRICS"
fi

if [[ "$SIM_STATUS" != "0" || "$IPAD_STATUS" != "0" ]]; then
  echo "One or more lanes failed: simulator=$SIM_STATUS iPad=$IPAD_STATUS" >&2
  exit 1
fi
