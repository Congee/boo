#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v samply >/dev/null 2>&1; then
  echo "samply is not installed; run 'cargo install samply'" >&2
  exit 1
fi

if [[ ! -x target/profiling/boo ]]; then
  echo "target/profiling/boo is missing; run 'cargo build --profile profiling' first" >&2
  exit 1
fi

if [[ "${OSTYPE:-}" == darwin* ]]; then
  exec samply record "$PWD/scripts/profiling-boo.sh" "$@"
else
  exec samply record "$PWD/target/profiling/boo" "$@"
fi
