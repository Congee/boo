#!/usr/bin/env python3
"""Generate deterministic runtime-view e2e terminal workloads.

The output is intentionally kept under ignored/generated directories by callers.
Given the same seed, byte target, profile, and pane count, generated files are
stable so baseline runs are reproducible without committing large artifacts.
"""

from __future__ import annotations

import argparse
import json
import random
from pathlib import Path

ANSI = [
    "\x1b[31m", "\x1b[32m", "\x1b[33m", "\x1b[34m", "\x1b[35m", "\x1b[36m", "\x1b[1m", "\x1b[0m"
]
EMOJI = ["🙂", "🚀", "🧪", "📦", "✅", "🔥", "🧭"]
CJK = ["界", "測試", "終端", "速度", "布局", "窗格"]
COMBINING = ["e\u0301", "a\u0308", "n\u0303", "o\u0302"]
WORDS = [
    "runtime", "pane", "focus", "render", "delta", "throughput", "latency",
    "scheduler", "terminal", "unicode", "snapshot", "statusbar", "tab", "resize",
]


def line_for(rng: random.Random, pane: int, line_no: int, profile: str) -> str:
    kind = rng.randrange(9)
    stamp = f"2026-04-24T12:{line_no % 60:02d}:{(line_no * 7) % 60:02d}.000Z"
    if kind == 0:
        return f"{stamp} INFO pane={pane} event=render_apply bytes={rng.randrange(64, 65536)} latency_ms={rng.random() * 18:.3f} emoji={rng.choice(EMOJI)} cjk={rng.choice(CJK)}"
    if kind == 1:
        return f"boo@runtime:{pane}$ cargo test -p boo-runtime --test pane_{pane}_{line_no} {rng.choice(EMOJI)}"
    if kind == 2:
        return f"   Compiling boo v0.1.0 (/work/boo) [{pane}:{line_no}] {rng.choice(ANSI)}colored-build-output\x1b[0m"
    if kind == 3:
        payload = {
            "pane": pane,
            "line": line_no,
            "level": rng.choice(["debug", "info", "warn"]),
            "message": " ".join(rng.choice(WORDS) for _ in range(6)),
            "unicode": rng.choice(CJK) + rng.choice(COMBINING) + rng.choice(EMOJI),
        }
        return json.dumps(payload, ensure_ascii=False, separators=(",", ":"))
    if kind == 4:
        return " | ".join([
            f"pane {pane:02d}", f"row {line_no:06d}", f"{rng.randrange(10**8):08d}",
            rng.choice(CJK), rng.choice(COMBINING), rng.choice(EMOJI),
        ])
    if kind == 5:
        base = "".join(rng.choice(WORDS) + "-" for _ in range(28))
        return f"wrap[{pane}:{line_no}] {base}{rng.choice(CJK)}{rng.choice(EMOJI)}"
    if kind == 6:
        return f"\x1b[38;5;{rng.randrange(16, 230)}mansi-color pane={pane} seq={line_no} {rng.choice(WORDS)} {rng.choice(EMOJI)}\x1b[0m"
    if kind == 7:
        return f"level=info pane={pane} line={line_no} component=runtime_view event=pane_update focused={rng.choice(['true','false'])} text={rng.choice(CJK)}{rng.choice(COMBINING)}{rng.choice(EMOJI)}"
    return f"{profile} plain pane={pane} line={line_no} " + " ".join(rng.choice(WORDS) for _ in range(12))


def generate_file(path: Path, seed: int, byte_target: int, profile: str, pane: int) -> dict:
    rng = random.Random((seed << 8) + pane)
    start_marker = f"RV_P{pane}_S{seed}_START"
    end_marker = f"RV_P{pane}_S{seed}_END"
    unicode_marker = "🙂 測試 e\u0301"
    written = 0
    line_no = 0
    with path.open("w", encoding="utf-8", newline="\n") as f:
        for line in (
            f"{start_marker} {rng.choice(EMOJI)} {rng.choice(CJK)} e\u0301",
            "# shell/log/json/table/ansi/wrap/cjk/combining/emoji workload",
        ):
            written += f.write(line + "\r\n")
        while written + len(end_marker.encode("utf-8")) + 8 < byte_target:
            line_no += 1
            line = line_for(rng, pane, line_no, profile)
            written += f.write(line + "\r\n")
        written += f.write(f"{end_marker} {unicode_marker}\r\n")
    return {
        "pane_index": pane,
        "path": str(path),
        "bytes": path.stat().st_size,
        "lines": line_no + 3,
        "start_marker": start_marker,
        "end_marker": end_marker,
        "unicode_marker": unicode_marker,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate Boo runtime-view e2e workload files")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--bytes", type=int, default=1_048_576, help="target bytes per pane")
    parser.add_argument("--output-dir", default="bench/generated/runtime-view-e2e")
    parser.add_argument("--profile", default="runtime-view-e2e")
    parser.add_argument("--panes", type=int, default=4, help="tab1 plus tab2 visible panes")
    args = parser.parse_args()

    out = Path(args.output_dir)
    out.mkdir(parents=True, exist_ok=True)
    files = []
    for pane in range(1, args.panes + 1):
        files.append(generate_file(out / f"pane-{pane}.txt", args.seed, args.bytes, args.profile, pane))
    manifest = {
        "profile": args.profile,
        "seed": args.seed,
        "bytes_per_pane_target": args.bytes,
        "files": files,
    }
    manifest_path = out / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps(manifest, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
