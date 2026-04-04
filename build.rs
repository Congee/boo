use std::env;
use std::fs;
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

        if let Some(vt_lib_dir) = ghostty_vt_lib_dir(&manifest_dir) {
            println!(
                "cargo:rustc-link-arg-bins=-Wl,-rpath,{}",
                vt_lib_dir.display()
            );
        }

        println!("cargo:rerun-if-changed=ghostty/macos/GhosttyKit.xcframework");
    } else if cfg!(target_os = "linux") {
        // Linux uses the published `libghostty-vt` crate, which owns fetching,
        // building, and linking the VT library. The vendored `ghostty`
        // submodule stays in the repo for the macOS surface backend.
    }

    println!("cargo:rerun-if-changed=build.rs");
}

fn ghostty_vt_lib_dir(manifest_dir: &std::path::Path) -> Option<PathBuf> {
    if let Ok(path) = env::var("DEP_GHOSTTY_VT_LIBDIR") {
        return Some(PathBuf::from(path));
    }

    let target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("target"));
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let build_dir = target_dir.join(profile).join("build");
    let entries = fs::read_dir(build_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path().join("out/ghostty-install/lib");
        if path.join("libghostty-vt.dylib").exists() {
            return Some(path);
        }
    }
    None
}
