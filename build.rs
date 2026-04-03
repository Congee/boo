use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ghostty_dir = manifest_dir.join("ghostty");

    if cfg!(target_os = "macos") {
        assert!(
            ghostty_dir.join("build.zig").exists(),
            "ghostty submodule not initialized — run: git submodule update --init"
        );

        let xcframework_lib = ghostty_dir
            .join("macos/GhosttyKit.xcframework/macos-arm64_x86_64/libghostty.a");

        assert!(
            xcframework_lib.exists(),
            "libghostty not found at {}\n\
             Build it from the ghostty submodule's devshell:\n\
             \n\
             cd ghostty && zig build -Doptimize=ReleaseFast -Demit-xcframework=true\n",
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
        // Linux uses the published `libghostty-vt` crate, which owns fetching,
        // building, and linking the VT library. The vendored `ghostty`
        // submodule stays in the repo for the macOS surface backend.
    }

    println!("cargo:rerun-if-changed=build.rs");
}
