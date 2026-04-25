#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 3 ]]; then
  echo "usage: $0 <ssh-host> [<remote-binary>|--nix-profile] [local-socket]" >&2
  exit 2
fi

host="$1"
remote_binary="${2:-}"
local_socket="${3:-/tmp/boo-remote-clients.sock}"
remote_socket="/tmp/boo-remote-clients-${host//[^A-Za-z0-9._-]/_}.sock"
out_json="/tmp/boo-remote-clients.json"
control_path="/tmp/boo-${host//[^A-Za-z0-9._-]/_}.ssh-ctl"
prefer_nix_profile_binary=0

if [[ "$remote_binary" == "--nix-profile" ]]; then
  prefer_nix_profile_binary=1
  remote_binary=""
fi

if [[ "$remote_binary" == /tmp/* || "$remote_binary" == *.sock ]]; then
  local_socket="$remote_binary"
  remote_binary=""
fi

rm -f "${local_socket}" "${local_socket}.stream" "${control_path}"

cmd=(./target/debug/boo --trace-filter error remote-clients
  --host "$host"
  --remote-socket "$remote_socket"
  --socket "$local_socket")

if [[ "$prefer_nix_profile_binary" -eq 1 ]]; then
  cmd+=(--remote-prefer-nix-profile-binary)
elif [[ -n "$remote_binary" ]]; then
  cmd+=(--remote-binary "$remote_binary")
fi

"${cmd[@]}" >"$out_json"

python3 - "$out_json" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    data = json.load(handle)
if not isinstance(data, dict):
    raise SystemExit(f"expected object JSON from remote-clients, got: {type(data).__name__}")
if not isinstance(data.get("clients"), list):
    raise SystemExit("expected 'clients' list in remote-clients JSON")
PY

echo "remote clients snapshot: ${out_json}"
