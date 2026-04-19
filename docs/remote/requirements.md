# Remote Requirements

boo remote work currently spans two client modes:

- desktop-to-desktop remote Boo
- iOS/mobile remote Boo

Those modes are expected to converge on one canonical Boo-native transport over
time, even though rollout and UX differ today.

## Current Foundations

- SSH-backed desktop remote bootstrap and forwarding
- Boo-native TCP daemon
- SwiftUI iOS client

## Core Requirements

- keep the server authoritative for PTYs, tabs, splits, and terminal state
- preserve deterministic verification paths
- keep desktop SSH practical without blocking on mobile-specific work
- converge toward one shared remote protocol/session model

## Current Canonical Product Doc

The full detailed product contract remains:

- [../../REMOTE_REQUIREMENTS.md](../../REMOTE_REQUIREMENTS.md)
