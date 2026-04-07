# Profiling Boo

This repo uses two complementary approaches:

- profiling: external measurement to find where time goes
- instrumentation: Boo's own low-overhead spans/counters to explain which app phase is hot

Use profiling first. Use instrumentation to explain the hotspots in Boo terms.

## Build Profile

Use the dedicated Cargo profiling profile for all serious profiling runs:

```bash
cargo build --profile profiling
```

This profile:

- inherits from `release`
- keeps line-table debug info for sampled stacks
- uses one codegen unit to avoid noisier profile artifacts

For better stack quality, also use frame pointers:

```bash
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling
```

## Recommended Tooling

Cross-platform default:

- `samply`

macOS deep dive:

- `Instruments` Time Profiler
- `Instruments` System Trace

Linux deep dive:

- `perf`
- `cargo flamegraph`
- `Hotspot` as a UI if desired

Repo-local supplemental view:

- `BOO_PROFILE=1` built-in stage profiler

## What To Measure

For Boo, keep CPU time, wait time, and I/O behavior separate.

CPU-bound paths:

- `server.backend.poll`
- `vt.write`
- `snapshot.refresh`
- `server.stream.encode_state`
- `client.stream.apply_delta`
- `client.canvas.draw`

I/O-bound or wait-heavy paths:

- PTY reads
- Unix socket reads/writes
- idle sleeps / scheduler wakeups
- server/client reconnect behavior

Representative workloads:

- startup
- typing at a shell prompt
- `cat ~/config.json`
- `seq 1 5000`
- full-screen apps such as `vim`, including page motion and exit

## Built-In Boo Instrumentation

Boo already exposes path summaries when enabled:

```bash
BOO_PROFILE=1 cargo run
```

This prints rolling summaries for named paths with:

- `cpu`
- `io`
- `wait`
- `bytes`
- `units`

Use this to correlate sampled hotspots with Boo-specific phases. It is not a replacement for a sampler.

Notable unit counters now include:

- `server.stream.encode_full_state.local`
- `server.stream.encode_delta.local`
- `server.stream.encode_delta_rows.local`
- `client.stream.decode_full_state`
- `client.stream.decode_delta`
- `client.stream.decode_delta_rows`
- `client.stream.apply_delta_rows`
- `client.stream.apply_delta_cells`
- `client.canvas.changed_rows`
- `client.canvas.changed_chunks`

## Cross-Platform Default: Samply

Install:

```bash
cargo install samply
```

Profile the server:

```bash
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling
samply record ./target/profiling/boo server --socket /tmp/boo.sock
```

In another shell, drive workload against it:

```bash
python3 scripts/ui-test-client.py --socket /tmp/boo.sock snapshot
```

Or run a burst workload:

```bash
python3 scripts/ui-test-client.py --socket /tmp/boo.sock request send-text text=$'cat ~/config.json\r'
```

Profile the GUI client:

```bash
samply record ./target/profiling/boo
```

Why use `samply` first:

- works on macOS and Linux
- good sampled call stacks
- Firefox Profiler UI is strong for threads and timelines

## macOS: Instruments

Use Instruments when the problem involves GUI responsiveness, wakeups, compositor behavior, or thread scheduling.

### Time Profiler

Use for:

- CPU hotspots
- hot functions in render, delta apply, VT ingest
- cross-thread attribution in the GUI process and server process

Suggested flow:

1. Build the profiling profile.
2. Launch `Instruments`.
3. Choose `Time Profiler`.
4. Profile either:
   - `target/profiling/boo`
   - `target/profiling/boo server --socket /tmp/boo.sock`
5. Run a reproducible workload:
   - startup
   - `cat ~/config.json`
   - `vim`
6. Inspect:
   - heavy stacks
   - per-thread activity
   - self time vs total time

### System Trace

Use for:

- blocked vs running threads
- wakeup storms
- socket/PTY wait behavior
- event loop scheduling issues

This is the right tool when Boo "feels laggy" but CPU hotspots alone do not explain it.

### Notes

- Time Profiler is the first macOS tool to reach for.
- System Trace is the second tool when the issue looks like wakeups, blocking, or scheduler behavior.

## Linux: perf

Use `perf` for canonical Linux sampling:

```bash
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling
perf record -g ./target/profiling/boo server --socket /tmp/boo.sock
perf report
```

If you want a better UI:

- open the data in `Hotspot`
- or use `perf script` plus other viewers as needed

## Linux: cargo flamegraph

Use for a fast CPU hotspot picture:

```bash
cargo install flamegraph
RUSTFLAGS="-C force-frame-pointers=yes" cargo flamegraph --profile profiling -- server --socket /tmp/boo.sock
```

This is most useful on Linux. On macOS, prefer Instruments and `samply`.

## When To Use What

Use `samply` when:

- you want one cross-platform default
- you want timeline and sampled stack views

Use `Instruments Time Profiler` when:

- the issue is macOS GUI performance
- you need good thread-level visibility

Use `Instruments System Trace` when:

- the app feels laggy but CPU stacks are not the whole story
- you suspect blocking, wakeups, or scheduling issues

Use `perf` or `cargo flamegraph` when:

- working on Linux
- you want quick CPU hotspot analysis

Use `BOO_PROFILE=1` when:

- you already know the hotspot area
- you need Boo-domain phase breakdowns

## Recommended Workflow

1. Reproduce with one stable workload.
2. Run a sampler first:
   - `samply` cross-platform
   - or Instruments/perf on the target platform
3. Use `BOO_PROFILE=1` to map the hotspot to Boo phases.
4. Make one targeted change.
5. Re-run the same workload and compare.

Avoid optimizing from logs or intuition alone when a sampler is available.
