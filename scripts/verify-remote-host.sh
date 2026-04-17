#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <ssh-host> <remote-binary> [local-socket]" >&2
  exit 2
fi

host="$1"
remote_binary="$2"
local_socket="${3:-/tmp/boo-remote-verify.sock}"

rm -f "${local_socket}" "${local_socket}.stream" "/tmp/boo-${host//[^A-Za-z0-9._-]/_}.ssh-ctl"

./target/debug/boo new-session \
  --host "$host" \
  --remote-binary "$remote_binary" \
  --socket "$local_socket"

python3 scripts/ui-test-client.py --socket "$local_socket" snapshot >/tmp/boo-remote-snapshot.json
python3 scripts/remote-stream-client.py --socket "$local_socket" >/tmp/boo-remote-stream.json

echo "control snapshot: /tmp/boo-remote-snapshot.json"
echo "stream session list: /tmp/boo-remote-stream.json"
