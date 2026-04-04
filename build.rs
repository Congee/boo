use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    if cfg!(target_os = "macos") {
        if let Some(vt_lib_dir) = ghostty_vt_lib_dir(&manifest_dir) {
            println!(
                "cargo:rustc-link-arg-bins=-Wl,-rpath,{}",
                vt_lib_dir.display()
            );
        }
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
