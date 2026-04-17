use crate::config;
use crate::control;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Generator, Shell, generate};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "boo",
    about = "Terminal multiplexer and GUI client for Boo sessions",
    disable_help_flag = true,
    long_about = "Terminal multiplexer and GUI client for Boo sessions.\n\nRunning `boo` with no subcommand opens the GUI client.",
    after_long_help = "Remote flags apply both before and after subcommands, for example `boo --host macbook ls` and `boo ls --host macbook`."
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, clap::Args, Default)]
pub struct GlobalArgs {
    #[arg(short = 'h', action = clap::ArgAction::HelpShort, global = true, help = "Print help")]
    pub help_short: bool,

    #[arg(long = "help", action = clap::ArgAction::HelpLong, global = true, help = "Print long help")]
    pub help_long: bool,

    #[arg(long, global = true, help = "Run without opening the GUI window")]
    pub headless: bool,

    #[arg(long, global = true, help = "Connect through SSH to a remote Boo host")]
    pub host: Option<String>,

    #[arg(long, global = true, help = "Override the local control socket path")]
    pub socket: Option<String>,

    #[arg(
        long = "remote-workdir",
        global = true,
        help = "Remote workdir used before starting Boo over SSH"
    )]
    pub remote_workdir: Option<String>,

    #[arg(
        long = "remote-socket",
        global = true,
        help = "Remote control socket path used on the SSH host"
    )]
    pub remote_socket: Option<String>,

    #[arg(
        long = "remote-binary",
        global = true,
        help = "Remote Boo binary path used on the SSH host"
    )]
    pub remote_binary: Option<String>,

    #[arg(long = "remote-port", global = true, help = "Start the TCP remote daemon on this port")]
    pub remote_port: Option<u16>,

    #[arg(
        long = "remote-auth-key",
        global = true,
        help = "Shared secret for the TCP remote daemon"
    )]
    pub remote_auth_key: Option<String>,

    #[arg(long, global = true, help = "Session layout to load at startup")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Connect the GUI client to a Boo server
    Attach,
    /// Print shell completion scripts
    Completions {
        #[arg(value_enum, default_value_t = CompletionShell::Bash)]
        shell: CompletionShell,
    },
    /// Stop the Boo server
    KillServer,
    /// List live sessions on the Boo server
    Ls,
    /// Create a new live session
    NewSession,
    /// Stop the Boo server
    QuitServer,
    /// Run the Boo session server without a GUI
    Server,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

pub enum Outcome {
    Continue,
    Exit(i32),
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

fn print_completions<G: Generator>(generator: G) -> Result<(), String> {
    let mut command = Cli::command();
    let mut stdout = std::io::stdout().lock();
    generate(generator, &mut command, "boo", &mut stdout);
    Ok(())
}

pub fn handle_command<F>(cli: &Cli, boo_config: &config::Config, mut ensure_server_running: F) -> Outcome
where
    F: FnMut(&str, &config::Config),
{
    let Some(command) = cli.command.as_ref() else {
        return Outcome::Continue;
    };
    if matches!(command, Command::Server) {
        return Outcome::Continue;
    }

    let socket_path = boo_config
        .control_socket
        .clone()
        .unwrap_or_else(control::default_socket_path);

    match command {
        Command::Ls => {
            ensure_server_running(&socket_path, boo_config);
            let client = control::Client::connect(socket_path);
            match client.request(&control::Request::ListTabs) {
                Ok(control::Response::Tabs { tabs }) => {
                    let mut stdout = std::io::stdout().lock();
                    use std::io::Write;
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
        Command::Completions { shell } => {
            let result = match shell {
                CompletionShell::Bash => print_completions(Shell::Bash),
                CompletionShell::Zsh => print_completions(Shell::Zsh),
                CompletionShell::Fish => print_completions(Shell::Fish),
            };
            if let Err(error) = result {
                eprintln!("{error}");
                return Outcome::Exit(1);
            }
            Outcome::Exit(0)
        }
        Command::KillServer | Command::QuitServer => {
            ensure_server_running(&socket_path, boo_config);
            let client = control::Client::connect(socket_path);
            if let Err(error) = client.send(&control::Request::Quit) {
                eprintln!("{error}");
                return Outcome::Exit(1);
            }
            Outcome::Exit(0)
        }
        Command::NewSession => {
            ensure_server_running(&socket_path, boo_config);
            let client = control::Client::connect(socket_path);
            if let Err(error) = client.send(&control::Request::NewTab) {
                eprintln!("{error}");
                return Outcome::Exit(1);
            }
            Outcome::Exit(0)
        }
        Command::Attach => Outcome::Continue,
        Command::Server => Outcome::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, CompletionShell};
    use clap::{CommandFactory, Parser, error::ErrorKind};

    #[test]
    fn parse_global_flags_after_subcommand() {
        let cli = Cli::parse_from([
            "boo",
            "ls",
            "--host",
            "example-mbp.local",
            "--remote-binary",
            "/Users/example/dev/boo/target/debug/boo",
        ]);
        assert_eq!(cli.global.host.as_deref(), Some("example-mbp.local"));
        assert_eq!(
            cli.global.remote_binary.as_deref(),
            Some("/Users/example/dev/boo/target/debug/boo")
        );
    }

    #[test]
    fn parse_global_flags_before_subcommand() {
        let cli = Cli::parse_from([
            "boo",
            "--host",
            "example-mbp.local",
            "--remote-binary",
            "/Users/example/dev/boo/target/debug/boo",
            "ls",
        ]);
        assert_eq!(cli.global.host.as_deref(), Some("example-mbp.local"));
        assert_eq!(
            cli.global.remote_binary.as_deref(),
            Some("/Users/example/dev/boo/target/debug/boo")
        );
        assert!(matches!(cli.command, Some(super::Command::Ls)));
    }

    #[test]
    fn parse_completions_shell() {
        let cli = Cli::parse_from(["boo", "completions", "zsh"]);
        match cli.command {
            Some(super::Command::Completions { shell }) => {
                assert!(matches!(shell, CompletionShell::Zsh));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn help_mentions_default_gui_behavior() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("Running `boo` with no subcommand opens the GUI client."));
        assert!(help.contains("Connect through SSH to a remote Boo host"));
    }

    #[test]
    fn short_and_long_help_are_different() {
        let short = Cli::try_parse_from(["boo", "-h"]).unwrap_err();
        let long = Cli::try_parse_from(["boo", "--help"]).unwrap_err();
        assert_eq!(short.kind(), ErrorKind::DisplayHelp);
        assert_eq!(long.kind(), ErrorKind::DisplayHelp);
        let short_text = short.to_string();
        let long_text = long.to_string();
        assert_ne!(short_text, long_text);
        assert!(!short_text.contains("Running `boo` with no subcommand opens the GUI client."));
        assert!(long_text.contains("Running `boo` with no subcommand opens the GUI client."));
        assert!(long_text.contains("Remote flags apply both before and after subcommands"));
    }
}
