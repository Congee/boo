#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_ROOT="${BOO_TEST_CONFIG_ROOT:-/tmp/boo-headless-scenarios}"
CONFIG_DIR="$CONFIG_ROOT/boo"
SOCKET_PATH="${BOO_TEST_SOCKET:-/tmp/boo-headless-scenarios.sock}"
LOG_PATH="${BOO_TEST_LOG:-/tmp/boo-headless-scenarios.log}"

mkdir -p "$CONFIG_DIR"
cat > "$CONFIG_DIR/config.boo" <<EOF
control-socket = $SOCKET_PATH
EOF

rm -f "$SOCKET_PATH" "$LOG_PATH"

cleanup() {
  if [[ -n "${BOO_PID:-}" ]] && kill -0 "$BOO_PID" 2>/dev/null; then
    kill "$BOO_PID" 2>/dev/null || true
    wait "$BOO_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

(
  cd "$ROOT_DIR"
  cargo build >/dev/null
)

(
  cd "$ROOT_DIR"
  XDG_CONFIG_HOME="$CONFIG_ROOT" target/debug/boo --headless >"$LOG_PATH" 2>&1
) &
BOO_PID=$!

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" wait-ready >/dev/null

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request new-tab >/dev/null
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request prev-tab >/dev/null
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request new-split direction=right >/dev/null
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-text 'text=printf HEADLESS_SPLIT_OK\r' >/dev/null
sleep 0.2
SNAPSHOT="$(python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" snapshot)"

printf '%s\n' "$SNAPSHOT" | python3 -c '
import json, sys
payload = json.load(sys.stdin)["snapshot"]
assert len(payload["tabs"]) == 2, payload["tabs"]
assert payload["tabs"][0]["pane_count"] == 2, payload["tabs"]
assert len(payload["visible_panes"]) == 2, payload["visible_panes"]
text = "\n".join(
    "".join(cell["text"] for cell in row["cells"])
    for row in payload["terminal"]["rows_data"]
)
if "HEADLESS_SPLIT_OK" not in text:
    raise SystemExit("HEADLESS_SPLIT_OK not present in headless scenario snapshot")
'

echo "headless scenarios passed"
