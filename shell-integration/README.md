# Boo Shell Integration

Boo understands shell prompt markers via `OSC 133`. These scripts emit the
signals Boo already consumes today:

- `OSC 133;A` for a new prompt
- `OSC 133;C;cmdline_url=...` when a command starts
- `OSC 133;D;<status>` when a command finishes
- `OSC 7` when `$PWD` changes

## Install

### Bash

Add this near the end of `~/.bashrc`:

```bash
source "/path/to/boo/shell-integration/bash/boo.bash"
```

### Zsh

Add this to `~/.zshrc`:

```zsh
source "/path/to/boo/shell-integration/zsh/boo.zsh"
```

### Fish

Add this to `~/.config/fish/conf.d/boo.fish` or source it from `config.fish`:

```fish
source /path/to/boo/shell-integration/fish/boo.fish
```
