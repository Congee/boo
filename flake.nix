{
  description = "boo — Rust/iced terminal app built on libghostty-vt";

  inputs.nixpkgs.url      = "github:nixos/nixpkgs/nixos-unstable";
  inputs.flake-utils.url  = "github:numtide/flake-utils";
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";
  inputs.zig.url          = "github:mitchellh/zig-overlay";

  outputs = { self, nixpkgs, flake-utils, rust-overlay, zig, ... }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs { inherit system overlays; };

      toolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = [ "rust-src" "rust-std" "clippy" "rustfmt" "rust-analyzer" ];
      };
      llvm = (if pkgs.stdenv.isLinux then pkgs.pkgsLLVM else pkgs).llvmPackages_latest;
      mkShell = if pkgs.stdenv.isLinux
        then llvm.stdenv.mkDerivation
        else pkgs.mkShellNoCC;
    in {
      devShells.default = mkShell {
        name = "boo-dev";
        nativeBuildInputs = with pkgs; [
          toolchain
          zig.packages.${system}."0.15.2"
          pkg-config
          clang-tools
          lldb
          gdb
        ] ++ lib.optional stdenv.isLinux valgrind;

        buildInputs = with pkgs; [
          openssl
        ]
        ++ lib.optional stdenv.isDarwin apple-sdk
        ++ lib.optional stdenv.isDarwin libiconv
        ++ lib.optionals stdenv.isLinux [
          libGL
          libxkbcommon
          wayland
          vulkan-loader
          gtk4
          glib
          fontconfig
          freetype
        ];

        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

        shellHook = ''
          echo "boo dev shell"
          echo "  Zig: $(zig version)"
          echo "  Rust: $(rustc --version)"
        '' + pkgs.lib.optionalString pkgs.stdenv.isLinux ''
          # Use system OpenGL/EGL stack via /run/opengl-driver (NixOS hardware.graphics)
          # and add nix-store libs for Wayland/XKB. System mesa needs its own deps
          # on LD_LIBRARY_PATH because dlopen doesn't follow RUNPATH transitively.
          SYSTEM_GL="$(dirname $(readlink -f /run/opengl-driver/lib/libEGL_mesa.so.0 2>/dev/null) 2>/dev/null)"
          export LD_LIBRARY_PATH="''${SYSTEM_GL:+$SYSTEM_GL:}${pkgs.lib.makeLibraryPath (with pkgs; [ wayland libxkbcommon libGL vulkan-loader ])}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
          export __EGL_VENDOR_LIBRARY_DIRS="/run/opengl-driver/share/glvnd/egl_vendor.d"
          # Strip -fmacro-prefix-map flags that zig doesn't understand
          export NIX_CFLAGS_COMPILE="$(echo "$NIX_CFLAGS_COMPILE" | sed 's/ *-fmacro-prefix-map=[^ ]*//g')"
        '' + pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
          unset SDKROOT
          unset DEVELOPER_DIR
          export PATH=$(echo "$PATH" | tr ':' '\n' | grep -v xcbuild | tr '\n' ':')
        '';
      };
    });
}
