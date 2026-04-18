# Remote Nix Builder Workflow

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

## Remote Darwin Nix Helper

The repository includes a helper for remote Darwin package attempts:

```bash
bash scripts/nix-build-remote-darwin.sh example-mbp.local .#packages.aarch64-darwin.default --dry-run --no-link
```

At the moment:

- Linux Nix packaging works.
- The split `libghostty-vt` package path is not yet healthy on Darwin.
- Darwin still fails in Ghostty's Apple SDK discovery path under Nix.

So the helper is useful for visibility, but not yet the primary Mac build path.

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
