# Remote Nix Builder

This doc covers the Nix-oriented build and verification workflow for remote Boo
work, especially Linux-to-macOS build delegation.

Current practical stance:

- use Nix for Linux package verification
- keep direct `cargo build` on the Mac as the source of truth for full remote
  Mac verification until Darwin Nix packaging is fully healthy

## Current Workflow

- inspect flake outputs locally
- build Linux packages locally with Nix
- offload Darwin builds to a remote Mac when needed
- use direct host-side `cargo build` and validation for the final truth on macOS

## Related Docs

- [./requirements.md](./requirements.md)
- [./implementation-checklist.md](./implementation-checklist.md)
