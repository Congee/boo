# Config Reference

Primary config file:

- `~/.config/boo/config.boo`

## Main Configuration Areas

- keybindings and prefix key
- UI appearance and colors
- control socket path
- remote desktop settings
- Boo-native remote daemon settings

## High-Value Keys

- `prefix-key`
- `control-socket`
- `remote-port`
- `remote-auth-key`
- `keybind`
- terminal and color-related appearance keys

## Include Model

Config files can `include` other snippets. Later entries override earlier ones.

## Related Implementation

- `src/config.rs`
- `src/cli.rs`

Related docs:

- [../remote/ssh-desktop.md](../remote/ssh-desktop.md)
- [../remote/requirements.md](../remote/requirements.md)
