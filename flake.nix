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
    in {
      devShells.default = pkgs.mkShellNoCC {
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
        ];

        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

        shellHook = ''
          echo "boo dev shell"
          echo "  Zig: $(zig version)"
          echo "  Rust: $(rustc --version)"
        '' + pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
          unset SDKROOT
          unset DEVELOPER_DIR
          export PATH=$(echo "$PATH" | tr ':' '\n' | grep -v xcbuild | tr '\n' ':')
        '';
      };
    });
}
