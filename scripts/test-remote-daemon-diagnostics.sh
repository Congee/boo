#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_PATH="${BOO_REMOTE_DIAG_SOCKET:-/tmp/boo-remote-daemon-diagnostics.sock}"
PORT="${BOO_REMOTE_DIAG_PORT:-7359}"
AUTH_KEY="${BOO_REMOTE_DIAG_AUTH_KEY:-boo-remote-diagnostics}"
AUTHLESS_PORT="${BOO_REMOTE_DIAG_AUTHLESS_PORT:-7360}"
LOG_PATH="${BOO_REMOTE_DIAG_LOG:-/tmp/boo-remote-daemon-diagnostics.log}"
AUTHLESS_LOG_PATH="${BOO_REMOTE_DIAG_AUTHLESS_LOG:-/tmp/boo-remote-daemon-authless-diagnostics.log}"

cleanup() {
  for pid_var in SERVER_PID AUTHLESS_SERVER_PID; do
    local pid="${!pid_var:-}"
    if [[ -n "$pid" ]]; then
      kill "$pid" >/dev/null 2>&1 || true
      wait "$pid" >/dev/null 2>&1 || true
    fi
  done
  for socket_var in SOCKET_PATH AUTHLESS_SOCKET_PATH; do
    local socket_path="${!socket_var:-}"
    if [[ -n "$socket_path" ]]; then
      rm -f "$socket_path"
    fi
  done
}
trap cleanup EXIT

AUTHLESS_SOCKET_PATH="${BOO_REMOTE_DIAG_AUTHLESS_SOCKET:-/tmp/boo-remote-daemon-authless-diagnostics.sock}"

cd "$ROOT"

cargo build >/dev/null
rm -f "$SOCKET_PATH"
target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" --remote-auth-key "$AUTH_KEY" >"$LOG_PATH" 2>&1 &
SERVER_PID=$!
rm -f "$AUTHLESS_SOCKET_PATH"
target/debug/boo server --socket "$AUTHLESS_SOCKET_PATH" --remote-port "$AUTHLESS_PORT" >"$AUTHLESS_LOG_PATH" 2>&1 &
AUTHLESS_SERVER_PID=$!
sleep 1

python3 - "$PORT" "$AUTH_KEY" "$SOCKET_PATH" <<'PY'
import hashlib
import hmac
import json
import socket
import struct
import subprocess
import sys
import time

MAGIC = b"GS"
HEADER_LEN = 7
AUTH = 0x01
AUTH_CHALLENGE = 0x09
AUTH_OK = 0x80
HEARTBEAT = 0x11
HEARTBEAT_ACK = 0x92


def send_message(sock: socket.socket, ty: int, payload: bytes = b"") -> None:
    sock.sendall(MAGIC + bytes([ty]) + struct.pack("<I", len(payload)) + payload)


def read_exact(sock: socket.socket, size: int) -> bytes:
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise SystemExit(f"unexpected EOF while reading {size} bytes")
        data += chunk
    return data


def read_message(sock: socket.socket) -> tuple[int, bytes]:
    header = read_exact(sock, HEADER_LEN)
    if header[:2] != MAGIC:
        raise SystemExit(f"invalid remote magic: {header[:2]!r}")
    ty = header[2]
    payload_len = struct.unpack("<I", header[3:])[0]
    payload = read_exact(sock, payload_len)
    return ty, payload


port = int(sys.argv[1])
auth_key = sys.argv[2].encode("utf-8")
socket_path = sys.argv[3]

pending = socket.create_connection(("127.0.0.1", port))
send_message(pending, AUTH)
ty, pending_challenge = read_message(pending)
if ty != AUTH_CHALLENGE:
    raise SystemExit(f"expected auth challenge for pending client, got type {ty:#x}")

active = socket.create_connection(("127.0.0.1", port))
send_message(active, AUTH)
ty, challenge = read_message(active)
if ty != AUTH_CHALLENGE:
    raise SystemExit(f"expected auth challenge for active client, got type {ty:#x}")
