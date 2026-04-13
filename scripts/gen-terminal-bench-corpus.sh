#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

OUT_DIR="${OUT_DIR:-bench/generated}"
PLAIN_MB="${PLAIN_MB:-32}"
WRAP_MB="${WRAP_MB:-32}"
UNICODE_MB="${UNICODE_MB:-16}"
PAGER_MB="${PAGER_MB:-32}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/gen-terminal-bench-corpus.sh

Environment overrides:
  OUT_DIR      Output directory. Default: bench/generated
  PLAIN_MB     Size of plain-text corpus in MiB. Default: 32
  WRAP_MB      Size of wrapped-line corpus in MiB. Default: 32
  UNICODE_MB   Size of unicode corpus in MiB. Default: 16
  PAGER_MB     Size of pager corpus in MiB. Default: 32

Outputs:
  plain-<N>mb.txt
  wrap-<N>mb.txt
  unicode-<N>mb.txt
  pager-<N>mb.txt
  ansi-truecolor.sh
  cursor-storm.sh
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

mkdir -p "$OUT_DIR"

generate_fixed_size_file() {
  local target_mb="$1"
  local out_path="$2"
  local mode="$3"

  python3 - <<'PY' "$target_mb" "$out_path" "$mode"
import os
import sys
from pathlib import Path

target_mb = int(sys.argv[1])
out_path = Path(sys.argv[2])
mode = sys.argv[3]
target_bytes = target_mb * 1024 * 1024

def plain_line(i: int) -> str:
    return f"{i:09d} plain throughput line with fixed-width payload abcdefghijklmnopqrstuvwxyz 0123456789\n"

def wrap_line(i: int) -> str:
    payload = (
        f"{i:09d} "
        + "wrap-stress "
        + "".join(f"{(i + j) % 10}" for j in range(256))
        + " "
        + ("abcdefghijklmnopqrstuvwxyz" * 24)
    )
    return payload + "\n"

def unicode_line(i: int) -> str:
    samples = [
        "ASCII",
        "コンニチハ",
        "你好世界",
        "한글테스트",
        "emoji 👩🏽\u200d💻 🚀 🧪",
        "combining e\u0301 a\u0308 o\u0302",
        "zwj family 👨\u200d👩\u200d👧\u200d👦",
        "box ─│┌┐└┘",
    ]
    body = " | ".join(samples)
    return f"{i:09d} {body}\n"

def pager_line(i: int) -> str:
    section = i // 40
    return f"section {section:06d} line {i:09d} pager benchmark content for less -N and scroll cadence testing\n"

generators = {
    "plain": plain_line,
    "wrap": wrap_line,
    "unicode": unicode_line,
    "pager": pager_line,
}

line_for = generators[mode]
written = 0
i = 0
with out_path.open("w", encoding="utf-8", newline="") as fh:
    while written < target_bytes:
        line = line_for(i)
        fh.write(line)
        written += len(line.encode("utf-8"))
        i += 1
PY
}

generate_ansi_script() {
  local out_path="$1"
  python3 - <<'PY' "$out_path"
from pathlib import Path
path = Path(__import__("sys").argv[1])
path.write_text("""#!/usr/bin/env bash
set -euo pipefail

rows="${1:-800}"
cols="${2:-120}"

for ((row = 0; row < rows; row++)); do
  for ((col = 0; col < cols; col++)); do
    r=$(((row * 5 + col * 3) % 256))
    g=$(((row * 7 + col * 11) % 256))
    b=$(((row * 13 + col * 17) % 256))
    printf '\\033[38;2;%d;%d;%dm' "$r" "$g" "$b"
    printf '%X' $(((row + col) % 16))
  done
  printf '\\033[0m\\n'
done
""", encoding="utf-8")
path.chmod(0o755)
PY
}

generate_cursor_storm_script() {
  local out_path="$1"
  python3 - <<'PY' "$out_path"
from pathlib import Path
path = Path(__import__("sys").argv[1])
path.write_text("""#!/usr/bin/env bash
set -euo pipefail

frames="${1:-1500}"
width="${2:-100}"
height="${3:-30}"

printf '\\033[2J\\033[H'
for ((frame = 0; frame < frames; frame++)); do
  row=$(((frame % height) + 1))
  col=$((((frame * 3) % width) + 1))
  printf '\\033[%d;%dH' "$row" "$col"
  printf 'frame=%06d row=%02d col=%03d' "$frame" "$row" "$col"
done
printf '\\033[0m\\n'
""", encoding="utf-8")
path.chmod(0o755)
PY
}

generate_fixed_size_file "$PLAIN_MB" "$OUT_DIR/plain-${PLAIN_MB}mb.txt" plain
generate_fixed_size_file "$WRAP_MB" "$OUT_DIR/wrap-${WRAP_MB}mb.txt" wrap
generate_fixed_size_file "$UNICODE_MB" "$OUT_DIR/unicode-${UNICODE_MB}mb.txt" unicode
generate_fixed_size_file "$PAGER_MB" "$OUT_DIR/pager-${PAGER_MB}mb.txt" pager
generate_ansi_script "$OUT_DIR/ansi-truecolor.sh"
generate_cursor_storm_script "$OUT_DIR/cursor-storm.sh"

printf 'generated benchmark corpus in %s\n' "$OUT_DIR"
ls -lh "$OUT_DIR"
