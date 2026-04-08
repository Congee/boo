#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "usage: $0 <owner-name> <output.png> [window-name-substring]" >&2
  exit 2
fi

OWNER_NAME="$1"
OUTPUT_PATH="$2"
WINDOW_NAME_FILTER="${3:-}"

WINDOW_ID="$(
swift - "$OWNER_NAME" "$WINDOW_NAME_FILTER" <<'SWIFT'
import CoreGraphics
import Foundation

let ownerName = CommandLine.arguments[1]
let titleFilter = CommandLine.arguments[2]

let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let infoList = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
    exit(1)
}

struct Candidate {
    let id: Int
    let area: Double
}

var best: Candidate?

for info in infoList {
    guard let owner = info[kCGWindowOwnerName as String] as? String, owner == ownerName else {
        continue
    }
    let title = (info[kCGWindowName as String] as? String) ?? ""
    if !titleFilter.isEmpty && !title.localizedCaseInsensitiveContains(titleFilter) {
        continue
    }
    guard let layer = info[kCGWindowLayer as String] as? Int, layer == 0 else {
        continue
    }
    guard let bounds = info[kCGWindowBounds as String] as? [String: Any] else {
        continue
    }
    let width = (bounds["Width"] as? Double) ?? 0
    let height = (bounds["Height"] as? Double) ?? 0
    let area = width * height
    guard area > 0 else { continue }
    guard let id = info[kCGWindowNumber as String] as? Int else { continue }
    if best == nil || area > best!.area {
        best = Candidate(id: id, area: area)
    }
}

if let best {
    print(best.id)
} else {
    exit(1)
}
SWIFT
)"

screencapture -o -l "$WINDOW_ID" "$OUTPUT_PATH"
