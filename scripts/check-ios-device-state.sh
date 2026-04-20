#!/usr/bin/env bash
set -euo pipefail

DEVICE_ID="${BOO_IOS_DEVICE_ID:-${1:-}}"

if [[ -z "$DEVICE_ID" ]]; then
  echo "usage: BOO_IOS_DEVICE_ID=<device-id> $0 [device-id]" >&2
  echo "tip: use scripts/list-ios-devices.sh to find a device identifier" >&2
  exit 2
fi

DETAILS_JSON="$(mktemp -t boo-ios-device-details.XXXXXX.json)"
LOCK_JSON="$(mktemp -t boo-ios-device-lock.XXXXXX.json)"
cleanup() {
  rm -f "$DETAILS_JSON" "$LOCK_JSON"
}
trap cleanup EXIT

xcrun devicectl device info details \
  --device "$DEVICE_ID" \
  --json-output "$DETAILS_JSON" >/dev/null

xcrun devicectl device info lockState \
  --device "$DEVICE_ID" \
  --json-output "$LOCK_JSON" >/dev/null

python3 - "$DETAILS_JSON" "$LOCK_JSON" <<'PY'
import json
import sys

details_path, lock_path = sys.argv[1:3]

with open(details_path, "r", encoding="utf-8") as fh:
    details = json.load(fh)
with open(lock_path, "r", encoding="utf-8") as fh:
    lock = json.load(fh)

device = details["result"]["deviceProperties"]
hardware = details["result"]["hardwareProperties"]
lock_state = lock["result"]

name = device["name"]
developer_mode = device["developerModeStatus"]
passcode_required = lock_state["passcodeRequired"]
unlocked_since_boot = lock_state["unlockedSinceBoot"]

print(f"device: {name}")
print(f"udid: {hardware['udid']}")
print(f"os: {device['osVersionNumber']} ({device['osBuildUpdate']})")
print(f"developer mode: {developer_mode}")
print(f"passcode required now: {'yes' if passcode_required else 'no'}")
print(f"unlocked since boot: {'yes' if unlocked_since_boot else 'no'}")

problems = []
if developer_mode != "enabled":
    problems.append(
        "Developer Mode is disabled. Finish the enable, reboot, and post-reboot confirmation flow on the device."
    )
if passcode_required:
    problems.append("The device is currently locked. Unlock it before install or launch.")

if problems:
    print("")
    print("device is not ready:")
    for problem in problems:
        print(f"- {problem}")
    sys.exit(1)

print("")
print("device is ready for install and launch")
PY
