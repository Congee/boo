use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned ghostty commit. Update this to pull a newer version.
const GHOSTTY_REPO: &str = "https://github.com/ghostty-org/ghostty.git";
const GHOSTTY_COMMIT: &str = "bebca84668947bfc92b9a30ed58712e1c34eee1d";

fn main() {
    // docs.rs has no Zig toolchain. The checked-in bindings in src/bindings.rs
    // are enough for generating documentation, so skip the entire native
    // build when running under docs.rs.
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_SYS_NO_VENDOR");
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_SYS_OPTIMIZE");
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_SYS_LIBDIR");
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_SYS_INCLUDEDIR");
    println!("cargo:rerun-if-env-changed=GHOSTTY_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-env-changed=HOST");
    println!("cargo:rerun-if-changed=crates/libghostty-vt-sys/build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let target = env::var("TARGET").expect("TARGET must be set");
    let host = env::var("HOST").expect("HOST must be set");

    let (lib_dir, include_dir) = if let Some(paths) = externally_provided_paths() {
        paths
    } else {
        ensure_cached_install_prefix(&out_dir, &target, &host)
    };

    let lib_name = if target.contains("darwin") {
        "libghostty-vt.0.1.0.dylib"
    } else {
        "libghostty-vt.so.0.1.0"
    };

    assert!(
        lib_dir.join(lib_name).exists(),
        "expected shared library at {}",
        lib_dir.join(lib_name).display()
    );
    assert!(
        include_dir.join("ghostty").join("vt.h").exists(),
        "expected header at {}",
        include_dir.join("ghostty").join("vt.h").display()
    );

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=ghostty-vt");
    println!("cargo:include={}", include_dir.display());
    println!("cargo:libdir={}", lib_dir.display());
}

fn externally_provided_paths() -> Option<(PathBuf, PathBuf)> {
    let lib_dir = env::var("LIBGHOSTTY_VT_SYS_LIBDIR").ok().map(PathBuf::from)?;
    let include_dir = env::var("LIBGHOSTTY_VT_SYS_INCLUDEDIR")
        .ok()
        .map(PathBuf::from)?;
    Some((lib_dir, include_dir))
}

fn ensure_cached_install_prefix(out_dir: &Path, target: &str, host: &str) -> (PathBuf, PathBuf) {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace root");
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let optimize = zig_optimize_mode();
    let install_prefix = workspace_root
        .join("target")
        .join("libghostty-vt")
        .join(target)
        .join(profile);
    let stamp_path = install_prefix.join(".build-stamp");

    let expected_stamp = format!("{GHOSTTY_COMMIT}\n{optimize}\n{target}\n");
    let lib_dir = install_prefix.join("lib");
    let include_dir = install_prefix.join("include");
    let lib_name = if target.contains("darwin") {
        "libghostty-vt.0.1.0.dylib"
    } else {
        "libghostty-vt.so.0.1.0"
    };
    let ready = lib_dir.join(lib_name).exists()
        && include_dir.join("ghostty").join("vt.h").exists()
        && fs::read_to_string(&stamp_path).ok().as_deref() == Some(expected_stamp.as_str());
    if !ready {
        if install_prefix.exists() {
            let _ = fs::remove_dir_all(&install_prefix);
        }
        fs::create_dir_all(&install_prefix)
            .unwrap_or_else(|e| panic!("failed to create {}: {e}", install_prefix.display()));

        let ghostty_dir = match env::var("GHOSTTY_SOURCE_DIR") {
            Ok(dir) => {
                let p = PathBuf::from(dir);
                assert!(
                    p.join("build.zig").exists(),
                    "GHOSTTY_SOURCE_DIR does not contain build.zig: {}",
                    p.display()
                );
                p
            }
            Err(_) => find_cached_ghostty_source(out_dir).unwrap_or_else(|| fetch_ghostty(out_dir)),
        };

        let mut build = Command::new("zig");
        build
            .arg("build")
            .arg("-Demit-lib-vt")
            .arg(format!("-Doptimize={optimize}"))
            .arg("--prefix")
            .arg(&install_prefix)
            .current_dir(&ghostty_dir);

        if target != host {
            let zig_target = zig_target(target);
            build.arg(format!("-Dtarget={zig_target}"));
        }

        run(build, "zig build");
        let _ = fs::remove_dir_all(install_prefix.join("Ghostty.app"));
        fs::write(&stamp_path, expected_stamp)
            .unwrap_or_else(|e| panic!("failed to write {}: {e}", stamp_path.display()));
    }

    (lib_dir, include_dir)
}

fn zig_optimize_mode() -> &'static str {
    if let Ok(explicit) = env::var("LIBGHOSTTY_VT_SYS_OPTIMIZE") {
        return match explicit.as_str() {
            "Debug" | "ReleaseSafe" | "ReleaseSmall" | "ReleaseFast" => {
                Box::leak(explicit.into_boxed_str())
            }
            other => panic!("unsupported LIBGHOSTTY_VT_SYS_OPTIMIZE value: {other}"),
        };
    }

    match env::var("PROFILE").as_deref() {
        Ok("release") => "ReleaseFast",
        Ok("bench") => "ReleaseFast",
        _ => "ReleaseFast",
    }
}

