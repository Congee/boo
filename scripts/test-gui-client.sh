#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

SOCKET="${BOO_TEST_SOCKET:-/tmp/boo-gui-test.sock}"
GUI_TEST_SOCKET="${BOO_GUI_TEST_SOCKET:-/tmp/boo-gui-input.sock}"

cleanup() {
  target/debug/boo quit-server >/dev/null 2>&1 || true
  pkill -f 'target/debug/boo|cargo run' >/dev/null 2>&1 || true
  rm -f "$SOCKET" "$SOCKET.stream" "$GUI_TEST_SOCKET"
}

cleanup
trap cleanup EXIT

cargo build >/dev/null

BOO_GUI_TEST_SOCKET="$GUI_TEST_SOCKET" target/debug/boo --socket "$SOCKET" >/dev/null 2>&1 &
GUI_PID=$!

for _ in $(seq 1 100); do
  if [ -S "$SOCKET" ] && [ -S "$GUI_TEST_SOCKET" ]; then
    break
  fi
  sleep 0.1
done

if [ ! -S "$SOCKET" ] || [ ! -S "$GUI_TEST_SOCKET" ]; then
  echo "boo GUI test sockets not ready" >&2
  exit 1
fi

python3 - <<'PY' "$GUI_TEST_SOCKET"
import socket, sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(sys.argv[1])
sock.sendall(b"text abc\n")
sock.close()
PY

for _ in $(seq 1 30); do
  SNAPSHOT="$(python3 scripts/ui-test-client.py --socket "$SOCKET" snapshot)"
  if python3 - <<'PY' "$SNAPSHOT"
import json, sys
data = json.loads(sys.argv[1])["snapshot"]
line = "".join(cell["text"] for cell in data["terminal"]["rows_data"][0]["cells"])
raise SystemExit(0 if "abc" in line else 1)
PY
  then
    break
  fi
  sleep 0.1
done

python3 - <<'PY' "$SNAPSHOT"
import json, sys
data = json.loads(sys.argv[1])["snapshot"]
line = "".join(cell["text"] for cell in data["terminal"]["rows_data"][0]["cells"])
assert "abc" in line, line
PY
