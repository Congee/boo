use crate::config;
use crate::control;
use std::io::Write;

const CLI_SUBCOMMANDS: &[(&str, &str)] = &[
    ("attach", "connect the GUI client to the local Boo server"),
    ("completions", "print shell completion scripts"),
    ("kill-server", "stop the local Boo server"),
    ("ls", "list live sessions on the local Boo server"),
    ("new-session", "create a new live session on the local Boo server"),
    ("quit-server", "stop the local Boo server"),
    ("server", "run the Boo session server without a GUI"),
];

pub enum Outcome {
    Continue,
    Exit(i32),
}

fn bash_completion_script() -> String {
    let commands = CLI_SUBCOMMANDS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        r#"_boo_completions() {{
    local cur prev words cword
    _init_completion || return

    prev="${{COMP_WORDS[COMP_CWORD-1]}}"
    case "$prev" in
        --socket)
            COMPREPLY=($(compgen -f -- "$cur"))
            return
            ;;
        --remote-port)
            return
            ;;
        --remote-auth-key|--session)
            return
            ;;
        completions)
            COMPREPLY=($(compgen -W "bash zsh fish" -- "$cur"))
            return
            ;;
    esac

    if [[ $COMP_CWORD -eq 1 ]]; then
        COMPREPLY=($(compgen -W "{commands}" -- "$cur"))
        return
    fi

    COMPREPLY=($(compgen -W "--headless --socket --remote-port --remote-auth-key --session --help" -- "$cur"))
}}

complete -F _boo_completions boo
"#
    )
}

fn zsh_completion_script() -> String {
    let mut lines = vec![
        "#compdef boo".to_string(),
        String::new(),
        "_boo() {".to_string(),
        "  local -a commands".to_string(),
        "  commands=(".to_string(),
    ];
    for (name, description) in CLI_SUBCOMMANDS {
        lines.push(format!("    '{}:{}'", name, description));
    }
    lines.extend([
        "  )".to_string(),
        "  _arguments -C \\".to_string(),
        "    '--headless[run without opening the GUI window]' \\".to_string(),
        "    '--socket=[override local control socket path]:socket path:_files' \\".to_string(),
        "    '--remote-port=[start the TCP remote daemon]:port:' \\".to_string(),
        "    '--remote-auth-key=[shared secret for remote auth]:secret:' \\".to_string(),
        "    '--session=[session layout to load]:session:' \\".to_string(),
        "    '--help[show help]' \\".to_string(),
        "    '1:command:->command' \\".to_string(),
        "    '*::arg:->args'".to_string(),
        String::new(),
        "  case $state in".to_string(),
        "    command)".to_string(),
        "      _describe 'boo command' commands".to_string(),
        "      ;;".to_string(),
        "    args)".to_string(),
        "      if [[ ${words[2]} == completions ]]; then".to_string(),
        "        _values 'shell' bash zsh fish".to_string(),
        "      fi".to_string(),
        "      ;;".to_string(),
        "  esac".to_string(),
        "}".to_string(),
        String::new(),
        "_boo \"$@\"".to_string(),
    ]);
    lines.join("\n")
}

fn fish_completion_script() -> String {
    let mut lines = vec![
        "complete -c boo -f".to_string(),
        "complete -c boo -l headless -d 'run without opening the GUI window'".to_string(),
        "complete -c boo -l socket -r -d 'override local control socket path'".to_string(),
        "complete -c boo -l remote-port -r -d 'start the TCP remote daemon'".to_string(),
        "complete -c boo -l remote-auth-key -r -d 'shared secret for remote auth'".to_string(),
        "complete -c boo -l session -r -d 'session layout to load'".to_string(),
    ];
    for (name, description) in CLI_SUBCOMMANDS {
        lines.push(format!(
            "complete -c boo -n '__fish_use_subcommand' -a '{name}' -d '{description}'"
        ));
    }
    lines.push(
        "complete -c boo -n '__fish_seen_subcommand_from completions' -a 'bash zsh fish' -d 'shell'"
            .to_string(),
    );
    lines.join("\n")
}

fn print_completions(shell: &str) -> Result<(), String> {
    let script = match shell {
        "bash" => bash_completion_script(),
        "zsh" => zsh_completion_script(),
        "fish" => fish_completion_script(),
        other => return Err(format!("unsupported shell: {other}")),
    };
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(script.as_bytes())
        .map_err(|error| format!("write completions: {error}"))?;
    stdout
        .write_all(b"\n")
        .map_err(|error| format!("write completions: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("flush completions: {error}"))
}

pub fn handle_command<F>(
    args: &[String],
    boo_config: &config::Config,
    mut ensure_server_running: F,
) -> Outcome
where
    F: FnMut(&str, &config::Config),
{
    let Some(command) = args.get(1).map(String::as_str) else {
        return Outcome::Continue;
    };
    if matches!(command, "server") {
        return Outcome::Continue;
    }

    let socket_path = boo_config
        .control_socket
        .clone()
        .unwrap_or_else(control::default_socket_path);

    match command {
        "ls" => {
            let client = control::Client::connect(socket_path);
            match client.request(&control::Request::ListTabs) {
                Ok(control::Response::Tabs { tabs }) => {
                    let mut stdout = std::io::stdout().lock();
                    for tab in tabs {
                        let marker = if tab.active { "*" } else { " " };
                        let _ = writeln!(stdout, "{marker} {}\t{}", tab.index + 1, tab.title);
                    }
                    let _ = stdout.flush();
                    Outcome::Exit(0)
                }
                Ok(control::Response::Error { error }) => {
                    eprintln!("{error}");
                    Outcome::Exit(1)
                }
                Ok(other) => {
                    eprintln!("unexpected response: {other:?}");
                    Outcome::Exit(1)
                }
                Err(error) => {
                    eprintln!("{error}");
                    Outcome::Exit(1)
                }
            }
        }
        "completions" => {
            let shell = args.get(2).map(String::as_str).unwrap_or("bash");
            if let Err(error) = print_completions(shell) {
                eprintln!("{error}");
                return Outcome::Exit(1);
            }
            Outcome::Exit(0)
        }
        "kill-server" | "quit-server" => {
            let client = control::Client::connect(socket_path);
            if let Err(error) = client.send(&control::Request::Quit) {
                eprintln!("{error}");
                return Outcome::Exit(1);
            }
            Outcome::Exit(0)
        }
        "new-session" => {
            ensure_server_running(&socket_path, boo_config);
            let client = control::Client::connect(socket_path);
            if let Err(error) = client.send(&control::Request::NewTab) {
                eprintln!("{error}");
                return Outcome::Exit(1);
            }
            Outcome::Exit(0)
        }
        "attach" => Outcome::Continue,
        _ => Outcome::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::{bash_completion_script, fish_completion_script, zsh_completion_script};

    #[test]
    fn completion_scripts_include_core_subcommands() {
        let bash = bash_completion_script();
        let zsh = zsh_completion_script();
        let fish = fish_completion_script();

        for script in [bash, zsh, fish] {
            assert!(script.contains("new-session"));
            assert!(script.contains("kill-server"));
            assert!(script.contains("quit-server"));
            assert!(script.contains("completions"));
        }
    }
}
