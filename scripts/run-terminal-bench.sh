#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

CORPUS_DIR="${CORPUS_DIR:-bench/generated}"
SCENARIO="${1:-}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/run-terminal-bench.sh <scenario> [-- print-only]

Scenarios:
  plain-cat      cat plain large text corpus
  wrap-cat       cat long wrapped-line corpus
  unicode-cat    cat unicode-heavy corpus
  pager-less     open large pager corpus in less -N
  ansi-churn     run dense ANSI truecolor churn
  cursor-storm   run cursor motion/update storm

Examples:
  bash scripts/run-terminal-bench.sh plain-cat
  bash scripts/run-terminal-bench.sh pager-less
  bash scripts/run-terminal-bench.sh unicode-cat -- print-only

Notes:
  - Generates the benchmark corpus automatically if missing.
  - Prints the exact terminal command before executing it.
  - `-- print-only` prints the command without executing it.
EOF
}

if [[ -z "$SCENARIO" || "$SCENARIO" == "-h" || "$SCENARIO" == "--help" ]]; then
  usage
  exit 0
fi

PRINT_ONLY=0
if [[ "${2:-}" == "--" && "${3:-}" == "print-only" ]]; then
  PRINT_ONLY=1
elif [[ "${2:-}" == "--print-only" || "${2:-}" == "print-only" ]]; then
  PRINT_ONLY=1
fi

ensure_corpus() {
  local required=(
    "$CORPUS_DIR/plain-32mb.txt"
    "$CORPUS_DIR/wrap-32mb.txt"
    "$CORPUS_DIR/unicode-16mb.txt"
    "$CORPUS_DIR/pager-32mb.txt"
    "$CORPUS_DIR/ansi-truecolor.sh"
    "$CORPUS_DIR/cursor-storm.sh"
  )
  for path in "${required[@]}"; do
    if [[ ! -e "$path" ]]; then
      bash scripts/gen-terminal-bench-corpus.sh
      break
    fi
  done
}

ensure_corpus

COMMAND=""
case "$SCENARIO" in
  plain-cat)
    COMMAND="cat '$CORPUS_DIR/plain-32mb.txt'"
    ;;
  wrap-cat)
    COMMAND="cat '$CORPUS_DIR/wrap-32mb.txt'"
    ;;
  unicode-cat)
    COMMAND="cat '$CORPUS_DIR/unicode-16mb.txt'"
    ;;
  pager-less)
    COMMAND="less -N '$CORPUS_DIR/pager-32mb.txt'"
    ;;
  ansi-churn)
    COMMAND="bash '$CORPUS_DIR/ansi-truecolor.sh'"
    ;;
  cursor-storm)
    COMMAND="bash '$CORPUS_DIR/cursor-storm.sh'"
    ;;
  *)
    echo "unknown scenario: $SCENARIO" >&2
    usage >&2
    exit 1
    ;;
esac

printf 'scenario=%s\n' "$SCENARIO"
printf 'command=%s\n' "$COMMAND"

if [[ "$PRINT_ONLY" -eq 1 ]]; then
  exit 0
fi

exec /bin/zsh -lc "$COMMAND"
