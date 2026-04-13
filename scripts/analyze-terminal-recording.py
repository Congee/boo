#!/usr/bin/env python3
import argparse
import subprocess
from typing import List, Tuple


DEFAULT_BANDS: List[Tuple[str, str]] = [
    ("row0", "crop=940:22:40:62"),
    ("row1", "crop=940:22:40:86"),
    ("row2", "crop=940:22:40:110"),
]


def parse_fps(path: str) -> float:
    out = subprocess.check_output(
        [
            "ffprobe",
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=avg_frame_rate",
            "-of",
            "default=nw=1:nk=1",
            path,
        ],
        text=True,
    ).strip()
    num, den = out.split("/")
    return float(num) / float(den)


def frame_hashes(path: str, vf: str) -> List[str]:
    out = subprocess.check_output(
        ["ffmpeg", "-v", "error", "-i", path, "-vf", vf, "-an", "-f", "framemd5", "-"],
        text=True,
    )
    frames: List[str] = []
    for line in out.splitlines():
        if not line or line.startswith("#"):
            continue
        parts = [p.strip() for p in line.split(",")]
        if len(parts) >= 6:
            frames.append(parts[-1])
    return frames


def best_window(frames: List[str], fps: float, window_s: float):
    window_frames = max(1, int(round(window_s * fps)))
    best = None
    for start in range(0, max(1, len(frames) - window_frames + 1)):
        chunk = frames[start : start + window_frames]
        if len(chunk) < 2:
            continue
        changes = sum(1 for a, b in zip(chunk, chunk[1:]) if a != b)
        max_run = 1
        run = 1
        for a, b in zip(chunk, chunk[1:]):
            if a == b:
                run += 1
                max_run = max(max_run, run)
            else:
                run = 1
        score = (changes, -max_run)
        if best is None or score > best[0]:
            best = (score, start, changes, max_run)
    if best is None:
        return None
    _, start, changes, max_run = best
    return {
        "start_s": start / fps,
        "end_s": (start + window_frames) / fps,
        "changes_per_s": changes / window_s,
        "max_run": max_run,
        "max_s": max_run / fps,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Analyze Boo terminal recordings by row-band change cadence")
    parser.add_argument("videos", nargs="+", help="One or more mp4 recordings to analyze")
    parser.add_argument("--window", type=float, default=5.0, help="Active window length in seconds")
    args = parser.parse_args()

    for path in args.videos:
        fps = parse_fps(path)
        print(path)
        print(f"  fps={fps:.3f}")
        for band_name, vf in DEFAULT_BANDS:
            metrics = best_window(frame_hashes(path, vf), fps, args.window)
            if metrics is None:
                print(f"  {band_name}: no frames")
                continue
            print(
                f"  {band_name}: start={metrics['start_s']:.3f}s "
                f"end={metrics['end_s']:.3f}s "
                f"cps={metrics['changes_per_s']:.3f} "
                f"max_run={metrics['max_run']} "
                f"max_s={metrics['max_s']:.3f}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