digest = hmac.new(auth_key, challenge, hashlib.sha256).digest()
send_message(active, AUTH, digest)
ty, payload = read_message(active)
if ty != AUTH_OK:
    raise SystemExit(f"expected auth ok for active client, got type {ty:#x}")

send_message(active, HEARTBEAT, b"diag")
ty, payload = read_message(active)
if ty != HEARTBEAT_ACK or payload != b"diag":
    raise SystemExit("heartbeat acknowledgement mismatch")

time.sleep(0.15)
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
if not server_info.get("server_identity_id"):
    raise SystemExit("missing server_identity_id in server diagnostics")
if server_info.get("auth_required") is not True:
    raise SystemExit(f"expected auth_required true in server diagnostics, got {server_info.get('auth_required')!r}")
if server_info.get("auth_challenge_window_ms") != 10_000:
    raise SystemExit(f"unexpected auth_challenge_window_ms: {server_info.get('auth_challenge_window_ms')!r}")
if server_info.get("heartbeat_window_ms") != 20_000:
    raise SystemExit(f"unexpected heartbeat_window_ms: {server_info.get('heartbeat_window_ms')!r}")
if server_info.get("revive_window_ms") != 30_000:
    raise SystemExit(f"unexpected revive_window_ms: {server_info.get('revive_window_ms')!r}")
if server_info.get("connected_clients") != len(clients := snapshot.get("clients", [])):
    raise SystemExit(
        f"unexpected connected_clients count: {server_info.get('connected_clients')!r} vs {len(clients)}"
    )
if server_info.get("pending_auth_clients") != 1:
    raise SystemExit(f"unexpected pending_auth_clients: {server_info.get('pending_auth_clients')!r}")
if server_info.get("attached_clients") != 0:
    raise SystemExit(f"unexpected attached_clients: {server_info.get('attached_clients')!r}")
if server_info.get("revivable_attachments") != 0:
    raise SystemExit(
        f"unexpected revivable_attachments count: {server_info.get('revivable_attachments')!r}"
    )
if not isinstance(clients, list):
    raise SystemExit("expected clients list in remote-clients output")
if len(clients) < 2:
    raise SystemExit(f"expected at least 2 direct clients, got {len(clients)}")

pending_client = next((client for client in clients if client.get("challenge_pending")), None)
if pending_client is None:
    raise SystemExit("expected a pending-challenge client in diagnostics snapshot")
if pending_client.get("authenticated"):
    raise SystemExit("pending-challenge client unexpectedly marked authenticated")
if pending_client.get("transport_kind") != "tcp":
    raise SystemExit(f"unexpected pending client transport_kind: {pending_client.get('transport_kind')!r}")
if pending_client.get("server_socket_path") is not None:
    raise SystemExit(
        f"expected no local server_socket_path for tcp client, got {pending_client.get('server_socket_path')!r}"
    )
expires_in = pending_client.get("challenge_expires_in_ms")
if not isinstance(expires_in, int) or expires_in <= 0:
    raise SystemExit(f"expected positive challenge_expires_in_ms, got {expires_in!r}")

heartbeat_client = next(
    (
        client
        for client in clients
        if client.get("authenticated") and client.get("last_heartbeat_age_ms") is not None
    ),
    None,
)
if heartbeat_client is None:
    raise SystemExit("expected an authenticated heartbeat client in diagnostics snapshot")
if heartbeat_client.get("transport_kind") != "tcp":
    raise SystemExit(f"unexpected heartbeat client transport_kind: {heartbeat_client.get('transport_kind')!r}")
if heartbeat_client.get("server_socket_path") is not None:
    raise SystemExit(
        f"expected no local server_socket_path for tcp heartbeat client, got {heartbeat_client.get('server_socket_path')!r}"
    )
heartbeat_age = heartbeat_client.get("last_heartbeat_age_ms")
if not isinstance(heartbeat_age, int) or heartbeat_age < 0 or heartbeat_age > 5_000:
    raise SystemExit(f"unexpected last_heartbeat_age_ms: {heartbeat_age!r}")
