#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOO_REPO_ROOT="$ROOT"
SOCKET_PATH="${BOO_REMOTE_UPGRADE_SOCKET:-/tmp/boo-remote-upgrade.sock}"
LOG_PATH="${BOO_REMOTE_UPGRADE_LOG:-/tmp/boo-remote-upgrade.log}"
VT_LIB_DIR="${BOO_VT_LIB_DIR:-${VT_LIB_DIR:-}}"
PORT="${BOO_REMOTE_UPGRADE_PORT:-}"

source "$ROOT/scripts/lib/vt-dylib-env.sh"

usage() {
  cat <<'EOF'
Usage: bash scripts/test-remote-upgrade-flow.sh [options]

Options:
  --socket PATH
  --port PORT
  --log PATH
  --vt-lib-dir PATH
  -h, --help
EOF
}

require_arg() {
  if [[ $# -lt 2 ]]; then
    echo "Missing value for $1" >&2
    usage >&2
    exit 2
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --socket)
      require_arg "$@"
      SOCKET_PATH="$2"
      shift 2
      ;;
    --port)
      require_arg "$@"
      PORT="$2"
      shift 2
      ;;
    --log)
      require_arg "$@"
      LOG_PATH="$2"
      shift 2
      ;;
    --vt-lib-dir)
      require_arg "$@"
      VT_LIB_DIR="$2"
      shift 2
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

if [[ -n "$VT_LIB_DIR" && -z "${BOO_VT_LIB_DIR:-}" ]]; then
  BOO_VT_LIB_DIR="$VT_LIB_DIR"
fi

if [[ -z "$PORT" ]]; then
  PORT="$(
    python3 - <<'PY'
import socket

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
  )"
fi

cleanup() {
  local pid="${SERVER_PID:-}"
  if [[ -n "$pid" ]]; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH"
}
trap cleanup EXIT

cd "$ROOT"

cargo build >/dev/null
rm -f "$SOCKET_PATH"
boo_with_vt_lib_env target/debug/boo server \
  --socket "$SOCKET_PATH" \
  --remote-port "$PORT" \
  --remote-bind-address 0.0.0.0 \
  >"$LOG_PATH" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 40); do
  if ./target/debug/boo probe-remote-daemon --host 127.0.0.1 --port "$PORT" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

for _ in $(seq 1 40); do
  if ./target/debug/boo remote-clients --socket "$SOCKET_PATH" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

upgrade_json="$(./target/debug/boo remote-upgrade-target --host 127.0.0.1 --socket "$SOCKET_PATH")"
upgrade_fields="$(
  python3 - "$upgrade_json" "$PORT" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
expected_port = int(sys.argv[2])

if data.get("ssh_host") != "127.0.0.1":
    raise SystemExit(f"unexpected ssh_host: {data.get('ssh_host')!r}")
if data.get("upgrade_ready") is not True:
    raise SystemExit(f"expected upgrade_ready true, got {data.get('upgrade_ready')!r}")
if data.get("selected_transport") != "quic-direct":
    raise SystemExit(f"unexpected selected_transport: {data.get('selected_transport')!r}")
if data.get("direct_host") != "127.0.0.1":
    raise SystemExit(f"unexpected direct_host: {data.get('direct_host')!r}")
if data.get("port") != expected_port:
    raise SystemExit(f"unexpected direct port: {data.get('port')!r}")
if not data.get("server_instance_id"):
    raise SystemExit("missing server_instance_id")
if not data.get("build_id"):
    raise SystemExit("missing build_id")
capabilities = data.get("capabilities")
if not isinstance(capabilities, int) or capabilities <= 0:
    raise SystemExit(f"unexpected capabilities: {capabilities!r}")

print(data["server_instance_id"])
print(data["build_id"])
PY
)"
SERVER_INSTANCE_ID="$(printf '%s\n' "$upgrade_fields" | sed -n '1p')"
SERVER_BUILD_ID="$(printf '%s\n' "$upgrade_fields" | sed -n '2p')"

probe_json="$(
  ./target/debug/boo remote-upgrade-probe \
    --host 127.0.0.1 \
    --socket "$SOCKET_PATH"
)"
python3 - "$probe_json" "$SERVER_INSTANCE_ID" "$SERVER_BUILD_ID" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
expected_instance = sys.argv[2]
expected_build = sys.argv[3]

target = data.get("target")
probe = data.get("probe")
if not isinstance(target, dict):
    raise SystemExit("missing target summary")
if not isinstance(probe, dict):
    raise SystemExit("missing probe summary")
if target.get("selected_transport") != "quic-direct":
    raise SystemExit(f"unexpected target selected_transport: {target.get('selected_transport')!r}")
if probe.get("selected_transport") != "quic-direct":
    raise SystemExit(f"unexpected probe selected_transport: {probe.get('selected_transport')!r}")
probe_summary = probe.get("probe")
if not isinstance(probe_summary, dict):
    raise SystemExit("missing nested probe summary")
if probe_summary.get("host") != "127.0.0.1":
    raise SystemExit(f"unexpected probe host: {probe_summary.get('host')!r}")
if probe_summary.get("protocol_version") != 1:
    raise SystemExit(f"unexpected probe protocol_version: {probe_summary.get('protocol_version')!r}")
if probe_summary.get("server_instance_id") != expected_instance:
    raise SystemExit(
        f"unexpected probe server_instance_id: {probe_summary.get('server_instance_id')!r}"
    )
if probe_summary.get("build_id") != expected_build:
    raise SystemExit(f"unexpected probe build_id: {probe_summary.get('build_id')!r}")
if not isinstance(probe_summary.get("heartbeat_rtt_ms"), int):
    raise SystemExit(f"unexpected heartbeat_rtt_ms: {probe_summary.get('heartbeat_rtt_ms')!r}")
PY

echo "remote upgrade flow validation passed"
