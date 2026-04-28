#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOO_REPO_ROOT="$ROOT"
SOCKET_PATH="${BOO_REMOTE_DIAG_SOCKET:-/tmp/boo-remote-daemon-diagnostics.sock}"
PORT="${BOO_REMOTE_DIAG_PORT:-7359}"
LOG_PATH="${BOO_REMOTE_DIAG_LOG:-/tmp/boo-remote-daemon-diagnostics.log}"
VT_LIB_DIR="${BOO_VT_LIB_DIR:-${VT_LIB_DIR:-}}"

source "$ROOT/scripts/lib/vt-dylib-env.sh"

usage() {
  cat <<'EOF'
Usage: bash scripts/test-remote-daemon-diagnostics.sh [options]

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
boo_with_vt_lib_env target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" >"$LOG_PATH" 2>&1 &
SERVER_PID=$!

wait_for_probe() {
  local port="$1"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if ./target/debug/boo probe-remote-daemon --host 127.0.0.1 --port "$port" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  echo "timed out waiting for Boo remote daemon on 127.0.0.1:$port" >&2
  cat "$LOG_PATH" >&2 || true
  return 1
}

wait_for_probe "$PORT"

python3 - "$SOCKET_PATH" <<'PYCHECK'
import json
import subprocess
import sys

socket_path = sys.argv[1]
output = subprocess.check_output(
    ["./target/debug/boo", "remote-clients", "--socket", socket_path],
    text=True,
)
snapshot = json.loads(output)
servers = snapshot.get("servers")
if not isinstance(servers, list) or not servers:
    raise SystemExit("expected non-empty servers list in remote-clients output")
server_info = servers[0]
if server_info.get("protocol_version") != 1:
    raise SystemExit(f"unexpected server protocol version: {server_info.get('protocol_version')!r}")
if not isinstance(server_info.get("capabilities"), int) or server_info["capabilities"] <= 0:
    raise SystemExit(f"unexpected server capabilities: {server_info.get('capabilities')!r}")
if not server_info.get("build_id"):
    raise SystemExit("missing build_id in server diagnostics")
if not server_info.get("server_instance_id"):
    raise SystemExit("missing server_instance_id in server diagnostics")
if server_info.get("auth_challenge_window_ms") != 30_000:
    raise SystemExit(f"unexpected auth_challenge_window_ms: {server_info.get('auth_challenge_window_ms')!r}")
if server_info.get("heartbeat_window_ms") != 20_000:
    raise SystemExit(f"unexpected heartbeat_window_ms: {server_info.get('heartbeat_window_ms')!r}")
clients = snapshot.get("clients", [])
if server_info.get("connected_clients") != len(clients):
    raise SystemExit(
        f"unexpected connected_clients count: {server_info.get('connected_clients')!r} vs {len(clients)}"
    )
if server_info.get("pending_auth_clients") != 0:
    raise SystemExit(f"unexpected pending_auth_clients: {server_info.get('pending_auth_clients')!r}")
if server_info.get("viewing_clients") != 0:
    raise SystemExit(f"unexpected viewing_clients: {server_info.get('viewing_clients')!r}")
if not isinstance(clients, list):
    raise SystemExit("expected clients list")
PYCHECK

probe_json="$(./target/debug/boo probe-remote-daemon --host 127.0.0.1 --port "$PORT")"
python3 - "$probe_json" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
if data.get("host") != "127.0.0.1":
    raise SystemExit(f"unexpected probe host: {data.get('host')!r}")
if data.get("protocol_version") != 1:
    raise SystemExit(f"unexpected protocol version: {data.get('protocol_version')!r}")
if not isinstance(data.get("capabilities"), int) or data["capabilities"] <= 0:
    raise SystemExit(f"unexpected capabilities: {data.get('capabilities')!r}")
if not data.get("build_id"):
    raise SystemExit("missing build_id in probe summary")
if not data.get("server_instance_id"):
    raise SystemExit("missing server_instance_id in probe summary")
if not isinstance(data.get("heartbeat_rtt_ms"), int) or data["heartbeat_rtt_ms"] < 0:
    raise SystemExit(f"unexpected heartbeat_rtt_ms: {data.get('heartbeat_rtt_ms')!r}")
PY

echo "remote daemon diagnostics validation passed"
