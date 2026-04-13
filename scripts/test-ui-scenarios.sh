#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_ROOT="${BOO_TEST_CONFIG_ROOT:-/tmp/boo-ui-scenarios}"
CONFIG_DIR="$CONFIG_ROOT/boo"
SOCKET_PATH="${BOO_TEST_SOCKET:-/tmp/boo-ui-scenarios.sock}"
LOG_PATH="${BOO_TEST_LOG:-/tmp/boo-ui-scenarios.log}"
LOG_LEVEL="${BOO_TEST_LOG_LEVEL:-warn}"
EXPECTED_FONT="$(fc-match -f '%{family[0]}' 'JetBrains Mono' 2>/dev/null || printf 'JetBrains Mono')"
VT_LIB_DIR="${VT_LIB_DIR:-}"

find_vt_lib_dir() {
  local target="${TARGET:-$(rustc -vV | awk '/host:/ {print $2}')}"
  local candidates=(
    "$ROOT_DIR/target/libghostty-vt/$target/debug/lib"
    "$ROOT_DIR/target/libghostty-vt/$target/profiling/lib"
    "$ROOT_DIR/target/libghostty-vt/$target/release/lib"
  )
  local path
  for path in "${candidates[@]}"; do
    if [[ -e "$path/libghostty-vt.so.0" || -e "$path/libghostty-vt.so" ]]; then
      printf '%s\n' "$path"
      return 0
    fi
  done
  return 1
}

mkdir -p "$CONFIG_DIR"
cat > "$CONFIG_DIR/config.boo" <<EOF
control-socket = $SOCKET_PATH
font-family = JetBrains Mono
font-size = 18
background-opacity = 0.72
background-opacity-cells = true
keybind = ctrl+shift+t = new_tab
keybind = ctrl+a>c = new_tab
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
  if [[ -z "$VT_LIB_DIR" ]]; then
    VT_LIB_DIR="$(find_vt_lib_dir || true)"
  fi
  if [[ -n "$VT_LIB_DIR" ]]; then
    LD_LIBRARY_PATH="$VT_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
      RUST_LOG="$LOG_LEVEL" XDG_CONFIG_HOME="$CONFIG_ROOT" target/debug/boo >"$LOG_PATH" 2>&1
  else
    RUST_LOG="$LOG_LEVEL" XDG_CONFIG_HOME="$CONFIG_ROOT" target/debug/boo >"$LOG_PATH" 2>&1
  fi
) &
BOO_PID=$!

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" wait-ready >/tmp/boo-ui-initial.json

snapshot_matches() {
  local path="$1"
  local expected="$2"
  python3 - "$path" "$expected" <<'PY'
import json
import sys

path = sys.argv[1]
expected = sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    payload = json.load(f)

snapshot = payload["snapshot"]
env = {
    "tabs": snapshot["tabs"],
    "visible_panes": snapshot["visible_panes"],
    "active_tab": snapshot["active_tab"],
    "focused_pane": snapshot["focused_pane"],
    "appearance": snapshot["appearance"],
    "copy_mode": snapshot["copy_mode"],
    "search": snapshot["search"],
    "command_prompt": snapshot["command_prompt"],
    "scrollbar": snapshot["scrollbar"],
    "terminal": snapshot["terminal"],
}

safe_builtins = {"len": len, "any": any, "all": all, "abs": abs}
if not eval(expected, {"__builtins__": safe_builtins}, env):
    raise SystemExit(f"assertion failed: {expected}\nsnapshot={json.dumps(snapshot)}")
PY
}

assert_snapshot() {
  local path="$1"
  local expected="$2"
  snapshot_matches "$path" "$expected"
}

capture_snapshot() {
  local path="$1"
  python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" snapshot >"$path"
}

capture_clipboard() {
  local path="$1"
  python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request get-clipboard >"$path"
}

snapshot_has_terminal() {
  local path="$1"
  python3 - "$path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    payload = json.load(f)

raise SystemExit(0 if payload["snapshot"].get("terminal") is not None else 1)
PY
}

terminal_contains() {
  local path="$1"
  local expected="$2"
  python3 - "$path" "$expected" <<'PY'
import json
import sys

path = sys.argv[1]
expected = sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    payload = json.load(f)

terminal = payload["snapshot"].get("terminal")
if not terminal:
    raise SystemExit("terminal snapshot missing")

lines = []
for row in terminal["rows_data"]:
    line = "".join(cell["text"] or " " for cell in row["cells"]).rstrip()
    lines.append(line)

text = "\n".join(lines)
if expected not in text:
    raise SystemExit(f"missing terminal text: {expected!r}\nterminal={text!r}")
PY
}

