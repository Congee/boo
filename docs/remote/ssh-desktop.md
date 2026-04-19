# SSH Desktop Remote

This is the current desktop remote milestone:

```bash
boo --host <ssh-host>
```

## Current Design

- bootstrap the remote Boo server over SSH
- forward the remote control socket
- forward the remote `.stream` socket
- attach the local GUI/client to the forwarded local sockets

## Why This Exists

This preserves Boo's current local IPC and GUI model while providing secure
desktop remote access through SSH.

## Still Open

- remote path expansion hardening
- tunnel lifecycle recovery
- clearer mismatch and startup errors

## Detailed Status Doc

- [../../REMOTE_SSH_INTEGRATION_PLAN.md](../../REMOTE_SSH_INTEGRATION_PLAN.md)
