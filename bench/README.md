# Terminal Benchmark Corpus

This directory is for local benchmark artifacts used to stress Boo with repeatable workloads.

The generated files are intentionally not checked into git. Create them with:

```bash
bash scripts/gen-terminal-bench-corpus.sh
```

Artifacts are written under `bench/generated/`.

Current workload categories:

- `plain-*.txt`: large plain-text scrollback throughput
- `wrap-*.txt`: very long wrapped lines for layout pressure
- `unicode-*.txt`: wide glyphs, combining marks, emoji, and ZWJ sequences
- `pager-*.txt`: large numbered pager input for `less -N`
- `ansi-truecolor.sh`: dense ANSI color/style churn emitter
- `cursor-storm.sh`: partial-screen cursor motion/update stress

These are intended for:

- PTY throughput tests such as `cat bench/generated/plain-32mb.txt`
- pager cadence tests such as `less -N bench/generated/pager-32mb.txt`
- render/update stress such as `bash bench/generated/ansi-truecolor.sh`

Convenience runners:

- `scripts/run-terminal-bench.sh`
  - runs a named workload directly, for example:
    - `bash scripts/run-terminal-bench.sh plain-cat`
    - `bash scripts/run-terminal-bench.sh pager-less`
- `scripts/profile-bench-scenario.sh`
  - maps a named workload into `scripts/profile-macos-sample-client.sh`, for example:
    - `bash scripts/profile-bench-scenario.sh plain-cat`
    - `bash scripts/profile-bench-scenario.sh unicode-cat --duration 8`
- `scripts/record-bench-scenario.sh`
  - records a named workload with the process-targeted macOS recorder, for example:
    - `bash scripts/record-bench-scenario.sh plain-cat /tmp/boo-plain.mp4 10`
    - `bash scripts/record-bench-scenario.sh pager-less /tmp/boo-pager.mp4 15`
- `scripts/analyze-terminal-recording.py`
  - applies the row-band movement metric to one or more MP4 recordings, for example:
    - `python3 scripts/analyze-terminal-recording.py /tmp/boo-plain.mp4`
    - `python3 scripts/analyze-terminal-recording.py /tmp/boo-a.mp4 /tmp/boo-b.mp4`