terminal_run_matches() {
  local path="$1"
  local text="$2"
  local predicate="$3"
  python3 - "$path" "$text" "$predicate" <<'PY'
import json
import sys

path = sys.argv[1]
needle = sys.argv[2]
predicate = sys.argv[3]
with open(path, "r", encoding="utf-8") as f:
    payload = json.load(f)

terminal = payload["snapshot"].get("terminal")
if not terminal:
    raise SystemExit("terminal snapshot missing")

rows = terminal["rows_data"]
for row in rows:
    cells = row["cells"]
    texts = [cell["text"] or " " for cell in cells]
    line = "".join(texts)
    start = line.find(needle)
    if start == -1:
        continue

    matched = cells[start:start + len(needle)]
    env = {
        "cells": matched,
        "all": all,
        "any": any,
        "len": len,
    }
    if eval(predicate, {"__builtins__": {}}, env):
        raise SystemExit(0)

raise SystemExit(f"missing terminal run {needle!r} matching {predicate!r}")
PY
}

wait_snapshot() {
  local path="$1"
  local expected="$2"
  for _ in $(seq 1 30); do
    capture_snapshot "$path"
    if snapshot_matches "$path" "$expected" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  assert_snapshot "$path" "$expected"
}

wait_terminal_contains() {
  local path="$1"
  local expected="$2"
  for _ in $(seq 1 40); do
    capture_snapshot "$path"
    if terminal_contains "$path" "$expected" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  terminal_contains "$path" "$expected"
}

wait_terminal_run_matches() {
  local path="$1"
  local text="$2"
  local predicate="$3"
  for _ in $(seq 1 40); do
    capture_snapshot "$path"
    if terminal_run_matches "$path" "$text" "$predicate" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  terminal_run_matches "$path" "$text" "$predicate"
}

terminal_snapshot_matches() {
  local path="$1"
  local predicate="$2"
  python3 - "$path" "$predicate" <<'PY'
import json
import sys

path = sys.argv[1]
predicate = sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    payload = json.load(f)

terminal = payload["snapshot"].get("terminal")
if not terminal:
    raise SystemExit("terminal snapshot missing")

lines = []
for row in terminal["rows_data"]:
    line = "".join(cell["text"] or " " for cell in row["cells"]).rstrip()
    lines.append(line)

text = "\n".join(lines)
env = {
    "terminal": terminal,
    "text": text,
}
if not eval(predicate, {"__builtins__": {}}, env):
    raise SystemExit(f"assertion failed: {predicate}\nterminal={text!r}")
PY
}

wait_terminal_snapshot_matches() {
  local path="$1"
  local predicate="$2"
  for _ in $(seq 1 40); do
    capture_snapshot "$path"
    if terminal_snapshot_matches "$path" "$predicate" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  terminal_snapshot_matches "$path" "$predicate"
}

clipboard_matches() {
  local path="$1"
  local expected="$2"
  python3 - "$path" "$expected" <<'PY'
import json
import sys

path = sys.argv[1]
expected = sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    payload = json.load(f)

text = payload.get("text")
if text != expected:
    raise SystemExit(f"clipboard mismatch: expected {expected!r}, got {text!r}")
PY
}

wait_clipboard_matches() {
  local path="$1"
  local expected="$2"
  for _ in $(seq 1 30); do
    capture_clipboard "$path"
    if clipboard_matches "$path" "$expected" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  clipboard_matches "$path" "$expected"
}

assert_snapshot /tmp/boo-ui-initial.json 'len(tabs) == 1 and len(visible_panes) == 1 and active_tab == 0'
assert_snapshot /tmp/boo-ui-initial.json "\"$EXPECTED_FONT\" in appearance[\"font_families\"] and abs(appearance[\"font_size\"] - 18.0) < 0.01 and abs(appearance[\"background_opacity\"] - 0.72) < 0.01 and appearance[\"background_opacity_cells\"]"

HAS_TERMINAL_SNAPSHOT=0
if snapshot_has_terminal /tmp/boo-ui-initial.json; then
  HAS_TERMINAL_SNAPSHOT=1
  wait_snapshot /tmp/boo-ui-initial.json 'terminal is not None and terminal["cols"] > 0 and terminal["rows"] > 0'
