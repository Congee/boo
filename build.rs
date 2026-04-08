use std::env;
use std::fs;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::os::unix::fs::symlink;

const MACOS_APP_BUNDLE_ID: &str = "me.congee.boo";
const MACOS_APP_NAME: &str = "boo";
const MACOS_APP_DISPLAY_NAME: &str = "boo";
const BOO_APP_DIR_NAME: &str = "boo.app";
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
        ensure_macos_app_bundle(&manifest_dir);
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

fn ensure_macos_app_bundle(manifest_dir: &std::path::Path) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let target_dir = env::var("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| manifest_dir.join("target"));
            let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
            target_dir.join(profile)
        });
    let bundle_dir = profile_dir.join(BOO_APP_DIR_NAME);
    let contents_dir = bundle_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");

    let legacy_bundle_root = out_dir.join("ghostty-install");
    if legacy_bundle_root.exists() {
        let _ = fs::remove_dir_all(&legacy_bundle_root);
    }

    fs::create_dir_all(&macos_dir)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", macos_dir.display()));
    fs::create_dir_all(&resources_dir)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", resources_dir.display()));

    let info_plist = contents_dir.join("Info.plist");
    fs::write(&info_plist, macos_info_plist())
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", info_plist.display()));

    let executable_link = macos_dir.join(BOO_EXECUTABLE_NAME);
    if executable_link.exists() {
        let _ = fs::remove_file(&executable_link);
    }
    let relative_binary = PathBuf::from("../../../").join(BOO_EXECUTABLE_NAME);
    symlink(&relative_binary, &executable_link).unwrap_or_else(|e| {
        panic!(
            "failed to create symlink {} -> {}: {e}",
            executable_link.display(),
            relative_binary.display()
        )
    });

    let pkg_info = contents_dir.join("PkgInfo");
    fs::write(&pkg_info, "APPL????")
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", pkg_info.display()));
}

fn macos_info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>{display_name}</string>
  <key>CFBundleExecutable</key>
  <string>{executable}</string>
  <key>CFBundleIdentifier</key>
  <string>{bundle_id}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>{name}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSAppleEventsUsageDescription</key>
  <string>A program running within boo would like to use AppleScript.</string>
  <key>NSBluetoothAlwaysUsageDescription</key>
  <string>A program running within boo would like to use Bluetooth.</string>
  <key>NSCalendarsUsageDescription</key>
  <string>A program running within boo would like to access your Calendar.</string>
  <key>NSCameraUsageDescription</key>
  <string>A program running within boo would like to use the camera.</string>
  <key>NSContactsUsageDescription</key>
  <string>A program running within boo would like to access your Contacts.</string>
  <key>NSLocalNetworkUsageDescription</key>
  <string>A program running within boo would like to access the local network.</string>
  <key>NSLocationUsageDescription</key>
  <string>A program running within boo would like to access your location information.</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>A program running within boo would like to use your microphone.</string>
  <key>NSMotionUsageDescription</key>
  <string>A program running within boo would like to access motion data.</string>
  <key>NSPhotoLibraryUsageDescription</key>
  <string>A program running within boo would like to access your Photo Library.</string>
  <key>NSRemindersUsageDescription</key>
  <string>A program running within boo would like to access your reminders.</string>
  <key>NSSpeechRecognitionUsageDescription</key>
  <string>A program running within boo would like to use speech recognition.</string>
  <key>NSSystemAdministrationUsageDescription</key>
  <string>A program running within boo requires elevated privileges.</string>
</dict>
</plist>
"#,
        bundle_id = MACOS_APP_BUNDLE_ID,
        executable = BOO_EXECUTABLE_NAME,
        name = MACOS_APP_NAME,
        display_name = MACOS_APP_DISPLAY_NAME,
    )
}
