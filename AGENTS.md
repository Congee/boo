# Boo Agent Notes

## Build

```bash
# Enter dev shell when you need the full repo toolchain
nix develop

# Build
cargo build
```

## Architecture Snapshot

- `libghostty-vt` is the shared terminal runtime on macOS and Linux
- `boo --headless` runs the same VT/runtime stack without starting the GUI
- `iced` owns window chrome, terminal rendering, overlays, and pane layout
- macOS host code handles native view focus, text input/IME, clipboard, and notifications
- Linux host code provides platform glue while sharing the same VT core

## Project Structure

- `src/ffi.rs` is the hand-written FFI boundary where Boo still talks to native APIs
- `src/vt_backend_core.rs` is the shared VT pane/runtime core
- `src/main.rs` is the `iced` application shell and shared app state
- `src/platform/macos.rs` is the macOS host integration layer
- `build.rs` links Boo against the native dependencies it needs

## Conventions

- No bindgen. Hand-write the relatively small FFI surface we actually use.
- The shipped app depends on the published `libghostty-vt` crates, not the full Ghostty app runtime.
- macOS and Linux share one VT architecture; platform code should stay thin.

## Visual Verification

- When verifying Boo visually, use the repo's process-targeted screenshot workflow or helper script for the `boo` app/window.
- Do not rely on plain `screencapture` of the frontmost window or full screen, because it can capture the Codex window instead of Boo.
- Treat a mis-targeted screenshot as invalid verification.

## Input Testing

- Do not rely on Boo being frontmost for automated testing if there is a direct, socket-based, or otherwise app-targeted path available.
- Prefer reproducible input injection that targets Boo directly over global OS key injection that depends on focus.
- Do not use synthetic key injection as the primary acceptance test for focus-sensitive bugs if the injection method can change focus or alter event routing into Boo.
## Boo Repro Notes

- For the held-`j` `less docs/reference/features.md` stutter issue, use the real acceptance path:
  - preload `less docs/reference/features.md`
  - verify `docs/reference/features.md` is actually active from the GUI status file before recording
  - record the Boo window with `scripts/record-macos-window.swift ... --until-exit`
  - judge regressions by the recorded window, not proxy counters alone

- Do not rely on synthetic key injection as the primary acceptance test for focus-sensitive bugs if the injection path can alter focus or event routing.

- Do not rely on Boo being frontmost for automated testing when a direct app-targeted path exists, unless the bug is specifically about frontmost/native focus behavior.

## Benchmark Workflow

- `docs/reference/features.md` is too small to use as the primary terminal performance artifact.
- On Linux, video capture is optional. Treat socket-based scenario checks, UI snapshots, `BOO_PROFILE=1`, and sampled profiling as the primary benchmark and verification path.
- Use video on Linux only when a regression is specifically visual, such as cadence, stutter, or compositor-facing behavior.
- Generate the local benchmark corpus with:
  - `bash scripts/gen-terminal-bench-corpus.sh`
- Use named scenarios instead of ad hoc commands:
  - `bash scripts/run-terminal-bench.sh <scenario>`
  - `bash scripts/profile-bench-scenario.sh <scenario>`
  - `bash scripts/record-bench-scenario.sh <scenario> /tmp/out.mp4 <seconds>`
- Analyze recorded MP4s with:
  - `python3 scripts/analyze-terminal-recording.py <video...>`
- Current generated workload categories:
  - `plain-cat`
  - `wrap-cat`
  - `unicode-cat`
  - `pager-less`
  - `ansi-churn`
  - `cursor-storm`

## Renderer Findings

- Commit `e50f98d` added:
  - the terminal benchmark corpus/tooling
  - `TerminalBodyLayer`
  - `ModelParagraph` as the older paragraph-heavy renderer
  - `canvas_text` as the direct renderer path selected by default
- Corpus-backed findings:
  - `pager-less` on the large generated pager corpus was promising for `canvas_text`
  - `unicode-cat` initially collapsed badly on `canvas_text`
  - the finite-width fix in `src/vt_terminal_canvas.rs` removed that unicode collapse
  - the remaining paragraph path still pays per-run paragraph build/diff overhead that the direct canvas path avoids
- Current conclusion:
  - prefer the direct `canvas_text` path by default
  - keep `BOO_TERMINAL_BODY_IMPL=model_paragraph` available for comparison and regression checks
  - if continuing this line of work, investigate canvas invalidation/redraw cadence and row/run generation costs before adding more paragraph caching

## Held-j Findings

- The large visible freeze regression was introduced by unsafe redraw-skip paths and was fixed by commit `77d7d6d`.
- After that fix, the remaining held-`j` issue is not dominated by renderer CPU.
- Profiling findings from the real `BOO_PROFILE=1` held-`j` repro:
  - `client.canvas.draw` is about `0.21–0.28ms` avg
  - `client.stream.apply_delta` is about `0.10–0.15ms` avg
  - `client.view.render_terminal_scene` is about `0.013–0.018ms` avg
  - the bigger recurring cost is cadence:
    - `client.latency.stream_delta` about `17–22ms` avg, up to about `37.8ms`
    - `client.stream.read_message` about `29–34ms` avg wait
    - server `stream.read_message` / update cadence shows similar `~30ms` timing

- Rejected directions for this issue because they increased throughput but made visible cadence worse:
  - uncapped or synthetic repeaters
  - forcing per-frame subscription redraws
  - changing focused local transport to scroll/all-row delta shapes
  - naive text-layer paragraph/run merging
  - naive focused full-state snapshot reuse

- Current best next target:
  - investigate focused-pane local stream/message cadence and publish timing before touching renderer logic again