fi

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-text 'text=pwd\r' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_contains /tmp/boo-ui-after-pwd.json "$ROOT_DIR"
fi

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-text 'text=printf BOOX' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_snapshot_matches /tmp/boo-ui-backspace-before.json '"BOOX" in text'
fi
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=backspace >/tmp/boo-ui-backspace-response.json
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_snapshot_matches /tmp/boo-ui-backspace-after.json '"BOOX" not in text and "BOO" in text'
fi
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-text 'text=Y\r' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_contains /tmp/boo-ui-after-backspace.json "BOOY"
fi

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-vt 'text=\x1b[4 q' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_snapshot_matches /tmp/boo-ui-after-cursor-underline.json 'terminal["cursor"]["style"] == 2'
fi
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-vt 'text=\x1b[2 q' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_snapshot_matches /tmp/boo-ui-after-cursor-block.json 'terminal["cursor"]["style"] == 1'
fi

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=ctrl+shift+t >/tmp/boo-ui-new-tab-response.json
wait_snapshot /tmp/boo-ui-after-new-tab.json 'len(tabs) == 2 and active_tab == 1 and len(visible_panes) == 1'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-text 'text=printf TAB2_OK\\r' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_contains /tmp/boo-ui-after-tab2-text.json "TAB2_OK"
fi

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request prev-tab >/tmp/boo-ui-prev-tab-response.json
wait_snapshot /tmp/boo-ui-after-prev-tab.json 'len(tabs) == 2 and active_tab == 0 and len(visible_panes) == 1'
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_contains /tmp/boo-ui-after-prev-tab-content.json "$ROOT_DIR"
fi

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=ctrl+a >/dev/null
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=c >/dev/null
wait_snapshot /tmp/boo-ui-after-prefix-new-tab.json 'len(tabs) == 3 and active_tab == 2 and len(visible_panes) == 1'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request prev-tab >/dev/null
wait_snapshot /tmp/boo-ui-after-prefix-prev.json 'len(tabs) == 3 and active_tab == 1'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request prev-tab >/dev/null
wait_snapshot /tmp/boo-ui-after-prefix-prev2.json 'len(tabs) == 3 and active_tab == 0'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request new-split direction=right >/tmp/boo-ui-split-response.json
wait_snapshot /tmp/boo-ui-after-split.json 'len(visible_panes) == 2 and any(p["split_direction"] == "horizontal" for p in visible_panes)'
assert_snapshot /tmp/boo-ui-after-split.json 'visible_panes[0]["frame"]["width"] > 0 and visible_panes[1]["frame"]["width"] > 0 and visible_panes[0]["frame"]["x"] < visible_panes[1]["frame"]["x"]'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request resize-viewport cols=140 rows=36 >/tmp/boo-ui-resize-viewport-response.json
wait_snapshot /tmp/boo-ui-after-resize-viewport.json 'len(visible_panes) == 2 and abs((visible_panes[0]["frame"]["width"] + visible_panes[1]["frame"]["width"] + 1.0) - 1680.0) < 1.0 and abs(visible_panes[0]["frame"]["height"] - 900.0) < 1.0 and abs(visible_panes[1]["frame"]["height"] - 900.0) < 1.0'

SECOND_LEAF_ID="$(python3 - <<'PY'
import json
with open("/tmp/boo-ui-after-resize-viewport.json", "r", encoding="utf-8") as f:
    snapshot = json.load(f)["snapshot"]
for pane in snapshot["visible_panes"]:
    if not pane["focused"]:
        print(pane["leaf_id"])
        break
else:
    raise SystemExit("no secondary pane found")
PY
)"

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request focus-surface index="$SECOND_LEAF_ID" >/tmp/boo-ui-focus-response.json
wait_snapshot /tmp/boo-ui-after-focus.json 'len([p for p in visible_panes if p["focused"]]) == 1 and any(p["leaf_id"] == '"$SECOND_LEAF_ID"' and p["focused"] for p in visible_panes)'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-text 'text=printf SPLIT_OK\\r' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_contains /tmp/boo-ui-after-split-text.json "SPLIT_OK"
fi
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-vt 'text=\x1b[1;3;4mSTYLE\x1b[0m' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_run_matches /tmp/boo-ui-after-style-text.json "STYLE" 'all(cell["bold"] and cell["italic"] and cell["underline"] != 0 for cell in cells)'
fi
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-vt 'text=\r\ne\u0301🙂' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_contains /tmp/boo-ui-after-unicode-text.json 'é🙂'
fi
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-vt 'text=\r\nCOPYSEL' >/dev/null
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_terminal_contains /tmp/boo-ui-after-copysel-text.json "COPYSEL"
fi
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request execute-command input=copy-mode >/tmp/boo-ui-copy-mode-copy-response.json
wait_snapshot /tmp/boo-ui-after-copy-mode-copy-open.json 'copy_mode["active"] and copy_mode["selection_mode"] == "none"'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=h >/tmp/boo-ui-copy-mode-copy-h-response.json
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=space >/tmp/boo-ui-copy-mode-copy-space-response.json
wait_snapshot /tmp/boo-ui-after-copy-mode-copy-select.json 'copy_mode["active"] and copy_mode["selection_mode"] == "character" and copy_mode["has_selection_anchor"] and len(copy_mode["selection_rects"]) >= 1'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=y >/tmp/boo-ui-copy-mode-copy-y-response.json
wait_snapshot /tmp/boo-ui-after-copy-mode-copy-done.json 'not copy_mode["active"]'
if [[ "$HAS_TERMINAL_SNAPSHOT" == "1" ]]; then
  wait_clipboard_matches /tmp/boo-ui-clipboard-after-copy.json "L"
