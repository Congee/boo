#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <ssh-host> [local-socket]" >&2
  exit 2
fi

host="$1"
local_socket="${2:-/tmp/boo-remote-clients.sock}"
remote_socket="/tmp/boo-remote-clients-${host//[^A-Za-z0-9._-]/_}.sock"
out_json="/tmp/boo-remote-clients.json"
control_path="/tmp/boo-${host//[^A-Za-z0-9._-]/_}.ssh-ctl"

rm -f "${local_socket}" "${local_socket}.stream" "${control_path}"

./target/debug/boo remote-clients \
  --host "$host" \
  --remote-socket "$remote_socket" \
  --socket "$local_socket" >"$out_json"

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
