#!/usr/bin/env bash
set -euo pipefail

host="${1:-example-mbp.local}"
flake_attr="${2:-.#packages.aarch64-darwin.default}"
shift $(( $# > 0 ? 1 : 0 ))
shift $(( $# > 0 ? 1 : 0 ))

builder="ssh-ng://${host} aarch64-darwin - 10 1 big-parallel"

exec nix build "${flake_attr}" \
  --builders "${builder}" \
  "$@"
