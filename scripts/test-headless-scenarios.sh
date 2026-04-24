#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOO_REPO_ROOT="$ROOT_DIR"
source "$ROOT_DIR/scripts/lib/vt-dylib-env.sh"
CONFIG_ROOT="${BOO_TEST_CONFIG_ROOT:-/tmp/boo-headless-scenarios}"
SOCKET_PATH="${BOO_TEST_SOCKET:-/tmp/boo-headless-scenarios.sock}"
LOG_PATH="${BOO_TEST_LOG:-/tmp/boo-headless-scenarios.log}"
VT_LIB_DIR="${VT_LIB_DIR:-}"
TERMINAL_BODY_IMPL="${BOO_TERMINAL_BODY_IMPL:-}"

usage() {
  cat <<'EOF'
Usage: bash scripts/test-headless-scenarios.sh [options]

Options:
  --config-root PATH
  --socket PATH
  --log PATH
  --vt-lib-dir PATH
  --terminal-body-impl NAME
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
    --config-root)
      require_arg "$@"; CONFIG_ROOT="$2"; shift 2 ;;
    --socket)
      require_arg "$@"; SOCKET_PATH="$2"; shift 2 ;;
    --log)
      require_arg "$@"; LOG_PATH="$2"; shift 2 ;;
    --vt-lib-dir)
      require_arg "$@"; VT_LIB_DIR="$2"; shift 2 ;;
    --terminal-body-impl)
      require_arg "$@"; TERMINAL_BODY_IMPL="$2"; shift 2 ;;
    -h|--help)
      usage; exit 0 ;;
    --)
      shift; break ;;
    *)
      echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

CONFIG_DIR="$CONFIG_ROOT/boo"
if [[ -n "$VT_LIB_DIR" && -z "${BOO_VT_LIB_DIR:-}" ]]; then
  BOO_VT_LIB_DIR="$VT_LIB_DIR"
fi

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
  export XDG_CONFIG_HOME="$CONFIG_ROOT"
  if [[ -n "$TERMINAL_BODY_IMPL" ]]; then
    export BOO_TERMINAL_BODY_IMPL="$TERMINAL_BODY_IMPL"
  fi
  boo_with_vt_lib_env target/debug/boo --headless >"$LOG_PATH" 2>&1
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
