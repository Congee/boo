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

wait_for_ready() {
  for _ in $(seq 1 100); do
    if [ -S "$SOCKET" ] && [ -S "$GUI_TEST_SOCKET" ]; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

send_gui_text() {
python3 - <<'PY' "$GUI_TEST_SOCKET" "$1"
import socket, sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(sys.argv[1])
sock.sendall(f"text {sys.argv[2]}\n".encode())
sock.close()
PY
}

send_gui_key() {
python3 - <<'PY' "$GUI_TEST_SOCKET" "$1"
import socket, sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(sys.argv[1])
sock.sendall(f"key {sys.argv[2]}\n".encode())
sock.close()
PY
}

send_gui_command() {
python3 - <<'PY' "$GUI_TEST_SOCKET" "$1"
import socket, sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(sys.argv[1])
sock.sendall(f"command {sys.argv[2]}\n".encode())
sock.close()
PY
}

send_gui_click() {
python3 - <<'PY' "$GUI_TEST_SOCKET" "$1" "$2"
import socket, sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(sys.argv[1])
sock.sendall(f"click {sys.argv[2]} {sys.argv[3]}\n".encode())
sock.close()
PY
}

send_gui_appkey() {
python3 - <<'PY' "$GUI_TEST_SOCKET" "$1"
import socket, sys
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect(sys.argv[1])
sock.sendall(f"appkey {sys.argv[2]}\n".encode())
sock.close()
PY
}

snapshot_json() {
  python3 scripts/ui-test-client.py --socket "$SOCKET" snapshot
}

snapshot_to_file() {
  local file
  file="$(mktemp)"
  snapshot_json >"$file"
  printf '%s\n' "$file"
}

assert_snapshot_contains() {
for _ in $(seq 1 30); do
  SNAPSHOT_FILE="$(snapshot_to_file)"
  if python3 - <<'PY' "$SNAPSHOT_FILE" "$1"
import json, sys
with open(sys.argv[1]) as fh:
    data = json.load(fh)["snapshot"]
rows = data["terminal"]["rows_data"]
text = "\n".join("".join(cell["text"] for cell in row["cells"]) for row in rows)
raise SystemExit(0 if sys.argv[2] in text else 1)
PY
  then
    rm -f "$SNAPSHOT_FILE"
    return 0
  fi
  rm -f "$SNAPSHOT_FILE"
  sleep 0.1
done
return 1
}

wait_for_terminal_snapshot() {
for _ in $(seq 1 30); do
  SNAPSHOT_FILE="$(snapshot_to_file)"
  if python3 - <<'PY' "$SNAPSHOT_FILE"
import json, sys
with open(sys.argv[1]) as fh:
    data = json.load(fh)["snapshot"]
raise SystemExit(0 if data.get("terminal") else 1)
PY
  then
    rm -f "$SNAPSHOT_FILE"
    return 0
  fi
  rm -f "$SNAPSHOT_FILE"
  sleep 0.1
done
return 1
}

assert_active_tab_and_row0() {
for _ in $(seq 1 40); do
  SNAPSHOT_FILE="$(snapshot_to_file)"
  if python3 - <<'PY' "$SNAPSHOT_FILE" "$1" "$2"
import json, sys
with open(sys.argv[1]) as fh:
    data = json.load(fh)["snapshot"]
active = data["active_tab"]
line = "".join(cell["text"] for cell in data["terminal"]["rows_data"][0]["cells"])
needle = sys.argv[2]
expected_tab = int(sys.argv[3])
raise SystemExit(0 if active == expected_tab and needle in line else 1)
PY
  then
    rm -f "$SNAPSHOT_FILE"
    return 0
  fi
  rm -f "$SNAPSHOT_FILE"
  sleep 0.1
done
return 1
}

assert_visible_pane_count() {
for _ in $(seq 1 40); do
  SNAPSHOT_FILE="$(snapshot_to_file)"
  if python3 - <<'PY' "$SNAPSHOT_FILE" "$1"
import json, sys
with open(sys.argv[1]) as fh:
    data = json.load(fh)["snapshot"]
raise SystemExit(0 if len(data["visible_panes"]) == int(sys.argv[2]) else 1)
PY
  then
    rm -f "$SNAPSHOT_FILE"
    return 0
  fi
  rm -f "$SNAPSHOT_FILE"
  sleep 0.1
done
return 1
}

assert_focused_pane() {
for _ in $(seq 1 40); do
  SNAPSHOT_FILE="$(snapshot_to_file)"
  if python3 - <<'PY' "$SNAPSHOT_FILE" "$1"
import json, sys
with open(sys.argv[1]) as fh:
    data = json.load(fh)["snapshot"]
raise SystemExit(0 if data["focused_pane"] == int(sys.argv[2]) else 1)
PY
  then
    rm -f "$SNAPSHOT_FILE"
    return 0
  fi
  rm -f "$SNAPSHOT_FILE"
  sleep 0.1
done
return 1
}

pane_info() {
python3 - <<'PY' "$1" "$2"
import json, sys
with open(sys.argv[1]) as fh:
    data = json.load(fh)["snapshot"]
side = sys.argv[2]
panes = sorted(data["visible_panes"], key=lambda pane: pane["frame"]["x"])
pane = panes[0 if side == "left" else -1]
frame = pane["frame"]
print(
    pane["pane_id"],
    frame["x"] + frame["width"] / 2.0,
    frame["y"] + frame["height"] / 2.0,
    frame["width"],
)
PY
}

assert_pane_contains() {
for _ in $(seq 1 40); do
  SNAPSHOT_FILE="$(snapshot_to_file)"
  if python3 - <<'PY' "$SNAPSHOT_FILE" "$1" "$2"
import json, sys
with open(sys.argv[1]) as fh:
    data = json.load(fh)["snapshot"]
pane_id = int(sys.argv[2])
needle = sys.argv[3]
pane = next((pane for pane in data["pane_terminals"] if pane["pane_id"] == pane_id), None)
if pane is None:
    raise SystemExit(1)
rows = pane["terminal"]["rows_data"]
text = "\n".join("".join(cell["text"] for cell in row["cells"]) for row in rows)
raise SystemExit(0 if needle in text else 1)
PY
  then
    rm -f "$SNAPSHOT_FILE"
    return 0
  fi
  rm -f "$SNAPSHOT_FILE"
  sleep 0.1
done
return 1
}

assert_pane_width_increased() {
for _ in $(seq 1 40); do
  SNAPSHOT_FILE="$(snapshot_to_file)"
  if python3 - <<'PY' "$SNAPSHOT_FILE" "$1" "$2" "$3"
import json, sys
with open(sys.argv[1]) as fh:
    data = json.load(fh)["snapshot"]
pane_id = int(sys.argv[2])
before = float(sys.argv[3])
minimum_delta = float(sys.argv[4])
pane = next((pane for pane in data["visible_panes"] if pane["pane_id"] == pane_id), None)
if pane is None:
    raise SystemExit(1)
after = float(pane["frame"]["width"])
raise SystemExit(0 if after > before + minimum_delta else 1)
PY
  then
    rm -f "$SNAPSHOT_FILE"
    return 0
  fi
  rm -f "$SNAPSHOT_FILE"
  sleep 0.1
done
return 1
}

wait_for_exit() {
  for _ in $(seq 1 80); do
    if ! kill -0 "$1" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

cargo build >/dev/null

BOO_GUI_TEST_SOCKET="$GUI_TEST_SOCKET" target/debug/boo --socket "$SOCKET" >/dev/null 2>&1 &
GUI_PID=$!

if ! wait_for_ready; then
  echo "boo GUI test sockets not ready" >&2
  exit 1
fi

send_gui_text "abc"

if ! assert_snapshot_contains "abc"; then
  echo "typed text never appeared in snapshot" >&2
  exit 1
fi

send_gui_key "enter"

send_gui_appkey "q"
send_gui_appkey "w"
send_gui_appkey "e"

if ! assert_snapshot_contains "qwe"; then
  echo "raw appkey typing never appeared in snapshot" >&2
  exit 1
fi

send_gui_text "printf TAB1_MARKER_123"
send_gui_key "enter"

if ! assert_snapshot_contains "TAB1_MARKER_123"; then
  echo "tab 1 marker never appeared in snapshot" >&2
  exit 1
fi

send_gui_command "new-tab"

if ! assert_active_tab_and_row0 "~/" 1; then
  echo "new tab did not become active with a fresh prompt" >&2
  exit 1
fi

send_gui_appkey "ctrl+s"
send_gui_appkey "shift+0x27"

if ! assert_visible_pane_count 2; then
  echo "prefix split key did not create a second pane" >&2
  exit 1
fi

SNAPSHOT_FILE="$(snapshot_to_file)"
read -r LEFT_PANE LEFT_X LEFT_Y LEFT_W <<<"$(pane_info "$SNAPSHOT_FILE" left)"
read -r RIGHT_PANE RIGHT_X RIGHT_Y RIGHT_W <<<"$(pane_info "$SNAPSHOT_FILE" right)"
rm -f "$SNAPSHOT_FILE"

if ! assert_focused_pane "$RIGHT_PANE"; then
  echo "new split did not focus the new pane" >&2
  exit 1
fi

send_gui_command "next-pane"

if ! assert_focused_pane "$LEFT_PANE"; then
  echo "next-pane did not cycle focus to the left pane" >&2
  exit 1
fi

send_gui_command "prev-pane"

if ! assert_focused_pane "$RIGHT_PANE"; then
  echo "prev-pane did not cycle focus back to the right pane" >&2
  exit 1
fi

send_gui_appkey "ctrl+s"
send_gui_appkey "h"

if ! assert_focused_pane "$LEFT_PANE"; then
  echo "directional pane focus did not move left" >&2
  exit 1
fi

send_gui_text "printf LEFTPANE"
send_gui_key "enter"

if ! assert_pane_contains "$LEFT_PANE" "LEFTPANE"; then
  echo "left pane never received focused text" >&2
  exit 1
fi

send_gui_click "$RIGHT_X" "$RIGHT_Y"

if ! assert_focused_pane "$RIGHT_PANE"; then
  echo "click-to-focus did not move focus to right pane" >&2
  exit 1
fi

send_gui_appkey "ctrl+s"
send_gui_appkey "shift+h"

if ! assert_pane_width_increased "$RIGHT_PANE" "$RIGHT_W" 20.0; then
  echo "tmux-style resize did not expand the focused pane by cell count" >&2
  exit 1
fi

send_gui_text "printf RIGHTPANE"
send_gui_key "enter"

if ! assert_pane_contains "$RIGHT_PANE" "RIGHTPANE"; then
  echo "right pane never received clicked text" >&2
  exit 1
fi

SNAPSHOT_FILE="$(snapshot_to_file)"
if python3 - <<'PY' "$SNAPSHOT_FILE"
import json, sys
with open(sys.argv[1]) as fh:
    line = "".join(cell["text"] for cell in json.load(fh)["snapshot"]["terminal"]["rows_data"][0]["cells"])
raise SystemExit(1 if "TAB1_MARKER_123" in line else 0)
PY
then
  :
else
  rm -f "$SNAPSHOT_FILE"
  echo "tab 1 marker leaked into new tab snapshot" >&2
  exit 1
fi
rm -f "$SNAPSHOT_FILE"

kill "$GUI_PID" >/dev/null 2>&1 || true
wait "$GUI_PID" >/dev/null 2>&1 || true
target/debug/boo quit-server --socket "$SOCKET" >/dev/null 2>&1 || true
rm -f "$SOCKET" "$SOCKET.stream" "$GUI_TEST_SOCKET"

BOO_GUI_TEST_SOCKET="$GUI_TEST_SOCKET" target/debug/boo --socket "$SOCKET" >/dev/null 2>&1 &
GUI_PID=$!

if ! wait_for_ready; then
  echo "boo GUI test sockets not ready for exit case" >&2
  exit 1
fi

if ! wait_for_terminal_snapshot; then
  echo "boo GUI terminal snapshot not ready for exit case" >&2
  exit 1
fi

send_gui_text "exit"
send_gui_key "enter"

if ! wait_for_exit "$GUI_PID"; then
  echo "boo GUI did not exit after shell exit" >&2
  exit 1
fi
