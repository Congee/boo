# Profiling

Profiling in boo uses two complementary lanes:

- external profilers to find where time goes
- built-in Boo instrumentation to explain the hot path in Boo terms

## First Tools To Reach For

- cross-platform: `samply`
- macOS: Instruments
- Linux: `perf`, `cargo flamegraph`, Hotspot
- repo-local instrumentation: `BOO_PROFILE=1`

## Build

```bash
cargo build --profile profiling
```

For better sampled stacks:

```bash
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling
```

## What To Focus On

- PTY ingest
- snapshot refresh
- server stream publish
- client delta apply
- renderer draw
- wait-heavy paths and cadence regressions

## Related Docs

- [../development/testing.md](../development/testing.md)
- [../modules/renderer.md](../modules/renderer.md)
- [../modules/vt-backend-core.md](../modules/vt-backend-core.md)
