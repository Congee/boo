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
      lib = pkgs.lib;
      ghosttyCommit = "bebca84668947bfc92b9a30ed58712e1c34eee1d";

      toolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = [ "rust-src" "rust-std" "clippy" "rustfmt" "rust-analyzer" ];
      };
      rustPlatform = pkgs.makeRustPlatform {
        cargo = toolchain;
        rustc = toolchain;
      };
      llvm = (if pkgs.stdenv.isLinux then pkgs.pkgsLLVM else pkgs).llvmPackages_latest;
      mkShell = if pkgs.stdenv.isLinux
        then llvm.stdenv.mkDerivation
        else pkgs.mkShellNoCC;
      booNativeBuildInputs = with pkgs; [
        toolchain
        pkg-config
        clang-tools
        fontconfig
      ];
      devNativeBuildInputs = booNativeBuildInputs ++ (with pkgs; [
        zig.packages.${system}."0.15.2"
        git
      ]);
      commonBuildInputs = with pkgs; [
        openssl
      ]
      ++ lib.optional pkgs.stdenv.isDarwin apple-sdk
      ++ lib.optional pkgs.stdenv.isDarwin libiconv
      ++ lib.optionals pkgs.stdenv.isLinux [
        libGL
        libxkbcommon
        wayland
        vulkan-loader
        gtk4
        glib
        fontconfig
        freetype
      ];
      ghosttySrc = pkgs.fetchFromGitHub {
        owner = "ghostty-org";
        repo = "ghostty";
        rev = ghosttyCommit;
        hash = "sha256-7MPEjIAQD+Z/zdP4h/yslysuVnhCESOPvdvwoLoPVmI=";
      };
      ghosttyBuildInputs = import "${ghosttySrc}/nix/build-support/build-inputs.nix" {
        inherit pkgs lib;
        stdenv = pkgs.stdenv;
        enableX11 = pkgs.stdenv.isLinux;
        enableWayland = pkgs.stdenv.isLinux;
      };
      libghosttyVtPackage = pkgs.stdenv.mkDerivation (finalAttrs: {
        pname = "libghostty-vt";
        version = "0.1.1-ghostty-${builtins.substring 0 12 ghosttyCommit}";
        src = ghosttySrc;
        deps = pkgs.callPackage "${ghosttySrc}/build.zig.zon.nix" {
          name = "ghostty-cache-libghostty-vt-${builtins.substring 0 12 ghosttyCommit}";
        };
        nativeBuildInputs = with pkgs; [
          ncurses
          zig_0_15
          pkg-config
        ] ++ lib.optionals pkgs.stdenv.isLinux [
          wayland-scanner
          wayland-protocols
        ];
        buildInputs = ghosttyBuildInputs ++ commonBuildInputs;
        dontConfigure = true;
        doCheck = false;
        dontSetZigDefaultFlags = true;
        zigBuildFlags = [
          "--system"
          "${finalAttrs.deps}"
          "-Demit-lib-vt"
          "-Dcpu=baseline"
          "-Doptimize=ReleaseFast"
        ];
        preBuild = lib.optionalString pkgs.stdenv.isLinux ''
          export NIX_CFLAGS_COMPILE="$(echo "$NIX_CFLAGS_COMPILE" | sed 's/ *-fmacro-prefix-map=[^ ]*//g')"
          export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-global-cache"
          export ZIG_LOCAL_CACHE_DIR="$TMPDIR/zig-local-cache"
          mkdir -p "$ZIG_GLOBAL_CACHE_DIR" "$ZIG_LOCAL_CACHE_DIR"
        '' + lib.optionalString pkgs.stdenv.isDarwin ''
          export PATH="$PATH:/usr/bin"
          export DEVELOPER_DIR="$(xcode-select -p)"
          export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
          export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-global-cache"
          export ZIG_LOCAL_CACHE_DIR="$TMPDIR/zig-local-cache"
          mkdir -p "$ZIG_GLOBAL_CACHE_DIR" "$ZIG_LOCAL_CACHE_DIR"
        '';
        postInstall = lib.optionalString pkgs.stdenv.isLinux ''
          if [ -f "$out/lib/libghostty-vt.so.0.1.0" ]; then
            ln -sf libghostty-vt.so.0.1.0 "$out/lib/libghostty-vt.so"
            ln -sf libghostty-vt.so.0.1.0 "$out/lib/libghostty-vt.so.0"
          fi
        '' + lib.optionalString pkgs.stdenv.isDarwin ''
          if [ -f "$out/lib/libghostty-vt.0.1.0.dylib" ]; then
            ln -sf libghostty-vt.0.1.0.dylib "$out/lib/libghostty-vt.dylib"
            # The dylib ships with install_name = @rpath/libghostty-vt.dylib. Any
            # binary that links against it then needs an LC_RPATH entry pointing
            # at this store path, which rustc does not emit by default. Rewriting
            # install_name to the absolute store path makes dependents (boo
            # itself) resolve the library at runtime without any rpath dance.
            install_name_tool -id "$out/lib/libghostty-vt.0.1.0.dylib" \
              "$out/lib/libghostty-vt.0.1.0.dylib"
          fi
        '' + ''
          rm -rf "$out/share" "$out/Ghostty.app" "$out/boo.app"
        '';
        meta = with lib; {
          description = "Ghostty terminal emulation library";
          platforms = platforms.unix;
        };
      });
      booPackage = rustPlatform.buildRustPackage {
        pname = "boo";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = booNativeBuildInputs;
        buildInputs = commonBuildInputs ++ [ libghosttyVtPackage ];
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        preBuild = lib.optionalString pkgs.stdenv.isLinux ''
          export NIX_CFLAGS_COMPILE="$(echo "$NIX_CFLAGS_COMPILE" | sed 's/ *-fmacro-prefix-map=[^ ]*//g')"
          export LIBGHOSTTY_VT_SYS_LIBDIR="${libghosttyVtPackage}/lib"
          export LIBGHOSTTY_VT_SYS_INCLUDEDIR="${libghosttyVtPackage}/include"
        '' + lib.optionalString pkgs.stdenv.isDarwin ''
          export PATH="$PATH:/usr/bin"
          export DEVELOPER_DIR="$(xcode-select -p)"
          export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
          export LIBGHOSTTY_VT_SYS_LIBDIR="${libghosttyVtPackage}/lib"
          export LIBGHOSTTY_VT_SYS_INCLUDEDIR="${libghosttyVtPackage}/include"
        '';
        # The test suite is local-only and sandbox-safe. The important Darwin
        # requirement is making the store-provided libghostty-vt dylib visible
        # to cargoCheckHook's test binaries inside the Nix sandbox.
        preCheck = lib.optionalString pkgs.stdenv.isDarwin ''
          export DYLD_LIBRARY_PATH="${libghosttyVtPackage}/lib''${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
        '' + lib.optionalString pkgs.stdenv.isLinux ''
          export LD_LIBRARY_PATH="${lib.makeLibraryPath [ libghosttyVtPackage ]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        '';
        meta = with lib; {
          description = "Rust/iced terminal app built on libghostty-vt";
          mainProgram = "boo";
          platforms = platforms.unix;
        };
      };
    in {
      packages.libghostty-vt = libghosttyVtPackage;
      packages.default = booPackage;

      apps.default = flake-utils.lib.mkApp {
        drv = booPackage;
      };

      checks.default = booPackage;

      devShells.default = mkShell {
        name = "boo-dev";
        nativeBuildInputs = devNativeBuildInputs ++ (with pkgs; [
          lldb
          gdb
        ]) ++ lib.optional pkgs.stdenv.isLinux pkgs.valgrind;

        buildInputs = commonBuildInputs;

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
          export PATH="$PATH:/usr/bin"
          export DEVELOPER_DIR="$(xcode-select -p)"
          export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
        '';
      };
    });
}
