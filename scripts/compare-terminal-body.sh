#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

OUT_DIR="${OUT_DIR:-/tmp/boo-terminal-body-bench}"
DURATION="${DURATION:-5}"
READY_TIMEOUT="${READY_TIMEOUT:-20}"
PROFILE_TARGET="${PROFILE_TARGET:-client}"
PROFILER="${PROFILER:-none}"
IMPLS=(canvas_text model_paragraph)
SUMMARY_PATHS=(
  client.text_layer.diff
  client.text_layer.draw
  client.canvas.draw
  client.view.render_terminal_scene
  client.stream.apply_delta
)

if [[ $# -gt 0 ]]; then
  SCENARIOS=("$@")
else
  SCENARIOS=(plain-cat unicode-cat wrap-cat pager-less)
fi

mkdir -p "$OUT_DIR"

if [[ ! -x target/profiling/boo ]]; then
  echo "target/profiling/boo is missing; run 'cargo build --profile profiling' first" >&2
  exit 1
fi

for scenario in "${SCENARIOS[@]}"; do
  for impl in "${IMPLS[@]}"; do
    run_dir="$OUT_DIR/$scenario/$impl"
    mkdir -p "$run_dir"
    socket="$run_dir/boo.sock"
    gui_socket="$run_dir/gui.sock"
    gui_status="$run_dir/gui-status.txt"
    perf_out="$run_dir/perf.data"
    server_log="$run_dir/server.log"
    client_log="$run_dir/client.log"
    summary_json="$run_dir/summary.json"

    echo "scenario=$scenario impl=$impl"
    CLIENT_IMPL="$impl" \
    SOCKET="$socket" \
    GUI_TEST_SOCKET="$gui_socket" \
    GUI_TEST_STATUS="$gui_status" \
    OUT="$perf_out" \
    SERVER_LOG="$server_log" \
    CLIENT_LOG="$client_log" \
    bash scripts/profile-bench-scenario.sh "$scenario" \
      --duration "$DURATION" \
      --ready-timeout "$READY_TIMEOUT" \
      --profile-target "$PROFILE_TARGET" \
      --profiler "$PROFILER"

    python3 scripts/summarize-boo-profile.py "$client_log" \
      --top 12 \
      --paths "${SUMMARY_PATHS[@]}" >"$summary_json"
  done
done

python3 - "$OUT_DIR" <<'PY'
import json
import os
import csv
import sys

out_dir = sys.argv[1]
rows = []
for scenario in sorted(os.listdir(out_dir)):
    scenario_dir = os.path.join(out_dir, scenario)
    if not os.path.isdir(scenario_dir):
        continue
    for impl in sorted(os.listdir(scenario_dir)):
        summary_path = os.path.join(scenario_dir, impl, "summary.json")
        if not os.path.exists(summary_path):
            continue
        with open(summary_path, "r", encoding="utf-8") as f:
            data = json.load(f)
        entry_map = {(entry["path"], entry["kind"]): entry for entry in data["entries"]}
        def metric(path, key):
            entry = entry_map.get((path, "cpu")) or entry_map.get((path, "io")) or entry_map.get((path, "wait"))
            return entry.get(key, 0) if entry else 0
        rows.append(
            {
                "scenario": scenario,
                "impl": impl,
                "render_ms": metric("client.view.render_terminal_scene", "total_ms"),
                "canvas_ms": metric("client.canvas.draw", "total_ms"),
                "text_diff_ms": metric("client.text_layer.diff", "total_ms"),
                "text_draw_ms": metric("client.text_layer.draw", "total_ms"),
                "stream_apply_ms": metric("client.stream.apply_delta", "total_ms"),
            }
        )

rows.sort(key=lambda row: (row["scenario"], row["impl"]))

results_json = os.path.join(out_dir, "results.json")
with open(results_json, "w", encoding="utf-8") as f:
    json.dump(rows, f, indent=2)

results_csv = os.path.join(out_dir, "results.csv")
with open(results_csv, "w", encoding="utf-8", newline="") as f:
    writer = csv.DictWriter(
        f,
        fieldnames=[
            "scenario",
            "impl",
            "render_ms",
            "canvas_ms",
            "text_diff_ms",
            "text_draw_ms",
            "stream_apply_ms",
        ],
    )
    writer.writeheader()
    writer.writerows(rows)

for row in rows:
    print(
        f"scenario={row['scenario']} impl={row['impl']} "
        f"render_ms={row['render_ms']:.3f} canvas_ms={row['canvas_ms']:.3f} "
        f"text_diff_ms={row['text_diff_ms']:.3f} text_draw_ms={row['text_draw_ms']:.3f} "
        f"stream_apply_ms={row['stream_apply_ms']:.3f}"
    )

print(f"wrote_json={results_json}")
print(f"wrote_csv={results_csv}")
PY
