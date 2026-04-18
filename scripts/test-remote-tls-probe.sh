#!/usr/bin/env bash
set -euo pipefail

# End-to-end test for the Rust --tls direct-daemon path.
# Starts two loopback boo daemons — one authless and one HMAC-auth — learns
# their daemon_identity values via plain probes, then verifies:
#   1. Authless: --tls probe with the matching identity succeeds.
#   2. Authless: --tls probe with a fabricated identity is rejected at handshake.
#   3. Authenticated: --tls + --auth-key + matching identity succeeds.
#   4. Authenticated: --tls + wrong --auth-key is rejected after handshake.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${BOO_REMOTE_TLS_PORT:-7361}"
SOCKET_PATH="${BOO_REMOTE_TLS_SOCKET:-/tmp/boo-remote-tls-probe.sock}"
LOG_PATH="${BOO_REMOTE_TLS_LOG:-/tmp/boo-remote-tls-probe.log}"
AUTH_PORT="${BOO_REMOTE_TLS_AUTH_PORT:-7362}"
AUTH_SOCKET_PATH="${BOO_REMOTE_TLS_AUTH_SOCKET:-/tmp/boo-remote-tls-probe-auth.sock}"
AUTH_LOG_PATH="${BOO_REMOTE_TLS_AUTH_LOG:-/tmp/boo-remote-tls-probe-auth.log}"
AUTH_KEY="${BOO_REMOTE_TLS_AUTH_KEY:-boo-remote-tls-test}"

cleanup() {
  for pid_var in SERVER_PID AUTH_SERVER_PID; do
    local pid="${!pid_var:-}"
    if [[ -n "$pid" ]]; then
      kill "$pid" >/dev/null 2>&1 || true
      wait "$pid" >/dev/null 2>&1 || true
    fi
  done
  rm -f "$SOCKET_PATH" "$AUTH_SOCKET_PATH"
}
trap cleanup EXIT

cd "$ROOT"

cargo build >/dev/null
rm -f "$SOCKET_PATH"
target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" >"$LOG_PATH" 2>&1 &
SERVER_PID=$!
rm -f "$AUTH_SOCKET_PATH"
target/debug/boo server \
  --socket "$AUTH_SOCKET_PATH" \
  --remote-port "$AUTH_PORT" \
  --remote-auth-key "$AUTH_KEY" \
  >"$AUTH_LOG_PATH" 2>&1 &
AUTH_SERVER_PID=$!

wait_for_port() {
  local port="$1"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if nc -z 127.0.0.1 "$port" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  return 1
}

wait_for_port "$PORT"
wait_for_port "$AUTH_PORT"

probe_plain_json="$(./target/debug/boo probe-remote-daemon --host 127.0.0.1 --port "$PORT")"
identity="$(python3 -c 'import json, sys; print(json.loads(sys.argv[1])["server_identity_id"])' "$probe_plain_json")"
if [[ -z "$identity" ]]; then
  echo "failed to extract server_identity_id from plain probe" >&2
  exit 1
fi

# 1. Matching-identity --tls probe must succeed and report the TLS capability bit.
probe_tls_json="$(./target/debug/boo probe-remote-daemon --host 127.0.0.1 --port "$PORT" \
  --expect-server-identity "$identity" --tls)"
python3 - "$probe_tls_json" "$identity" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
expected_identity = sys.argv[2]

if data.get("server_identity_id") != expected_identity:
    raise SystemExit(
        f"tls probe returned identity {data.get('server_identity_id')!r}, expected {expected_identity!r}"
    )
if data.get("protocol_version") != 1:
    raise SystemExit(f"unexpected tls probe protocol_version: {data.get('protocol_version')!r}")
caps = data.get("capabilities")
if not isinstance(caps, int):
    raise SystemExit(f"unexpected capabilities: {caps!r}")
# REMOTE_CAPABILITY_TCP_TLS_TRANSPORT = 1 << 9
if caps & (1 << 9) == 0:
    raise SystemExit(f"server did not advertise TLS transport capability: caps={caps}")
if not isinstance(data.get("heartbeat_rtt_ms"), int) or data["heartbeat_rtt_ms"] < 0:
    raise SystemExit(f"unexpected heartbeat_rtt_ms: {data.get('heartbeat_rtt_ms')!r}")
PY

# 2. Fabricated identity must be rejected.
bogus_identity="$(python3 -c '
import base64, hashlib
print(base64.urlsafe_b64encode(hashlib.sha256(b"not-a-real-cert").digest()).rstrip(b"=").decode())
')"
if [[ "$bogus_identity" == "$identity" ]]; then
  echo "test fixture collision: bogus identity matches real identity" >&2
  exit 1
fi
if ./target/debug/boo probe-remote-daemon --host 127.0.0.1 --port "$PORT" \
     --expect-server-identity "$bogus_identity" --tls >/dev/null 2>&1; then
  echo "tls probe with fabricated identity must fail, but it succeeded" >&2
  exit 1
fi

# 3. Authenticated --tls probe with correct auth-key and matching identity.
auth_probe_plain_json="$(./target/debug/boo probe-remote-daemon \
  --host 127.0.0.1 --port "$AUTH_PORT" --auth-key "$AUTH_KEY")"
auth_identity="$(python3 -c 'import json, sys; print(json.loads(sys.argv[1])["server_identity_id"])' \
  "$auth_probe_plain_json")"
if [[ -z "$auth_identity" ]]; then
  echo "failed to extract server_identity_id from authenticated plain probe" >&2
  exit 1
fi

auth_probe_tls_json="$(./target/debug/boo probe-remote-daemon \
  --host 127.0.0.1 --port "$AUTH_PORT" \
  --auth-key "$AUTH_KEY" \
  --expect-server-identity "$auth_identity" --tls)"
python3 - "$auth_probe_tls_json" "$auth_identity" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
if data.get("server_identity_id") != sys.argv[2]:
    raise SystemExit(
        f"auth tls probe identity mismatch: {data.get('server_identity_id')!r}"
    )
if data.get("auth_required") is not True:
    raise SystemExit(f"expected auth_required true, got {data.get('auth_required')!r}")
PY

# 4. Authenticated --tls probe with WRONG auth-key must fail (post-handshake auth).
if ./target/debug/boo probe-remote-daemon \
     --host 127.0.0.1 --port "$AUTH_PORT" \
     --auth-key "definitely-not-the-real-key" \
     --expect-server-identity "$auth_identity" --tls >/dev/null 2>&1; then
  echo "authenticated tls probe with wrong --auth-key must fail, but it succeeded" >&2
  exit 1
fi

echo "remote tls probe validation passed"
