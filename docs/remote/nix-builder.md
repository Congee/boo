# Remote Nix Builder

This doc covers the Nix-oriented build and verification workflow for remote Boo
work, especially Linux-to-macOS build delegation.

Current practical stance:

- use Nix for Linux package verification
- keep direct `cargo build` on the Mac as the source of truth for full remote
  Mac verification until Darwin Nix packaging is fully healthy

Detailed workflow:

- [../../REMOTE_NIX_BUILDER.md](../../REMOTE_NIX_BUILDER.md)