/// Clone ghostty at the pinned commit into OUT_DIR/ghostty-src.
/// Reuses an existing clone if the commit matches.
fn fetch_ghostty(out_dir: &Path) -> PathBuf {
    let src_dir = out_dir.join("ghostty-src");
    let stamp = src_dir.join(".ghostty-commit");

    // Skip fetch if we already have the right commit.
    if stamp.exists()
        && let Ok(existing) = std::fs::read_to_string(&stamp)
            && existing.trim() == GHOSTTY_COMMIT {
                return src_dir;
            }

    // Clean and clone fresh.
    if src_dir.exists() {
        std::fs::remove_dir_all(&src_dir)
            .unwrap_or_else(|e| panic!("failed to remove {}: {e}", src_dir.display()));
    }

    eprintln!("Fetching ghostty {GHOSTTY_COMMIT} ...");

    let mut clone = Command::new("git");
    clone
        .arg("clone")
        .arg("--filter=blob:none")
        .arg("--no-checkout")
        .arg(GHOSTTY_REPO)
        .arg(&src_dir);
    run(clone, "git clone ghostty");

    let mut checkout = Command::new("git");
    checkout
        .arg("checkout")
        .arg(GHOSTTY_COMMIT)
        .current_dir(&src_dir);
    run(checkout, "git checkout ghostty commit");

    std::fs::write(&stamp, GHOSTTY_COMMIT).unwrap_or_else(|e| panic!("failed to write stamp: {e}"));

    src_dir
}

fn find_cached_ghostty_source(out_dir: &Path) -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    let workspace_root = manifest_dir.parent()?.parent()?;
    let target_build_dir = workspace_root.join("target").join("debug").join("build");
    let entries = std::fs::read_dir(target_build_dir).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join("out").join("ghostty-src");
        if candidate.join("build.zig").exists() {
            return Some(candidate);
        }
    }
    let fallback = out_dir.join("ghostty-src");
    fallback.join("build.zig").exists().then_some(fallback)
}

fn run(mut command: Command, context: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("failed to execute {context}: {error}"));
    assert!(status.success(), "{context} failed with status {status}");
}

fn zig_target(target: &str) -> String {
    let value = match target {
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        "x86_64-unknown-linux-musl" => "x86_64-linux-musl",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        "aarch64-unknown-linux-musl" => "aarch64-linux-musl",
        "aarch64-apple-darwin" => "aarch64-macos-none",
        "x86_64-apple-darwin" => "x86_64-macos-none",
        other => panic!("unsupported Rust target for vendored build: {other}"),
    };
    value.to_owned()
}
