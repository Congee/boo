#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

LOCK_FILE=".git/index.lock"
LOCK_WAIT_SECONDS="${GIT_SAFE_LOCK_WAIT_SECONDS:-5}"
ROOT_DIR="$(pwd)"

usage() {
  cat <<'EOF'
Usage:
  scripts/git-safe.sh status
  scripts/git-safe.sh commit -m "message"
  scripts/git-safe.sh push [remote] [branch]
  scripts/git-safe.sh clear-lock

Behavior:
  - waits briefly for an active git process to release .git/index.lock
  - removes a stale .git/index.lock only if no git process is active in this repo
  - serializes the git action through this script so commit/push/status do not race
EOF
}

git_processes_for_repo() {
  local output=""
  output="$(ps -axo pid=,command= 2>/dev/null | grep '[[:space:]]git' || true)"
  if [[ -z "$output" ]]; then
    return 1
  fi
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    if [[ "$line" == *"$ROOT_DIR"* ]]; then
      printf '%s\n' "$line"
    fi
  done <<<"$output"
}

active_git_processes_exist() {
  git_processes_for_repo | grep -q .
}

wait_for_lock_release() {
  local deadline=$((SECONDS + LOCK_WAIT_SECONDS))
  while [[ -e "$LOCK_FILE" ]] && active_git_processes_exist; do
    if (( SECONDS >= deadline )); then
      break
    fi
    sleep 0.2
  done
}

clear_stale_lock_if_safe() {
  if [[ ! -e "$LOCK_FILE" ]]; then
    return 0
  fi
  wait_for_lock_release
  if [[ ! -e "$LOCK_FILE" ]]; then
    return 0
  fi
  if active_git_processes_exist; then
    echo "refusing to remove $LOCK_FILE because git is still active in $ROOT_DIR" >&2
    git_processes_for_repo >&2 || true
    return 1
  fi
  rm -f "$LOCK_FILE"
}

ensure_git_ready() {
  clear_stale_lock_if_safe
}

main() {
  local cmd="${1:-}"
  if [[ -z "$cmd" ]]; then
    usage >&2
    exit 1
  fi
  shift || true

  case "$cmd" in
    status)
      ensure_git_ready
      exec git status "$@"
      ;;
    commit)
      ensure_git_ready
      exec git commit "$@"
      ;;
    push)
      ensure_git_ready
      exec git push "$@"
      ;;
    clear-lock)
      clear_stale_lock_if_safe
      ;;
    -h|--help|help)
      usage
      ;;
    *)
      echo "unknown command: $cmd" >&2
      usage >&2
      exit 1
      ;;
  esac
}

main "$@"
