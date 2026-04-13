#!/usr/bin/env python3
import argparse
import json
import re
from collections import defaultdict


LINE_RE = re.compile(
    r"boo_profile path=(?P<path>\S+) kind=(?P<kind>\S+) count=(?P<count>\d+) "
    r"total_ms=(?P<total_ms>[0-9.]+) avg_ms=(?P<avg_ms>[0-9.]+) max_ms=(?P<max_ms>[0-9.]+)"
    r"(?: bytes=(?P<bytes>\d+) bytes_per_sec=(?P<bytes_per_sec>[0-9.]+) "
    r"units=(?P<units>\d+) units_per_sec=(?P<units_per_sec>[0-9.]+))?"
)


def main() -> int:
    parser = argparse.ArgumentParser(description="Summarize boo_profile log output")
    parser.add_argument("log", help="Path to a log file containing boo_profile lines")
    parser.add_argument(
        "--paths",
        nargs="*",
        default=[],
        help="Optional path filters to print first, in addition to top entries",
    )
    parser.add_argument("--top", type=int, default=10, help="Number of top entries to include")
    args = parser.parse_args()

    stats = defaultdict(lambda: {"count": 0, "total_ms": 0.0, "max_ms": 0.0, "units": 0, "bytes": 0})
    with open(args.log, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            match = LINE_RE.search(line)
            if not match:
                continue
            key = (match.group("path"), match.group("kind"))
            item = stats[key]
            item["count"] += int(match.group("count"))
            item["total_ms"] += float(match.group("total_ms"))
            item["max_ms"] = max(item["max_ms"], float(match.group("max_ms")))
            if match.group("units"):
                item["units"] += int(match.group("units"))
            if match.group("bytes"):
                item["bytes"] += int(match.group("bytes"))

    rows = []
    for (path, kind), item in stats.items():
        avg_ms = item["total_ms"] / item["count"] if item["count"] else 0.0
        rows.append(
            {
                "path": path,
                "kind": kind,
                "count": item["count"],
                "total_ms": round(item["total_ms"], 3),
                "avg_ms": round(avg_ms, 3),
                "max_ms": round(item["max_ms"], 3),
                "units": item["units"],
                "bytes": item["bytes"],
            }
        )

    rows.sort(key=lambda row: (-row["total_ms"], -row["units"], row["path"], row["kind"]))

    selected = []
    seen = set()
    for wanted in args.paths:
        for row in rows:
            if row["path"] == wanted and (row["path"], row["kind"]) not in seen:
                selected.append(row)
                seen.add((row["path"], row["kind"]))
    for row in rows:
        if len(selected) >= max(args.top, len(args.paths)):
            break
        key = (row["path"], row["kind"])
        if key in seen:
            continue
        selected.append(row)
        seen.add(key)

    print(json.dumps({"log": args.log, "entries": selected}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
