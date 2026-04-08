#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ ! -x target/profiling/boo ]]; then
  echo "target/profiling/boo is missing; run 'cargo build --profile profiling' first" >&2
  exit 1
fi

LIB_DIR="$(find "$PWD/target/libghostty-vt" -path '*/profiling/lib' | head -n 1)"
if [[ -z "${LIB_DIR:-}" ]]; then
  echo "could not locate libghostty-vt dylib directory under target/libghostty-vt" >&2
  exit 1
fi

export DYLD_LIBRARY_PATH="$LIB_DIR${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"

exec "$PWD/target/profiling/boo" "$@"
