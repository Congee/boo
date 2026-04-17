#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "usage: $0 <ssh-host> <remote-binary> [local-socket]" >&2
  exit 2
fi

host="$1"
remote_binary="$2"
local_socket="${3:-/tmp/boo-remote-recovery.sock}"
control_path="/tmp/boo-${host//[^A-Za-z0-9._-]/_}.ssh-ctl"

rm -f "${local_socket}" "${local_socket}.stream" "${control_path}"

./target/debug/boo new-session \
  --host "$host" \
  --remote-binary "$remote_binary" \
  --socket "$local_socket"

python3 scripts/ui-test-client.py --socket "$local_socket" snapshot >/tmp/boo-remote-recovery-before.json

ssh -S "$control_path" -O exit "$host" >/dev/null 2>&1 || true

./target/debug/boo ls \
  --host "$host" \
  --remote-binary "$remote_binary" \
  --socket "$local_socket" >/tmp/boo-remote-recovery-ls.txt

python3 scripts/ui-test-client.py --socket "$local_socket" snapshot >/tmp/boo-remote-recovery-after.json

echo "recovery snapshot before: /tmp/boo-remote-recovery-before.json"
echo "recovery ls output: /tmp/boo-remote-recovery-ls.txt"
echo "recovery snapshot after: /tmp/boo-remote-recovery-after.json"
