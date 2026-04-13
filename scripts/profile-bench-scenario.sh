#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ $# -lt 1 ]]; then
  cat <<'EOF' >&2
Usage:
  bash scripts/profile-bench-scenario.sh <scenario> [extra profile-macos-sample-client.sh args...]

Examples:
  bash scripts/profile-bench-scenario.sh plain-cat
  bash scripts/profile-bench-scenario.sh unicode-cat --duration 8
EOF
  exit 1
fi

SCENARIO="$1"
shift

COMMAND="$(bash scripts/run-terminal-bench.sh "$SCENARIO" --print-only | awk -F= '/^command=/{print substr($0,9)}')"

if [[ -z "$COMMAND" ]]; then
  echo "failed to resolve scenario command for: $SCENARIO" >&2
  exit 1
fi

WORKLOAD="${COMMAND}\r"
exec bash scripts/profile-macos-sample-client.sh --workload "$WORKLOAD" "$@"
