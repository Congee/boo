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
- settle the SSH auth boundary:
  use `authorized_keys`-style public-key verification on the server and
  platform keychain or agent-backed keys on the client, without storing
  private keys inside Boo

## Verification

Prefer direct socket-level checks:

- forwarded control socket health
- `get-ui-snapshot` over the tunnel
- tab creation through the forwarded path
- `.stream` attach/update behavior

## Related Docs

- [./requirements.md](./requirements.md)
- [./implementation-checklist.md](./implementation-checklist.md)
