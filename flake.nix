{
  description = "boo — Rust/iced UI wrapper around libghostty";

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
        ];

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
          mesa  # libgbm + libEGL_mesa
          libdrm
        ];

        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

        shellHook = ''
          echo "boo dev shell"
          echo "  Zig: $(zig version)"
          echo "  Rust: $(rustc --version)"
        '' + pkgs.lib.optionalString pkgs.stdenv.isLinux ''
          export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath (with pkgs; [ wayland libxkbcommon libGL vulkan-loader ])}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
          export __EGL_VENDOR_LIBRARY_DIRS="/run/opengl-driver/share/glvnd/egl_vendor.d"
          export NIX_CFLAGS_COMPILE="$(echo "$NIX_CFLAGS_COMPILE" | sed 's/-fmacro-prefix-map=[^ ]*//g')"
          unset NIX_CFLAGS_COMPILE
          unset NIX_LDFLAGS 
        '' + pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
          unset SDKROOT
          unset DEVELOPER_DIR
          export PATH=$(echo "$PATH" | tr ':' '\n' | grep -v xcbuild | tr '\n' ':')
        '';
      };
    });
}
