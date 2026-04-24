{
  description = "boo — Rust/iced terminal app built on libghostty-vt";

  inputs.nixpkgs.url      = "github:nixos/nixpkgs/nixos-unstable";
  inputs.flake-utils.url  = "github:numtide/flake-utils";
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";
  inputs.zig.url          = "github:mitchellh/zig-overlay";
  inputs.ghostty.url      = "github:ghostty-org/ghostty/48ccec182a932c2ec04c344d45a5fc553861cb13";

  outputs = { self, nixpkgs, flake-utils, rust-overlay, zig, ghostty, ... }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs { inherit system overlays; };
      lib = pkgs.lib;

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
        cargo-deny
        zig.packages.${system}."0.15.2"
        git
      ]);
      # Darwin dependency split:
      #
      # * `apple-sdk` is needed for native package builds that compile/link
      #   against macOS frameworks. In this repo that means:
      #   - Ghostty's Nix `libghostty-vt-releasefast` package, which runs
      #     Ghostty's Zig build with `-Demit-lib-vt`
      #   - the Nix `booPackage`, whose Rust dependencies include macOS
      #     framework users such as iced/winit/wgpu and whose
      #     `libghostty-vt-sys` crate links against libghostty-vt
      #   - non-Nix `cargo build`, where `libghostty-vt-sys` falls back to a
      #     local `target/libghostty-vt/...` dylib via Zig
      # * Inside `nix develop`, prefer a single libghostty-vt authority: the
      #   dev shell exports LIBGHOSTTY_VT_SYS_* so Cargo uses the same
      #   `libghosttyVtPackage` that `nix build` uses instead of making a
      #   second target-local copy.
      # * The dev shell must still not expose Nix's SDKROOT/DEVELOPER_DIR to
      #   interactive Apple tools. Xcode's Swift compiler and the Nix SDK can
      #   be from different toolchain generations, so the shellHook below
      #   resets those variables after Nix setup hooks run.
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
      darwinDevShellEnvReset = lib.optionalString pkgs.stdenv.isDarwin ''
          # Keep interactive macOS/iOS tooling on the selected Xcode toolchain.
          # Nix's Darwin setup hooks export SDKROOT/DEVELOPER_DIR plus compiler
          # and linker flags for package builds; those leak badly into xcrun,
          # swift, xcodebuild, and GUI capture scripts. The Nix package builds
          # above set their own build env explicitly, so the dev shell should
          # stay neutral and let Apple tools resolve Xcode themselves.
          # Experimentally, pairing Xcode's Swift compiler with the Nix
          # apple-sdk Swift interfaces failed with `no such module SwiftShims`;
          # after this reset, xcrun resolves the selected Xcode SDK and the
          # same Swift validation compile succeeds.
          # Put Apple's shims ahead of Nix wrappers for interactive commands
          # and Cargo test binaries. Appending /usr/bin is not enough: Nix's
          # xcbuild `xcrun` can otherwise be found first and fail to resolve the
          # real Xcode SDK during link steps.
          export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/Applications/Xcode.app/Contents/Developer/usr/bin:$PATH"
          unset DEVELOPER_DIR DEVELOPER_DIR_FOR_BUILD DEVELOPER_DIR_FOR_TARGET
          unset SDKROOT SDKROOT_FOR_BUILD SDKROOT_FOR_TARGET
          unset MACOSX_DEPLOYMENT_TARGET IPHONEOS_DEPLOYMENT_TARGET
          unset CC CXX LD AR NM RANLIB LIBTOOL LDPLUSPLUS
          unset NIX_CFLAGS_COMPILE NIX_CFLAGS_COMPILE_FOR_BUILD
          unset NIX_CXXSTDLIB_COMPILE NIX_CXXSTDLIB_COMPILE_FOR_BUILD
          unset NIX_LDFLAGS NIX_LDFLAGS_FOR_BUILD
          unset OTHER_LDFLAGS OTHER_SWIFT_FLAGS
        '';
      libghosttyVtDevEnv = ''
          # Keep Cargo builds in the Nix dev shell on the same libghostty-vt
          # package as `nix build .#default`. Without these variables,
          # `vendor/libghostty-vt-sys/build.rs` uses its vendored Zig fallback
          # and creates a second target/libghostty-vt copy.
          export LIBGHOSTTY_VT_SYS_NO_VENDOR="1"
          export LIBGHOSTTY_VT_SYS_LIBDIR="${libghosttyVtPackage}/lib"
          export LIBGHOSTTY_VT_SYS_INCLUDEDIR="${libghosttyVtPackage.dev}/include"
        '';
      ghosttyLibghosttyVtPackage = ghostty.packages.${system}.libghostty-vt-releasefast;
      libghosttyVtPackage = ghosttyLibghosttyVtPackage.overrideAttrs (old: {
        # Upstream Ghostty now exports libghostty-vt on Darwin, but the package
        # still lacks the Darwin SDK/build environment needed to be a drop-in
        # Boo dependency. Keep this as a small packaging override until the fix
        # lands upstream; do not reintroduce a Boo-local Ghostty build recipe.
        buildInputs = (old.buildInputs or [ ])
          ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.apple-sdk_26 ];
        preBuild = (old.preBuild or "") + lib.optionalString pkgs.stdenv.isDarwin ''
          export PATH="$PATH:/usr/bin"
          export DEVELOPER_DIR="${pkgs.apple-sdk_26}"
          export SDKROOT="${pkgs.apple-sdk_26}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.0.sdk"
          export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-global-cache"
          export ZIG_LOCAL_CACHE_DIR="$TMPDIR/zig-local-cache"
          mkdir -p "$ZIG_GLOBAL_CACHE_DIR" "$ZIG_LOCAL_CACHE_DIR"
        '';
        postFixup = (old.postFixup or "") + lib.optionalString pkgs.stdenv.isDarwin ''
          for dylib in "$out"/lib/libghostty-vt.*.*.*.dylib; do
            [ -e "$dylib" ] || continue
            install_name_tool -id "$dylib" "$dylib"
          done
        '';
      });
      libghosttyVtSysEnv = {
        LIBGHOSTTY_VT_SYS_NO_VENDOR = "1";
        LIBGHOSTTY_VT_SYS_LIBDIR = "${libghosttyVtPackage}/lib";
        LIBGHOSTTY_VT_SYS_INCLUDEDIR = "${libghosttyVtPackage.dev}/include";
      };
      booPackage = rustPlatform.buildRustPackage ({
        pname = "boo";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = booNativeBuildInputs;
        buildInputs = commonBuildInputs ++ [ libghosttyVtPackage ];
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        preBuild = lib.optionalString pkgs.stdenv.isLinux ''
          export NIX_CFLAGS_COMPILE="$(echo "$NIX_CFLAGS_COMPILE" | sed 's/ *-fmacro-prefix-map=[^ ]*//g')"
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
      } // libghosttyVtSysEnv);
    in {
      packages.libghostty-vt = libghosttyVtPackage;
      packages.default = booPackage;

      apps.default = {
        type = "app";
        program = "${booPackage}/bin/boo";
        meta.description = "Run the Boo desktop app and CLI via `nix run`.";
      };

      checks.default = booPackage;

      devShells.default = mkShell {
        name = "boo-dev";
        nativeBuildInputs = devNativeBuildInputs ++ (with pkgs; [
          lldb
          gdb
        ]) ++ lib.optional pkgs.stdenv.isLinux pkgs.valgrind;

        buildInputs = commonBuildInputs ++ [ libghosttyVtPackage ];

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
        '' + darwinDevShellEnvReset + libghosttyVtDevEnv;
      };
    });
}
