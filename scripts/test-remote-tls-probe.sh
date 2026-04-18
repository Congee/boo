#!/usr/bin/env bash
set -euo pipefail

# End-to-end test for the Rust --tls direct-daemon path.
# Starts an authless loopback boo daemon, learns its daemon_identity via a plain
# probe, then verifies that:
#   1. A --tls probe with the matching identity succeeds.
#   2. A --tls probe with a fabricated identity is rejected at the TLS handshake.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${BOO_REMOTE_TLS_PORT:-7361}"
SOCKET_PATH="${BOO_REMOTE_TLS_SOCKET:-/tmp/boo-remote-tls-probe.sock}"
LOG_PATH="${BOO_REMOTE_TLS_LOG:-/tmp/boo-remote-tls-probe.log}"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH"
}
trap cleanup EXIT

cd "$ROOT"

cargo build >/dev/null
rm -f "$SOCKET_PATH"
target/debug/boo server --socket "$SOCKET_PATH" --remote-port "$PORT" >"$LOG_PATH" 2>&1 &
SERVER_PID=$!
# Give the process time to bind the TCP listener; on cold caches this can take
# more than a second on macOS.
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if nc -z 127.0.0.1 "$PORT" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

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

echo "remote tls probe validation passed"
