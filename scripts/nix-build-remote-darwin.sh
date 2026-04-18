#!/usr/bin/env bash
set -euo pipefail

# Backwards-compatible shim. The canonical cross-platform helper is
# scripts/nix-build-remote.sh.
#
# Usage:
#   nix-build-remote-darwin.sh [<ssh-host>] [<flake-attr>] [extra nix build args...]
#
# Defaults:
#   ssh-host    example-mbp.local
#   flake-attr  .#packages.aarch64-darwin.default

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

host="${1:-example-mbp.local}"
flake_attr="${2:-.#packages.aarch64-darwin.default}"
shift $(( $# > 0 ? 1 : 0 ))
shift $(( $# > 0 ? 1 : 0 ))

exec "${root}/scripts/nix-build-remote.sh" "${host}" aarch64-darwin "${flake_attr}" "$@"
