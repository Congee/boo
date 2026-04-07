#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${OSTYPE:-}" == darwin* ]]; then
  echo "this helper is intended for Linux perf runs" >&2
  exit 1
fi

if ! command -v perf >/dev/null 2>&1; then
  echo "perf is not installed or not in PATH" >&2
  exit 1
fi

if [[ ! -x target/profiling/boo ]]; then
  echo "target/profiling/boo is missing; run 'cargo build --profile profiling' first" >&2
  exit 1
fi

exec perf record -g "$PWD/target/profiling/boo" "$@"
