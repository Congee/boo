#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <ssh-host> [<remote-binary>|--nix-profile] [local-socket]" >&2
  exit 2
fi

host="$1"
remote_binary="${2:-}"
local_socket="${3:-/tmp/boo-remote-verify.sock}"
remote_socket="/tmp/boo-remote-verify-${host//[^A-Za-z0-9._-]/_}.sock"
prefer_nix_profile_binary=0

if [[ "$remote_binary" == "--nix-profile" ]]; then
  prefer_nix_profile_binary=1
  remote_binary=""
  local_socket="${3:-/tmp/boo-remote-verify.sock}"
fi

if [[ -z "$remote_binary" && "$prefer_nix_profile_binary" -eq 0 ]]; then
  echo "usage: $0 <ssh-host> [<remote-binary>|--nix-profile] [local-socket]" >&2
  exit 2
fi

rm -f "${local_socket}" "${local_socket}.stream" "/tmp/boo-${host//[^A-Za-z0-9._-]/_}.ssh-ctl"

if [[ "$prefer_nix_profile_binary" -eq 1 ]]; then
  ./target/debug/boo new-session \
    --host "$host" \
    --remote-prefer-nix-profile-binary \
    --remote-socket "$remote_socket" \
    --socket "$local_socket"
else
  ./target/debug/boo new-session \
    --host "$host" \
    --remote-binary "$remote_binary" \
    --remote-socket "$remote_socket" \
    --socket "$local_socket"
fi

python3 scripts/ui-test-client.py --socket "$local_socket" snapshot >/tmp/boo-remote-snapshot.json
python3 scripts/remote-stream-client.py --socket "$local_socket" >/tmp/boo-remote-stream.json

echo "control snapshot: /tmp/boo-remote-snapshot.json"
echo "stream session list: /tmp/boo-remote-stream.json"
