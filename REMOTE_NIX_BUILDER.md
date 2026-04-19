# Remote Nix Builder Workflow

Structured remote docs now live under [`docs/remote/`](./docs/remote). For the
docs-tree entrypoint, start with
[docs/remote/nix-builder.md](./docs/remote/nix-builder.md).

This note documents the current Nix-oriented build and verification flow for Boo remote work.

## Local Linux

Show flake outputs:

```bash
nix flake show --all-systems --no-write-lock-file
```

Build the local Linux package:

```bash
nix build .#packages.x86_64-linux.default --no-link
```

The normal Rust verification path remains:

```bash
cargo check
```

## Remote Mac Sync And Direct Build

Today, the working macOS verification path is still a direct host build on
`example-mbp.local`:

```bash
bash scripts/sync-remote-mac.sh example-mbp.local /Users/example/dev/boo
ssh example-mbp.local 'cd /Users/example/dev/boo && cargo build'
ssh example-mbp.local 'cd /Users/example/dev/boo && bash scripts/test-ios-remote-view.sh'
```

This is the current source of truth for remote Mac validation while Darwin Nix
packaging remains incomplete.

## Remote Nix Helper (Portable, no-sudo)

`scripts/nix-build-remote.sh` is a portable helper that offloads a Nix build to an
SSH-reachable host. It works in both directions and needs no `/etc` edits or
root SSH key setup:

```bash
# Linux → Mac (aarch64-darwin build offloaded to a Mac host):
./scripts/nix-build-remote.sh example-mbp aarch64-darwin

# Mac → Linux (x86_64-linux build offloaded to a Linux host):
./scripts/nix-build-remote.sh blackbox x86_64-linux

# Non-default flake attribute + extra flags:
./scripts/nix-build-remote.sh example-mbp aarch64-darwin \
  .#checks.aarch64-darwin.default --print-build-logs
```

Under the hood the script issues `nix build --eval-store auto --store
ssh-ng://<user>@<host> ...`:

- **Evaluation runs locally** (so flake paths and local inputs resolve).
- **The build runs on the remote host** under the calling user's SSH creds, in
  the remote host's Nix store. No `nix-daemon` on the requester ever touches
  SSH, so there is nothing to set up as root.

`scripts/nix-build-remote-darwin.sh` is kept as a thin backwards-compatible shim
that forwards to `nix-build-remote.sh` with `aarch64-darwin` pre-filled.

### Prerequisites

- `ssh <ssh-host>` works non-interactively (key-based) as the calling user.
- The calling user is a trusted Nix user on the remote host, i.e.
  `nix store info --store ssh-ng://<ssh-host>` prints `Trusted: 1`.

Both are true today for `example@example-mbp` and `example@blackbox`.

### Where the build artifact ends up

Because `--store ssh-ng://...` targets the remote store, the output lives in
`/nix/store/...` on the remote host, not the requester. That fits the
deploy workflow ("build on the Mac so the Mac has the binary"). If you
specifically need the artifact back on the requester afterwards:

```bash
nix copy --from ssh-ng://<user>@<host> /nix/store/XXX-...
```

### Current state of the Darwin path

- Linux Nix packaging works natively.
- Linux → Mac trivial derivations (`nixpkgs#hello --system aarch64-darwin`)
  succeed end-to-end via the helper.
- Linux → Mac `boo` package: the Rust build completes cleanly; the
  `cargoCheckHook` test phase fails on `@rpath/libghostty-vt.dylib` not
  resolving inside the Nix sandbox. This is a package-level bug in
  `flake.nix`'s Darwin derivation (missing `DYLD_LIBRARY_PATH` / rpath
  setup around the cargo test invocation), not a remote-builder mechanism
  issue.

Until the Darwin flake derivation is patched, pass `--arg doCheck false`
or add `--override-input` workarounds when exercising the Darwin package,
or keep using direct `cargo build` on the Mac host for full-fidelity
verification.

## Current Recommendation

Use:

- Nix for Linux package verification
- direct `cargo build` on `example-mbp.local` for real Mac verification

Do not block the remote desktop or native transport work on the Darwin Nix issue.

## Stable Remote Binary Reference

When the Darwin Nix build is healthy, the preferred remote binary reference should be a managed
wrapper or symlink on the remote host, not a raw Nix store path pasted into config.

Recommended shape:

- keep `remote-binary` pointed at a stable host-local path such as:
  - `/Users/example/.local/bin/boo-current`
- update that symlink or wrapper to point at the latest successful Nix build result

Why:

- the user config stays stable
- SSH bootstrap does not need to know the current store hash
- changing the build output does not require rewriting config every time
