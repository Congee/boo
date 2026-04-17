#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 2 ]]; then
  echo "usage: $0 [ssh-host] [remote-repo]" >&2
  exit 2
fi

host="${1:-example-mbp.local}"
remote_repo="${2:-/Users/example/dev/boo}"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> syncing ${repo_root} to ${host}:${remote_repo}"
rsync -a \
  --exclude .git \
  --exclude target \
  --exclude .beads \
  --exclude .codex \
  --exclude ghostty \
  --exclude nvim.log \
  "${repo_root}/" "${host}:${remote_repo}/"

echo "==> remote source updated"
