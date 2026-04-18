#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <ssh-host> [local-socket]" >&2
  exit 2
fi

host="$1"
local_socket="${2:-/tmp/boo-remote-paths.sock}"
remote_socket='${HOME}/Library/Caches/boo-remote-test-'"${host//[^A-Za-z0-9._-]/_}"'.sock'
control_path="/tmp/boo-${host//[^A-Za-z0-9._-]/_}.ssh-ctl"

rm -f "${local_socket}" "${local_socket}.stream" "${control_path}"

./target/debug/boo new-session \
  --host "$host" \
  --remote-binary "~/dev/boo/target/debug/boo" \
  --remote-workdir '$HOME/dev/boo' \
  --remote-socket "$remote_socket" \
  --socket "$local_socket"

python3 scripts/ui-test-client.py --socket "$local_socket" snapshot >/tmp/boo-remote-paths.json

echo "path expansion snapshot: /tmp/boo-remote-paths.json"
