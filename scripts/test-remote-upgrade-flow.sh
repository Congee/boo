#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_PATH="${BOO_REMOTE_UPGRADE_SOCKET:-/tmp/boo-remote-upgrade.sock}"
AUTH_KEY="${BOO_REMOTE_UPGRADE_AUTH_KEY:-boo-remote-upgrade}"
LOG_PATH="${BOO_REMOTE_UPGRADE_LOG:-/tmp/boo-remote-upgrade.log}"

PORT="$(
  python3 - <<'PY'
import socket

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
)"

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
target/debug/boo server \
  --socket "$SOCKET_PATH" \
  --remote-port "$PORT" \
  --remote-auth-key "$AUTH_KEY" \
  >"$LOG_PATH" 2>&1 &
SERVER_PID=$!

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
if data.get("selected_transport") != "tcp-direct":
    raise SystemExit(f"unexpected selected_transport: {data.get('selected_transport')!r}")
if data.get("direct_host") != "127.0.0.1":
    raise SystemExit(f"unexpected direct_host: {data.get('direct_host')!r}")
if data.get("port") != expected_port:
    raise SystemExit(f"unexpected direct port: {data.get('port')!r}")
if data.get("auth_required") is not True:
    raise SystemExit(f"expected auth_required true, got {data.get('auth_required')!r}")
if not data.get("server_identity_id"):
    raise SystemExit("missing server_identity_id")
if not data.get("server_instance_id"):
    raise SystemExit("missing server_instance_id")
if not data.get("build_id"):
    raise SystemExit("missing build_id")
capabilities = data.get("capabilities")
if not isinstance(capabilities, int) or capabilities <= 0:
    raise SystemExit(f"unexpected capabilities: {capabilities!r}")

print(data["server_identity_id"])
print(data["server_instance_id"])
print(data["build_id"])
PY
)"
SERVER_IDENTITY_ID="$(printf '%s\n' "$upgrade_fields" | sed -n '1p')"
SERVER_INSTANCE_ID="$(printf '%s\n' "$upgrade_fields" | sed -n '2p')"
SERVER_BUILD_ID="$(printf '%s\n' "$upgrade_fields" | sed -n '3p')"

probe_json="$(
  ./target/debug/boo probe-remote-daemon \
    --host 127.0.0.1 \
    --port "$PORT" \
    --auth-key "$AUTH_KEY" \
    --expect-server-identity "$SERVER_IDENTITY_ID"
)"
python3 - "$probe_json" "$SERVER_IDENTITY_ID" "$SERVER_INSTANCE_ID" "$SERVER_BUILD_ID" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
expected_identity = sys.argv[2]
expected_instance = sys.argv[3]
expected_build = sys.argv[4]

if data.get("host") != "127.0.0.1":
    raise SystemExit(f"unexpected probe host: {data.get('host')!r}")
if data.get("auth_required") is not True:
    raise SystemExit(f"expected auth_required true, got {data.get('auth_required')!r}")
if data.get("protocol_version") != 1:
    raise SystemExit(f"unexpected protocol_version: {data.get('protocol_version')!r}")
if data.get("server_identity_id") != expected_identity:
    raise SystemExit("probe server_identity_id mismatch")
if data.get("server_instance_id") != expected_instance:
    raise SystemExit("probe server_instance_id mismatch")
if data.get("build_id") != expected_build:
    raise SystemExit("probe build_id mismatch")
if not isinstance(data.get("heartbeat_rtt_ms"), int) or data["heartbeat_rtt_ms"] < 0:
    raise SystemExit(f"unexpected heartbeat_rtt_ms: {data.get('heartbeat_rtt_ms')!r}")
PY

create_json="$(
  ./target/debug/boo remote-daemon-create \
    --host 127.0.0.1 \
    --port "$PORT" \
    --auth-key "$AUTH_KEY" \
    --expect-server-identity "$SERVER_IDENTITY_ID" \
    --cols 100 \
    --rows 30
)"
SESSION_ID="$(
  python3 - "$create_json" "$SERVER_IDENTITY_ID" "$SERVER_INSTANCE_ID" "$SERVER_BUILD_ID" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
expected_identity = sys.argv[2]
expected_instance = sys.argv[3]
expected_build = sys.argv[4]

if data.get("server_identity_id") != expected_identity:
    raise SystemExit("create server_identity_id mismatch")
if data.get("server_instance_id") != expected_instance:
    raise SystemExit("create server_instance_id mismatch")
if data.get("build_id") != expected_build:
    raise SystemExit("create build_id mismatch")
session_id = data.get("session_id")
if not isinstance(session_id, int) or session_id <= 0:
    raise SystemExit(f"unexpected session_id: {session_id!r}")
print(session_id)
PY
)"

sessions_json="$(
  ./target/debug/boo remote-daemon-sessions \
    --host 127.0.0.1 \
    --port "$PORT" \
    --auth-key "$AUTH_KEY" \
    --expect-server-identity "$SERVER_IDENTITY_ID"
)"
python3 - "$sessions_json" "$SESSION_ID" "$SERVER_IDENTITY_ID" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
expected_session_id = int(sys.argv[2])
expected_identity = sys.argv[3]

if data.get("server_identity_id") != expected_identity:
    raise SystemExit("session list server_identity_id mismatch")
sessions = data.get("sessions")
if not isinstance(sessions, list) or not sessions:
    raise SystemExit("expected non-empty sessions list")
session_ids = {session.get("id") for session in sessions}
if expected_session_id not in session_ids:
    raise SystemExit(f"session {expected_session_id} missing from session list")
PY

attach_json="$(
  ./target/debug/boo remote-daemon-attach \
    --host 127.0.0.1 \
    --port "$PORT" \
    --auth-key "$AUTH_KEY" \
    --expect-server-identity "$SERVER_IDENTITY_ID" \
    --session-id "$SESSION_ID"
)"
python3 - "$attach_json" "$SESSION_ID" "$SERVER_IDENTITY_ID" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
expected_session_id = int(sys.argv[2])
expected_identity = sys.argv[3]

if data.get("server_identity_id") != expected_identity:
    raise SystemExit("attach server_identity_id mismatch")
attached = data.get("attached")
if not isinstance(attached, dict):
    raise SystemExit("missing attached summary")
if attached.get("session_id") != expected_session_id:
    raise SystemExit(f"unexpected attached session_id: {attached.get('session_id')!r}")
if not isinstance(data.get("rows"), int) or data["rows"] <= 0:
    raise SystemExit(f"unexpected rows: {data.get('rows')!r}")
if not isinstance(data.get("cols"), int) or data["cols"] <= 0:
    raise SystemExit(f"unexpected cols: {data.get('cols')!r}")
PY

echo "upgrade target:"
printf '%s\n' "$upgrade_json"
echo "direct probe:"
printf '%s\n' "$probe_json"
echo "direct session create:"
printf '%s\n' "$create_json"
echo "direct session list:"
printf '%s\n' "$sessions_json"
echo "direct attach:"
printf '%s\n' "$attach_json"
