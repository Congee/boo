use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ghostty_dir = manifest_dir.join("ghostty");

    assert!(
        ghostty_dir.join("build.zig").exists(),
        "ghostty submodule not initialized — run: git submodule update --init"
    );

    // Look for pre-built libghostty in the xcframework (built via ghostty's own devshell).
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
    let include_dir = ghostty_dir.join("include");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty");

    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=framework=Cocoa");
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=IOSurface");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=CoreText");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=c++");
    }

    println!("cargo:include={}", include_dir.display());
    println!("cargo:rerun-if-changed=ghostty/macos/GhosttyKit.xcframework");
    println!("cargo:rerun-if-changed=ghostty/include/ghostty.h");
}
