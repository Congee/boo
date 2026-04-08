use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const MACOS_APP_BUNDLE_ID: &str = "me.congee.boo";
const MACOS_APP_NAME: &str = "boo";
const MACOS_APP_DISPLAY_NAME: &str = "boo";
const GHOSTTY_APP_DIR_NAME: &str = "Ghostty.app";
const BOO_APP_DIR_NAME: &str = "boo.app";
const GHOSTTY_EXECUTABLE_NAME: &str = "ghostty";
const BOO_EXECUTABLE_NAME: &str = "boo";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    if cfg!(target_os = "macos") {
        if let Some(vt_lib_dir) = libghostty_vt_lib_dir(&manifest_dir) {
            println!(
                "cargo:rustc-link-arg-bins=-Wl,-rpath,{}",
                vt_lib_dir.display()
            );
        }
        migrate_macos_app_bundle(&manifest_dir);
    }

    println!("cargo:rerun-if-changed=build.rs");
}

fn libghostty_vt_lib_dir(manifest_dir: &std::path::Path) -> Option<PathBuf> {
    if let Ok(path) = env::var("DEP_GHOSTTY_VT_LIBDIR") {
        return Some(PathBuf::from(path));
    }

    let target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("target"));
    let target = env::var("TARGET").ok()?;
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let lib_dir = target_dir
        .join("libghostty-vt")
        .join(target)
        .join(profile)
        .join("lib");
    if lib_dir.join("libghostty-vt.dylib").exists() {
        return Some(lib_dir);
    }
    None
}

fn migrate_macos_app_bundle(manifest_dir: &std::path::Path) {
    for bundle_dir in macos_app_bundle_dirs(manifest_dir) {
        let final_bundle_dir = rename_bundle_dir_if_needed(&bundle_dir).unwrap_or(bundle_dir);
        let contents_dir = final_bundle_dir.join("Contents");
        let info_plist = contents_dir.join("Info.plist");
        if !info_plist.exists() {
            continue;
        }
        rename_executable_if_needed(&contents_dir);
        patch_info_plist(&info_plist);
    }
}

fn macos_app_bundle_dirs(manifest_dir: &std::path::Path) -> Vec<PathBuf> {
    let target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("target"));
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let build_dir = target_dir.join(profile).join("build");
    let mut bundles = Vec::new();
    let Ok(entries) = fs::read_dir(build_dir) else {
        return bundles;
    };
    for entry in entries.flatten() {
        let out_dir = entry.path().join("out/ghostty-install");
        let boo_bundle = out_dir.join(BOO_APP_DIR_NAME);
        if boo_bundle.exists() {
            bundles.push(boo_bundle);
            continue;
        }
        let ghostty_bundle = out_dir.join(GHOSTTY_APP_DIR_NAME);
        if ghostty_bundle.exists() {
            bundles.push(ghostty_bundle);
        }
    }
    bundles
}

fn rename_bundle_dir_if_needed(bundle_dir: &std::path::Path) -> Option<PathBuf> {
    if bundle_dir.file_name().and_then(|name| name.to_str()) != Some(GHOSTTY_APP_DIR_NAME) {
        return None;
    }
    let target = bundle_dir.with_file_name(BOO_APP_DIR_NAME);
    if target.exists() {
        return Some(target);
    }
    if fs::rename(bundle_dir, &target).is_ok() {
        Some(target)
    } else {
        None
    }
}

fn rename_executable_if_needed(contents_dir: &std::path::Path) {
    let macos_dir = contents_dir.join("MacOS");
    let ghostty = macos_dir.join(GHOSTTY_EXECUTABLE_NAME);
    let boo = macos_dir.join(BOO_EXECUTABLE_NAME);
    if ghostty.exists() && !boo.exists() {
        let _ = fs::rename(ghostty, boo);
    }
}

fn patch_info_plist(info_plist: &std::path::Path) {
    let replacements = [
        ("CFBundleIdentifier", MACOS_APP_BUNDLE_ID),
        ("CFBundleExecutable", BOO_EXECUTABLE_NAME),
        ("CFBundleName", MACOS_APP_NAME),
        ("CFBundleDisplayName", MACOS_APP_DISPLAY_NAME),
        ("NSAppleEventsUsageDescription", "A program running within boo would like to use AppleScript."),
        ("NSBluetoothAlwaysUsageDescription", "A program running within boo would like to use Bluetooth."),
        ("NSCalendarsUsageDescription", "A program running within boo would like to access your Calendar."),
        ("NSCameraUsageDescription", "A program running within boo would like to use the camera."),
        ("NSContactsUsageDescription", "A program running within boo would like to access your Contacts."),
        ("NSLocalNetworkUsageDescription", "A program running within boo would like to access the local network."),
        ("NSLocationUsageDescription", "A program running within boo would like to access your location information."),
        ("NSMicrophoneUsageDescription", "A program running within boo would like to use your microphone."),
        ("NSMotionUsageDescription", "A program running within boo would like to access motion data."),
        ("NSPhotoLibraryUsageDescription", "A program running within boo would like to access your Photo Library."),
        ("NSRemindersUsageDescription", "A program running within boo would like to access your reminders."),
        ("NSSpeechRecognitionUsageDescription", "A program running within boo would like to use speech recognition."),
        ("NSSystemAdministrationUsageDescription", "A program running within boo requires elevated privileges."),
    ];
    for (key, value) in replacements {
        let _ = set_plist_string(info_plist, key, value);
    }
    let _ = set_plist_string(info_plist, "NSServices:0:NSMenuItem:default", "New boo Tab Here");
    let _ = set_plist_string(info_plist, "NSServices:1:NSMenuItem:default", "New boo Window Here");
    let _ = set_plist_string(
        info_plist,
        "UTExportedTypeDeclarations:0:UTTypeIdentifier",
        "me.congee.booSurfaceId",
    );
}

fn set_plist_string(info_plist: &std::path::Path, key_path: &str, value: &str) -> std::io::Result<()> {
    let status = Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(format!("Set :{key_path} {value}"))
        .arg(info_plist)
        .status()?;
    if status.success() {
        return Ok(());
    }
    Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(format!("Add :{key_path} string {value}"))
        .arg(info_plist)
        .status()?;
    Ok(())
}
