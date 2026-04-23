# CLI Reference

The CLI is defined in `src/cli.rs` and uses `clap`.

## Main Entry Points

- `boo`
- `boo server`
- `boo --headless`
- `boo --host <ssh-host>`
- `boo ls`
- `boo new-tab`
- `boo kill-server`
- `boo remote-clients`

## Important Flags

- `--socket`: override the local control socket path
- `--host`: SSH-backed desktop remote host
- `--remote-socket`: remote control socket path on the SSH host
- `--remote-workdir`: remote working directory for SSH bootstrap
- `--remote-binary`: remote Boo binary path for SSH bootstrap
- `--remote-port`: Boo-native TCP daemon port

## Mental Model

- local mode: view the local runtime server
- server mode: run the long-lived runtime owner directly
- headless mode: run the shared runtime without a GUI
- SSH remote mode: use SSH to bootstrap and forward the remote local-socket
  contract
- native remote daemon mode: expose Boo's TCP daemon for direct/iOS clients

For current behavior details, read:

- [../reference/features.md](../reference/features.md)
- [../remote/ssh-desktop.md](../remote/ssh-desktop.md)
- [../modules/control-socket.md](../modules/control-socket.md)
