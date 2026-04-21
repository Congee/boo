#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_PATH="${BOO_REMOTE_DIAG_SOCKET:-/tmp/boo-remote-daemon-diagnostics.sock}"
PORT="${BOO_REMOTE_DIAG_PORT:-7359}"
LOG_PATH="${BOO_REMOTE_DIAG_LOG:-/tmp/boo-remote-daemon-diagnostics.log}"

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
target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" >"$LOG_PATH" 2>&1 &
SERVER_PID=$!

wait_for_port() {
  local port="$1"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if nc -z 127.0.0.1 "$port" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  echo "timed out waiting for 127.0.0.1:$port" >&2
  return 1
}

wait_for_port "$PORT"

python3 - "$PORT" "$SOCKET_PATH" <<'PY'
import json
import socket
import struct
import subprocess
import sys
import time

MAGIC = b"GS"
HEADER_LEN = 7
AUTH = 0x01
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
socket_path = sys.argv[2]

active = socket.create_connection(("127.0.0.1", port))
send_message(active, AUTH)
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
if server_info.get("auth_challenge_window_ms") != 30_000:
    raise SystemExit(f"unexpected auth_challenge_window_ms: {server_info.get('auth_challenge_window_ms')!r}")
if server_info.get("heartbeat_window_ms") != 20_000:
    raise SystemExit(f"unexpected heartbeat_window_ms: {server_info.get('heartbeat_window_ms')!r}")
if server_info.get("revive_window_ms") != 30_000:
    raise SystemExit(f"unexpected revive_window_ms: {server_info.get('revive_window_ms')!r}")
if server_info.get("connected_clients") != len(clients := snapshot.get("clients", [])):
    raise SystemExit(
        f"unexpected connected_clients count: {server_info.get('connected_clients')!r} vs {len(clients)}"
    )
if server_info.get("pending_auth_clients") != 0:
    raise SystemExit(f"unexpected pending_auth_clients: {server_info.get('pending_auth_clients')!r}")
if server_info.get("attached_clients") != 0:
    raise SystemExit(f"unexpected attached_clients: {server_info.get('attached_clients')!r}")
if server_info.get("revivable_attachments") != 0:
    raise SystemExit(
        f"unexpected revivable_attachments count: {server_info.get('revivable_attachments')!r}"
    )

if not isinstance(clients, list) or len(clients) != 1:
    raise SystemExit(f"expected exactly 1 direct client, got {len(clients) if isinstance(clients, list) else clients!r}")
client = clients[0]
if not client.get("authenticated"):
    raise SystemExit("expected authenticated client in diagnostics snapshot")
if client.get("challenge_pending"):
    raise SystemExit("authenticated client unexpectedly marked challenge pending")
if client.get("transport_kind") != "tcp":
    raise SystemExit(f"unexpected client transport_kind: {client.get('transport_kind')!r}")
if client.get("server_socket_path") is not None:
    raise SystemExit(
        f"expected no local server_socket_path for tcp client, got {client.get('server_socket_path')!r}"
    )
heartbeat_age = client.get("last_heartbeat_age_ms")
if not isinstance(heartbeat_age, int) or heartbeat_age < 0 or heartbeat_age > 5_000:
    raise SystemExit(f"unexpected last_heartbeat_age_ms: {heartbeat_age!r}")
heartbeat_expires_in = client.get("heartbeat_expires_in_ms")
if not isinstance(heartbeat_expires_in, int) or heartbeat_expires_in <= 0:
    raise SystemExit(
        f"expected positive heartbeat_expires_in_ms, got {heartbeat_expires_in!r}"
    )
if client.get("heartbeat_overdue") is not False:
    raise SystemExit(
        f"expected heartbeat_overdue false for active client, got {client.get('heartbeat_overdue')!r}"
    )
if not isinstance(client.get("connection_age_ms"), int):
    raise SystemExit("missing connection_age_ms for authenticated client")

active.close()
PY

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
if not data.get("server_identity_id"):
    raise SystemExit("missing server_identity_id in probe summary")
if not isinstance(data.get("heartbeat_rtt_ms"), int) or data["heartbeat_rtt_ms"] < 0:
    raise SystemExit(f"unexpected heartbeat_rtt_ms: {data.get('heartbeat_rtt_ms')!r}")
PY

echo "remote daemon diagnostics validation passed"
