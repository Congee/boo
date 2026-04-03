use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ghostty_dir = manifest_dir.join("ghostty");

    assert!(
        ghostty_dir.join("build.zig").exists(),
        "ghostty submodule not initialized — run: git submodule update --init"
    );

    let include_dir = ghostty_dir.join("include");

    if cfg!(target_os = "macos") {
        let xcframework_lib = ghostty_dir
            .join("macos/GhosttyKit.xcframework/macos-arm64_x86_64/libghostty.a");

        assert!(
            xcframework_lib.exists(),
            "libghostty not found at {}\n\
             Build it from the ghostty submodule's devshell:\n\
             \n\
             cd ghostty && zig build -Doptimize=Debug -Demit-xcframework=true\n",
            xcframework_lib.display()
        );

        let lib_dir = xcframework_lib.parent().unwrap();
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-lib=static=ghostty");

        println!("cargo:rustc-link-lib=framework=Cocoa");
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=IOSurface");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=CoreText");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=c++");

        println!("cargo:rerun-if-changed=ghostty/macos/GhosttyKit.xcframework");
    } else if cfg!(target_os = "linux") {
        // Build with: cd ghostty && zig build -Doptimize=Debug -Dapp-runtime=none
        // The .so bundles all vendored deps (oniguruma, harfbuzz, spirv-cross, etc.)
        let lib_dir = ghostty_dir.join("zig-out/lib");
        let zig_out_so = lib_dir.join("libghostty.so");

        if zig_out_so.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            println!("cargo:rustc-link-lib=dylib=ghostty");
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
            println!("cargo:rustc-link-arg=-Wl,--allow-shlib-undefined");
            // libghostty.so uses eglMakeCurrent — link EGL so the runtime linker can resolve it
            println!("cargo:rustc-link-lib=dylib=EGL");
        } else {
            println!(
                "cargo:warning=libghostty not found at {}. \
                 Build it: cd ghostty && zig build -Doptimize=Debug -Dapp-runtime=none",
                zig_out_so.display()
            );
        }

        println!("cargo:rerun-if-changed=ghostty/zig-out/lib/libghostty.so");
    }

    println!("cargo:include={}", include_dir.display());
    println!("cargo:rerun-if-changed=ghostty/include/ghostty.h");
}
