#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <ssh-host> [local-socket]" >&2
  exit 2
fi

host="$1"
local_socket="${2:-/tmp/boo-remote-nix-profile.sock}"
control_path="/tmp/boo-${host//[^A-Za-z0-9._-]/_}.ssh-ctl"

rm -f "${local_socket}" "${local_socket}.stream" "${control_path}"

./target/debug/boo new-tab \
  --host "$host" \
  --remote-prefer-nix-profile-binary \
  --socket "$local_socket"

python3 scripts/ui-test-client.py --socket "$local_socket" snapshot >/tmp/boo-remote-nix-profile.json

echo "nix-profile bootstrap snapshot: /tmp/boo-remote-nix-profile.json"
echo "host: ${host}"
echo "local forwarded socket: ${local_socket}"