heartbeat_expires_in = heartbeat_client.get("heartbeat_expires_in_ms")
if not isinstance(heartbeat_expires_in, int) or heartbeat_expires_in <= 0:
    raise SystemExit(
        f"expected positive heartbeat_expires_in_ms, got {heartbeat_expires_in!r}"
    )
if heartbeat_client.get("heartbeat_overdue") is not False:
    raise SystemExit(
        f"expected heartbeat_overdue false for active client, got {heartbeat_client.get('heartbeat_overdue')!r}"
    )
if not isinstance(heartbeat_client.get("connection_age_ms"), int):
    raise SystemExit("missing connection_age_ms for authenticated client")

pending.close()
active.close()
PY

probe_json="$(./target/debug/boo probe-remote-daemon --host 127.0.0.1 --port "$PORT" --auth-key "$AUTH_KEY")"
python3 - "$probe_json" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
if data.get("host") != "127.0.0.1":
    raise SystemExit(f"unexpected probe host: {data.get('host')!r}")
if data.get("auth_required") is not True:
    raise SystemExit(f"expected auth_required true, got {data.get('auth_required')!r}")
if data.get("protocol_version") != 1:
    raise SystemExit(f"unexpected protocol version: {data.get('protocol_version')!r}")
if not isinstance(data.get("capabilities"), int) or data["capabilities"] <= 0:
    raise SystemExit(f"unexpected capabilities: {data.get('capabilities')!r}")
if not data.get("build_id"):
    raise SystemExit("missing build_id in probe summary")
if not data.get("server_instance_id"):
    raise SystemExit("missing server_instance_id in probe summary")
if not data.get("server_identity_id"):
    raise SystemExit("missing server_identity_id in probe summary")
if not isinstance(data.get("heartbeat_rtt_ms"), int) or data["heartbeat_rtt_ms"] < 0:
    raise SystemExit(f"unexpected heartbeat_rtt_ms: {data.get('heartbeat_rtt_ms')!r}")
PY

authless_probe_json="$(./target/debug/boo probe-remote-daemon --host 127.0.0.1 --port "$AUTHLESS_PORT")"
python3 - "$authless_probe_json" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
if data.get("auth_required") is not False:
    raise SystemExit(f"expected auth_required false for authless probe, got {data.get('auth_required')!r}")
if data.get("protocol_version") != 1:
    raise SystemExit(f"unexpected authless protocol version: {data.get('protocol_version')!r}")
if not data.get("build_id"):
    raise SystemExit("missing build_id in authless probe summary")
if not data.get("server_instance_id"):
    raise SystemExit("missing server_instance_id in authless probe summary")
if not data.get("server_identity_id"):
    raise SystemExit("missing server_identity_id in authless probe summary")
PY

authless_snapshot_json="$(./target/debug/boo remote-clients --socket "$AUTHLESS_SOCKET_PATH")"
python3 - "$authless_snapshot_json" <<'PY'
import json
import sys

snapshot = json.loads(sys.argv[1])
servers = snapshot.get("servers")
if not isinstance(servers, list) or not servers:
    raise SystemExit("expected non-empty servers list for authless diagnostics")
server_info = servers[0]
if server_info.get("auth_required") is not False:
    raise SystemExit(
        f"expected auth_required false for authless server diagnostics, got {server_info.get('auth_required')!r}"
    )
if server_info.get("local_socket_path") is not None:
    raise SystemExit(
        f"expected no local_socket_path for authless tcp daemon, got {server_info.get('local_socket_path')!r}"
    )
if server_info.get("heartbeat_window_ms") != 20_000:
    raise SystemExit(
        f"unexpected authless heartbeat_window_ms: {server_info.get('heartbeat_window_ms')!r}"
    )
if server_info.get("connected_clients") != 0:
    raise SystemExit(
        f"expected authless connected_clients 0, got {server_info.get('connected_clients')!r}"
    )
if server_info.get("attached_clients") != 0:
    raise SystemExit(
        f"expected authless attached_clients 0, got {server_info.get('attached_clients')!r}"
    )
PY

echo "remote daemon diagnostics validation passed"
