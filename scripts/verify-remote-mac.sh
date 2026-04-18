#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 3 ]]; then
  echo "usage: $0 [ssh-host] [remote-repo] [local-socket]" >&2
  exit 2
fi

host="${1:-example-mbp.local}"
remote_repo="${2:-/Users/example/dev/boo}"
local_socket="${3:-/tmp/boo-remote-mac-verify.sock}"
remote_binary="${remote_repo}/target/debug/boo"
prefer_nix_profile_binary="${BOO_REMOTE_MAC_USE_NIX_PROFILE:-0}"

echo "==> sync sources to ${host}"
bash scripts/sync-remote-mac.sh "$host" "$remote_repo"

echo "==> remote build and smoke tests on ${host}"
ssh "$host" "cd '$remote_repo' && cargo build && cargo test short_and_long_help_are_different -- --nocapture && ./target/debug/boo --help >/dev/null"

echo "==> local SSH forwarding verification through ${host}"
if [[ "$prefer_nix_profile_binary" == "1" ]]; then
  bash scripts/verify-remote-host.sh "$host" --nix-profile "$local_socket"
else
  bash scripts/verify-remote-host.sh "$host" "$remote_binary" "$local_socket"
fi

echo "==> remote path expansion verification through ${host}"
bash scripts/test-remote-path-expansion.sh "$host" "${local_socket%.sock}-paths.sock"

echo "==> stale tunnel recovery verification through ${host}"
bash scripts/test-remote-tunnel-recovery.sh "$host" "$remote_binary" "${local_socket%.sock}-recovery.sock"

echo "==> remote client diagnostics verification through ${host}"
bash scripts/test-remote-clients.sh "$host" "${local_socket%.sock}-clients.sock"

echo "==> local native daemon diagnostics verification"
bash scripts/test-remote-daemon-diagnostics.sh

echo "==> verification complete"
echo "remote host: ${host}"
echo "remote repo: ${remote_repo}"
echo "remote binary: ${remote_binary}"
echo "local forwarded socket: ${local_socket}"
