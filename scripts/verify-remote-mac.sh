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

echo "==> sync sources to ${host}"
bash scripts/sync-remote-mac.sh "$host" "$remote_repo"

echo "==> remote build and smoke tests on ${host}"
ssh "$host" "cd '$remote_repo' && cargo build && cargo test short_and_long_help_are_different -- --nocapture && ./target/debug/boo --help >/dev/null"

echo "==> local SSH forwarding verification through ${host}"
bash scripts/verify-remote-host.sh "$host" "$remote_binary" "$local_socket"

echo "==> remote path expansion verification through ${host}"
bash scripts/test-remote-path-expansion.sh "$host" "${local_socket%.sock}-paths.sock"

echo "==> stale tunnel recovery verification through ${host}"
bash scripts/test-remote-tunnel-recovery.sh "$host" "$remote_binary" "${local_socket%.sock}-recovery.sock"

echo "==> verification complete"
echo "remote host: ${host}"
echo "remote repo: ${remote_repo}"
echo "remote binary: ${remote_binary}"
echo "local forwarded socket: ${local_socket}"
