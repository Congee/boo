#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOO_REPO_ROOT="$ROOT"
SOCKET_PATH="${BOO_LATENCY_TRACE_SOCKET:-/tmp/boo-latency-trace.sock}"
LOG_PATH="${BOO_LATENCY_TRACE_LOG:-/tmp/boo-latency-trace.log}"
VT_LIB_DIR="${BOO_VT_LIB_DIR:-${VT_LIB_DIR:-}}"

source "$ROOT/scripts/lib/vt-dylib-env.sh"

usage() {
  cat <<'EOF'
Usage: bash scripts/test-latency-traces.sh [options]

Options:
  --socket PATH
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
  local pid="${BOO_PID:-}"
  if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
  rm -f "$SOCKET_PATH" "${SOCKET_PATH}.stream"
}
trap cleanup EXIT

cd "$ROOT"

cargo build >/dev/null
rm -f "$SOCKET_PATH" "${SOCKET_PATH}.stream" "$LOG_PATH"

boo_with_vt_lib_env target/debug/boo \
  --trace-filter boo::latency=info \
  --socket "$SOCKET_PATH" \
  server >"$LOG_PATH" 2>&1 &
BOO_PID=$!

python3 - "$SOCKET_PATH" <<'PY'
import json
import socket
import struct
import sys
import time

socket_path = sys.argv[1] + ".stream"

MAGIC = b"GS"
MSG_RUNTIME_ACTION = 0x12
MSG_CREATE = 0x05
MSG_FULL_STATE = 0x83
MSG_DELTA = 0x84
MSG_UI_RUNTIME_STATE = 0x8D
MSG_UI_PANE_FULL_STATE = 0x90
MSG_UI_PANE_DELTA = 0x91


def encode_message(message_type: int, payload: bytes) -> bytes:
    return MAGIC + bytes([message_type]) + struct.pack("<I", len(payload)) + payload


def read_exact(sock: socket.socket, size: int) -> bytes:
    chunks = []
    remaining = size
    while remaining:
        chunk = sock.recv(remaining)
        if not chunk:
            raise RuntimeError("unexpected EOF from Boo stream")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def read_message(sock: socket.socket) -> tuple[int, bytes]:
    header = read_exact(sock, 7)
    if header[:2] != MAGIC:
        raise RuntimeError(f"invalid stream magic: {header[:2]!r}")
    payload_len = struct.unpack("<I", header[3:7])[0]
    payload = read_exact(sock, payload_len) if payload_len else b""
    return header[2], payload


def connect_socket(path: str, timeout: float = 10.0) -> socket.socket:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            probe.connect(path)
            return probe
        except OSError:
            probe.close()
            time.sleep(0.05)
    raise RuntimeError(f"timed out waiting for {path}")


def send_runtime_action(sock: socket.socket, action: dict) -> None:
    payload = json.dumps(action, separators=(",", ":")).encode("utf-8")
    sock.sendall(encode_message(MSG_RUNTIME_ACTION, payload))


def send_create(sock: socket.socket, cols: int = 120, rows: int = 36) -> None:
    payload = struct.pack("<HH", cols, rows)
    sock.sendall(encode_message(MSG_CREATE, payload))


def decode_pane_update_header(payload: bytes) -> tuple[int, int, int, int]:
    if len(payload) < 28:
        raise RuntimeError("truncated pane update payload")
    return struct.unpack("<IQQQ", payload[:28])


with connect_socket(socket_path) as sock:
    sock.settimeout(8)

    send_create(sock)
    send_runtime_action(sock, {"kind": "attach_view", "view_id": 1})

    initial_state = None
    ignored = []
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        message_type, payload = read_message(sock)
        ignored.append(f"0x{message_type:02x}")
        if message_type == MSG_UI_RUNTIME_STATE:
            candidate = json.loads(payload)
            if candidate.get("viewed_tab_id") is not None or candidate.get("tabs"):
                initial_state = candidate
                break
    if initial_state is None:
        raise RuntimeError(f"did not receive initial UI runtime state; saw {ignored}")

    view_id = initial_state.get("view_id") or 1
    tab_id = initial_state.get("viewed_tab_id")
    if tab_id is None:
        tabs = initial_state.get("tabs") or []
        tab_id = tabs[0]["tab_id"]

    send_runtime_action(
        sock,
        {"kind": "new_split", "view_id": view_id, "direction": "right"},
    )

    split_state = None
    ignored = []
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        message_type, payload = read_message(sock)
        ignored.append(f"0x{message_type:02x}")
        if message_type == MSG_UI_RUNTIME_STATE:
            candidate = json.loads(payload)
            if len(candidate.get("visible_pane_ids") or []) >= 2:
                split_state = candidate
                break
    if split_state is None:
        raise RuntimeError(f"did not receive split UI runtime state; saw {ignored}")

    tab_id = split_state.get("viewed_tab_id") or tab_id
    visible_panes = split_state["visible_pane_ids"]
    focused_pane = split_state["focused_pane"]
    target_pane = next((pane for pane in visible_panes if pane != focused_pane), visible_panes[-1])

    send_runtime_action(
        sock,
        {
            "kind": "focus_pane",
            "view_id": view_id,
            "tab_id": tab_id,
            "pane_id": target_pane,
        },
    )

    saw_focused_screen = False
    pane_update = None
    ignored = []
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        message_type, payload = read_message(sock)
        ignored.append(f"0x{message_type:02x}")
        if message_type in (MSG_FULL_STATE, MSG_DELTA):
            saw_focused_screen = True
        elif message_type in (MSG_UI_PANE_FULL_STATE, MSG_UI_PANE_DELTA):
            pane_update = (message_type, decode_pane_update_header(payload))
            break

    if not saw_focused_screen:
        raise RuntimeError(f"focus did not publish focused screen update; saw {ignored}")
    if pane_update is None:
        raise RuntimeError(f"focus did not publish non-focused pane update; saw {ignored}")

    message_type, (update_tab, update_pane, pane_revision, runtime_revision) = pane_update
    if update_tab != tab_id:
        raise RuntimeError(f"pane update tab mismatch: got {update_tab}, want {tab_id}")
    if update_pane == target_pane:
        raise RuntimeError("pane update unexpectedly targeted the newly focused pane")
    if pane_revision == 0:
        raise RuntimeError("pane update did not carry a pane revision")
    if runtime_revision == 0:
        raise RuntimeError("pane update did not carry a runtime revision")

    print(
        json.dumps(
            {
                "ok": True,
                "focused_pane": target_pane,
                "pane_update_type": f"0x{message_type:02x}",
                "pane_update": {
                    "tab_id": update_tab,
                    "pane_id": update_pane,
                    "pane_revision": pane_revision,
                    "runtime_revision": runtime_revision,
                },
            },
            sort_keys=True,
        )
    )
PY

kill "$BOO_PID" >/dev/null 2>&1 || true
wait "$BOO_PID" >/dev/null 2>&1 || true
BOO_PID=""

for event in \
  "remote.connect" \
  "remote.runtime_action" \
  "remote.focus_pane" \
  "remote.pane_update"
do
  if ! grep -q "$event" "$LOG_PATH"; then
    echo "missing trace event $event in $LOG_PATH" >&2
    cat "$LOG_PATH" >&2
    exit 1
  fi
done

echo "latency trace stream verification passed"
