#!/usr/bin/env bash
set -euo pipefail

# Portable cross-platform remote Nix builder for Boo.
#
# Usage:
#   nix-build-remote.sh <ssh-host> <target-system> [<flake-attr>] [extra nix build args...]
#
# Examples:
#   # Linux → Mac (Darwin build offloaded to the Mac):
#   ./scripts/nix-build-remote.sh example-mbp aarch64-darwin
#
#   # Mac → Linux (Linux build offloaded to blackbox):
#   ./scripts/nix-build-remote.sh blackbox x86_64-linux
#
#   # Non-default flake attribute + extra flags:
#   ./scripts/nix-build-remote.sh example-mbp aarch64-darwin \
#     .#checks.aarch64-darwin.default --print-build-logs
#
# When <flake-attr> is omitted, defaults to .#packages.<target-system>.default.
#
# This uses `nix build --store ssh-ng://<user>@<ssh-host>` so the build runs as the
# calling user over their own SSH credentials (no `/etc/nix/machines`, no
# `nix.buildMachines` NixOS option, and no root SSH key setup needed on the
# requester). The build output lives in the REMOTE host's Nix store — the local
# machine receives a derivation-level success signal, not a copied-back artifact.
# That fits the "build on the Mac so the Mac has the binary" deploy workflow. If
# you genuinely need the artifact on the local machine too, pair this with a
# `nix copy --from ssh-ng://<user>@<host> <store-path>` afterwards.
#
# Prerequisites:
#   - `ssh <ssh-host>` works non-interactively as the calling user (key-based).
#   - The calling user is a trusted Nix user on the remote (they show as
#     `Trusted: 1` in `nix store info --store ssh-ng://<ssh-host>`).

if [[ $# -lt 2 ]]; then
  cat >&2 <<EOF
usage: $0 <ssh-host> <target-system> [<flake-attr>] [extra nix build args...]
  ssh-host      SSH destination (matches ~/.ssh/config or resolvable hostname)
  target-system Nix platform double (e.g. aarch64-darwin, x86_64-linux)
  flake-attr    Defaults to .#packages.<target-system>.default
EOF
  exit 2
fi

host="$1"
system="$2"
shift 2

attr="${1:-.#packages.${system}.default}"
if [[ $# -gt 0 ]]; then
  shift
fi

# Prefer the calling user on the remote. Callers can override via BOO_REMOTE_USER.
remote_user="${BOO_REMOTE_USER:-$(whoami)}"

# --eval-store auto keeps flake evaluation on the local store (so flake paths and
# local inputs resolve correctly) while the actual derivation build runs on the
# remote store. Without it, `nix build --store ssh-ng://...` fails with
# "path '/nix/store/' is not in the Nix store" when the flake references
# local-store paths during evaluation.
exec nix build "${attr}" \
  --eval-store auto \
  --store "ssh-ng://${remote_user}@${host}" \
  --system "${system}" \
  "$@"
