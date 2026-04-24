#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

VT_LIB_DIR="${BOO_VT_LIB_DIR:-${VT_LIB_DIR:-}}"

usage() {
  cat <<'EOF'
Usage: scripts/profiling-boo.sh [--vt-lib-dir PATH] [--] [boo args...]

Runs target/profiling/boo. When needed, pass --vt-lib-dir or run from
nix develop so BOO_VT_LIB_DIR points at the Nix-built libghostty-vt. This
script does not scan target/libghostty-vt for a second library copy.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --vt-lib-dir)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for $1" >&2
        usage >&2
        exit 2
      fi
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
      break
      ;;
  esac
done

if [[ ! -x target/profiling/boo ]]; then
  echo "target/profiling/boo is missing; run 'cargo build --profile profiling' first" >&2
  exit 1
fi

if [[ -n "$VT_LIB_DIR" ]]; then
  export BOO_VT_LIB_DIR="$VT_LIB_DIR"
  case "$(uname -s)" in
    Darwin)
      export DYLD_LIBRARY_PATH="$VT_LIB_DIR${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
      ;;
    *)
      export LD_LIBRARY_PATH="$VT_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
      ;;
  esac
fi

exec "$PWD/target/profiling/boo" "$@"
