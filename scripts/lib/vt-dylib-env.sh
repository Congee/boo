#!/usr/bin/env bash

# Apply an explicit libghostty-vt directory to the final Boo process.
#
# `nix develop` exports BOO_VT_LIB_DIR for the Nix-built shared library. Keep
# this helper small and explicit: it must not scan target/libghostty-vt for a
# second Cargo-built copy because that reintroduces the split-library ambiguity
# the flake owns now.
boo_with_vt_lib_env() {
  local vt_lib_dir="${BOO_VT_LIB_DIR:-}"
  if [[ -z "$vt_lib_dir" ]]; then
    "$@"
    return
  fi

  export BOO_VT_LIB_DIR="$vt_lib_dir"

  case "$(uname -s)" in
    Darwin)
      DYLD_LIBRARY_PATH="$vt_lib_dir${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" "$@"
      ;;
    *)
      LD_LIBRARY_PATH="$vt_lib_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$@"
      ;;
  esac
}