fi

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request execute-command input=search >/tmp/boo-ui-search-response.json
wait_snapshot /tmp/boo-ui-after-search-open.json 'search["active"] and search["query"] == ""'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=a >/tmp/boo-ui-search-key-response.json
wait_snapshot /tmp/boo-ui-after-search-input.json 'search["active"] and search["query"] == "a"'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=esc >/tmp/boo-ui-search-esc-response.json
wait_snapshot /tmp/boo-ui-after-search-close.json 'not search["active"]'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request execute-command input=command-prompt >/tmp/boo-ui-command-prompt-response.json
wait_snapshot /tmp/boo-ui-after-command-prompt-open.json 'command_prompt["active"] and command_prompt["input"] == ""'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=s >/tmp/boo-ui-command-prompt-key-s-response.json
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=e >/tmp/boo-ui-command-prompt-key-e-response.json
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=a >/tmp/boo-ui-command-prompt-key-a-response.json
wait_snapshot /tmp/boo-ui-after-command-prompt-input.json 'command_prompt["active"] and command_prompt["input"] == "sea" and "search" in command_prompt["suggestions"]'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=tab >/tmp/boo-ui-command-prompt-tab-response.json
wait_snapshot /tmp/boo-ui-after-command-prompt-tab.json 'command_prompt["active"] and command_prompt["input"] == "search"'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=enter >/tmp/boo-ui-command-prompt-enter-response.json
wait_snapshot /tmp/boo-ui-after-command-prompt-enter.json 'not command_prompt["active"] and search["active"] and search["query"] == ""'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=esc >/tmp/boo-ui-search-after-prompt-esc-response.json
wait_snapshot /tmp/boo-ui-after-search-close-2.json 'not search["active"]'

python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request resize-viewport cols=80 rows=12 >/tmp/boo-ui-scroll-reset-resize-response.json
wait_snapshot /tmp/boo-ui-after-scroll-reset-resize.json 'scrollbar["len"] > 0'
python3 - "$SOCKET_PATH" <<'PY'
import json
import socket
import sys

socket_path = sys.argv[1]
payload = {
    "cmd": "send-vt",
    "text": "".join(f"SCROLL{i:03d}\r\n" for i in range(1, 121)),
}
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
    sock.connect(socket_path)
    sock.sendall(json.dumps(payload).encode("utf-8") + b"\n")
    data = b""
    while not data.endswith(b"\n"):
        chunk = sock.recv(65536)
        if not chunk:
            break
        data += chunk
    response = json.loads(data.decode("utf-8"))
    if response != {"ok": True}:
        raise SystemExit(f"unexpected response: {response}")
PY
wait_snapshot /tmp/boo-ui-after-scroll-fill.json 'scrollbar["total"] > scrollbar["len"] and scrollbar["offset"] + scrollbar["len"] == scrollbar["total"]'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request execute-command input=scroll-page-up >/tmp/boo-ui-scroll-page-up-response.json
wait_snapshot /tmp/boo-ui-after-scroll-page-up.json 'scrollbar["total"] > scrollbar["len"] and scrollbar["offset"] + scrollbar["len"] < scrollbar["total"]'
python3 "$ROOT_DIR/scripts/ui-test-client.py" --socket "$SOCKET_PATH" request send-key key=a >/tmp/boo-ui-scroll-reset-key-response.json
wait_snapshot /tmp/boo-ui-after-scroll-reset-key.json 'scrollbar["total"] > scrollbar["len"] and scrollbar["offset"] + scrollbar["len"] == scrollbar["total"]'

echo "ui scenarios passed"
