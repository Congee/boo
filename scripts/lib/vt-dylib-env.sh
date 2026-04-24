#!/usr/bin/env bash

boo_find_vt_lib_dir() {
  local root="${BOO_REPO_ROOT:-$(pwd)}"
  local target="${TARGET:-$(rustc -vV | awk '/host:/ {print $2}')}"
  local candidates=(
    "$root/target/libghostty-vt/$target/debug/lib"
    "$root/target/libghostty-vt/$target/profiling/lib"
    "$root/target/libghostty-vt/$target/release/lib"
  )
  local path
  for path in "${candidates[@]}"; do
    if [[ -e "$path/libghostty-vt.dylib" || -e "$path/libghostty-vt.so.0" || -e "$path/libghostty-vt.so" ]]; then
      printf '%s\n' "$path"
      return 0
    fi
  done
  return 1
}

boo_with_vt_lib_env() {
  local vt_lib_dir="${BOO_VT_LIB_DIR:-}"
  if [[ -z "$vt_lib_dir" ]]; then
    vt_lib_dir="$(boo_find_vt_lib_dir || true)"
  fi

  if [[ -z "$vt_lib_dir" ]]; then
    "$@"
    return
  fi

  case "$(uname -s)" in
    Darwin)
      DYLD_LIBRARY_PATH="$vt_lib_dir${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" "$@"
      ;;
    *)
      LD_LIBRARY_PATH="$vt_lib_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$@"
      ;;
  esac
}
