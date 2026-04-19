# Remote Implementation Checklist

This page tracks concrete remote deliverables and sequencing.

## Current Emphasis

- harden the desktop SSH milestone
- continue the canonical native transport work
- preserve strong verification coverage at each phase

## Phase 1: Desktop SSH

- remote path expansion
- remote bootstrap error classification
- tunnel lifecycle recovery
- version mismatch detection
- end-to-end verification around forwarded control and `.stream`

## Phase 2: Canonical Native Transport

- unified handshake and capability negotiation
- shared connection model for desktop and iOS directions
- stronger reconnect/resume behavior
- encrypted direct transport and transport fallback

## Related Docs

- [./requirements.md](./requirements.md)
- [./ssh-desktop.md](./ssh-desktop.md)
- [../modules/remote-daemon.md](../modules/remote-daemon.md)
