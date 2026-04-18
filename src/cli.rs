use crate::config;
use crate::control;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Generator, Shell, generate};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "boo",
    version = env!("CARGO_PKG_VERSION"),
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
    #[arg(
        short = 'h',
        action = clap::ArgAction::HelpShort,
        global = true,
        required = false,
        help = "Print help"
    )]
    pub help_short: Option<bool>,

    #[arg(
        long = "help",
        action = clap::ArgAction::HelpLong,
        global = true,
        required = false,
        help = "Print long help"
    )]
    pub help_long: Option<bool>,

    #[arg(long, global = true, help = "Run without opening the GUI window")]
    pub headless: bool,

    #[arg(
        long,
        global = true,
        help = "Connect through SSH to a remote Boo host using forwarded Boo control/stream sockets"
    )]
    pub host: Option<String>,

    #[arg(
        long,
        global = true,
        help = "Override the local control socket path; in SSH mode this is the local forwarded socket"
    )]
    pub socket: Option<String>,

    #[arg(
        long = "remote-workdir",
        global = true,
        help = "Remote working directory used before starting Boo over SSH"
    )]
    pub remote_workdir: Option<String>,

    #[arg(
        long = "remote-socket",
        global = true,
        help = "Remote Boo control socket path on the SSH host"
    )]
    pub remote_socket: Option<String>,

    #[arg(
        long = "remote-binary",
        global = true,
        help = "Remote Boo binary path on the SSH host"
    )]
    pub remote_binary: Option<String>,

    #[arg(
        long = "remote-port",
        global = true,
        help = "Start the Boo-native TCP remote daemon on this port"
    )]
    pub remote_port: Option<u16>,

    #[arg(
        long = "remote-bind-address",
        global = true,
        help = "Bind address for the Boo-native TCP remote daemon; authless daemons default to 127.0.0.1"
    )]
    pub remote_bind_address: Option<String>,

    #[arg(
        long = "remote-auth-key",
        global = true,
        help = "Shared secret for the Boo-native TCP remote daemon"
    )]
    pub remote_auth_key: Option<String>,

    #[arg(
        long = "remote-allow-insecure-no-auth",
        global = true,
        help = "Allow a Boo-native TCP remote daemon to bind publicly without --remote-auth-key"
    )]
    pub remote_allow_insecure_no_auth: bool,

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
    /// Probe a Boo-native TCP remote daemon directly
    ProbeRemoteDaemon {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long)]
        port: u16,
        #[arg(long = "auth-key")]
        auth_key: Option<String>,
        #[arg(long = "expect-server-identity")]
        expect_server_identity: Option<String>,
    },
    /// List sessions from a Boo-native TCP remote daemon directly
    RemoteDaemonSessions {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long)]
        port: u16,
        #[arg(long = "auth-key")]
        auth_key: Option<String>,
        #[arg(long = "expect-server-identity")]
        expect_server_identity: Option<String>,
    },
    /// Attach to a session on a Boo-native TCP remote daemon directly
    RemoteDaemonAttach {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long)]
        port: u16,
        #[arg(long = "session-id")]
        session_id: u32,
        #[arg(long = "auth-key")]
        auth_key: Option<String>,
        #[arg(long = "expect-server-identity")]
        expect_server_identity: Option<String>,
        #[arg(long = "attachment-id")]
        attachment_id: Option<u64>,
        #[arg(long = "resume-token")]
        resume_token: Option<u64>,
    },
    /// Show connected remote and local-stream client diagnostics
    RemoteClients,
    /// Create a new live session
    NewSession,
    #[command(hide = true)]
    Ping,
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
    F: FnMut(&str, &config::Config) -> bool,
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

    let mut require_server = || {
        if ensure_server_running(&socket_path, boo_config) {
            true
        } else {
            eprintln!("failed to ensure boo server is running at {socket_path}");
            false
        }
    };

    match command {
        Command::Ls => {
            if !require_server() {
                return Outcome::Exit(1);
            }
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
        Command::RemoteClients => {
            if !require_server() {
                return Outcome::Exit(1);
            }
            let client = control::Client::connect(socket_path);
            match client.get_remote_clients() {
                Ok(snapshot) => {
                    let mut stdout = std::io::stdout().lock();
                    use std::io::Write;
                    if serde_json::to_writer_pretty(&mut stdout, &snapshot).is_err() {
                        eprintln!("failed to serialize remote client diagnostics");
                        return Outcome::Exit(1);
                    }
                    let _ = writeln!(stdout);
                    let _ = stdout.flush();
                    Outcome::Exit(0)
                }
                Err(error) => {
                    eprintln!("{error}");
                    Outcome::Exit(1)
                }
            }
        }
        Command::ProbeRemoteDaemon {
            host,
            port,
            auth_key,
            expect_server_identity,
        } => match crate::remote::probe_remote_endpoint(
            host,
            *port,
            auth_key.as_deref(),
            expect_server_identity.as_deref(),
        ) {
            Ok(summary) => {
                let mut stdout = std::io::stdout().lock();
                use std::io::Write;
                if serde_json::to_writer_pretty(&mut stdout, &summary).is_err() {
                    eprintln!("failed to serialize remote daemon probe summary");
                    return Outcome::Exit(1);
                }
                let _ = writeln!(stdout);
                let _ = stdout.flush();
                Outcome::Exit(0)
            }
            Err(error) => {
                eprintln!("{error}");
                Outcome::Exit(1)
            }
        },
        Command::RemoteDaemonSessions {
            host,
            port,
            auth_key,
            expect_server_identity,
        } => match crate::remote::list_remote_daemon_sessions(
            host,
            *port,
            auth_key.as_deref(),
            expect_server_identity.as_deref(),
        ) {
            Ok(summary) => {
                let mut stdout = std::io::stdout().lock();
                use std::io::Write;
                if serde_json::to_writer_pretty(&mut stdout, &summary).is_err() {
                    eprintln!("failed to serialize remote daemon session summary");
                    return Outcome::Exit(1);
                }
                let _ = writeln!(stdout);
                let _ = stdout.flush();
                Outcome::Exit(0)
            }
            Err(error) => {
                eprintln!("{error}");
                Outcome::Exit(1)
            }
        },
        Command::RemoteDaemonAttach {
            host,
            port,
            session_id,
            auth_key,
            expect_server_identity,
            attachment_id,
            resume_token,
        } => match crate::remote::attach_remote_daemon_session(
            host,
            *port,
            auth_key.as_deref(),
            expect_server_identity.as_deref(),
            *session_id,
            *attachment_id,
            *resume_token,
        ) {
            Ok(summary) => {
                let mut stdout = std::io::stdout().lock();
                use std::io::Write;
                if serde_json::to_writer_pretty(&mut stdout, &summary).is_err() {
                    eprintln!("failed to serialize remote daemon attach summary");
                    return Outcome::Exit(1);
                }
                let _ = writeln!(stdout);
                let _ = stdout.flush();
                Outcome::Exit(0)
            }
            Err(error) => {
                eprintln!("{error}");
                Outcome::Exit(1)
            }
        },
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
            if !require_server() {
                return Outcome::Exit(1);
            }
            let client = control::Client::connect(socket_path);
            if let Err(error) = client.send(&control::Request::Quit) {
                eprintln!("{error}");
                return Outcome::Exit(1);
            }
            Outcome::Exit(0)
        }
        Command::Ping => {
            let client = control::Client::connect(socket_path);
            match client.ping() {
                Ok(()) => Outcome::Exit(0),
                Err(error) => {
                    eprintln!("{error}");
                    Outcome::Exit(1)
                }
            }
        }
        Command::NewSession => {
            if !require_server() {
                return Outcome::Exit(1);
            }
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
    fn parse_remote_clients_subcommand() {
        let cli = Cli::parse_from(["boo", "remote-clients"]);
        assert!(matches!(cli.command, Some(super::Command::RemoteClients)));
    }

    #[test]
    fn parse_probe_remote_daemon_subcommand() {
        let cli = Cli::parse_from([
            "boo",
            "probe-remote-daemon",
            "--host",
            "127.0.0.1",
            "--port",
            "7337",
            "--auth-key",
            "secret",
            "--expect-server-identity",
            "daemon-01",
        ]);
        match cli.command {
            Some(super::Command::ProbeRemoteDaemon {
                host,
                port,
                auth_key,
                expect_server_identity,
            }) => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, 7337);
                assert_eq!(auth_key.as_deref(), Some("secret"));
                assert_eq!(expect_server_identity.as_deref(), Some("daemon-01"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_remote_daemon_sessions_subcommand() {
        let cli = Cli::parse_from([
            "boo",
            "remote-daemon-sessions",
            "--host",
            "127.0.0.1",
            "--port",
            "7337",
            "--auth-key",
            "secret",
            "--expect-server-identity",
            "daemon-01",
        ]);
        match cli.command {
            Some(super::Command::RemoteDaemonSessions {
                host,
                port,
                auth_key,
                expect_server_identity,
            }) => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, 7337);
                assert_eq!(auth_key.as_deref(), Some("secret"));
                assert_eq!(expect_server_identity.as_deref(), Some("daemon-01"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_remote_daemon_attach_subcommand() {
        let cli = Cli::parse_from([
            "boo",
            "remote-daemon-attach",
            "--host",
            "127.0.0.1",
            "--port",
            "7337",
            "--session-id",
            "42",
            "--auth-key",
            "secret",
            "--expect-server-identity",
            "daemon-01",
            "--attachment-id",
            "99",
            "--resume-token",
            "1234",
        ]);
        match cli.command {
            Some(super::Command::RemoteDaemonAttach {
                host,
                port,
                session_id,
                auth_key,
                expect_server_identity,
                attachment_id,
                resume_token,
            }) => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, 7337);
                assert_eq!(session_id, 42);
                assert_eq!(auth_key.as_deref(), Some("secret"));
                assert_eq!(expect_server_identity.as_deref(), Some("daemon-01"));
                assert_eq!(attachment_id, Some(99));
                assert_eq!(resume_token, Some(1234));
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

    #[test]
    fn version_flag_is_supported() {
        let version = Cli::try_parse_from(["boo", "--version"]).unwrap_err();
        assert_eq!(version.kind(), ErrorKind::DisplayVersion);
        assert!(version.to_string().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn long_help_describes_remote_ssh_flags() {
        let mut command = Cli::command();
        let long = command.render_long_help().to_string();

        assert!(long.contains("Connect through SSH to a remote Boo host"));
        assert!(long.contains("local forwarded socket"));
        assert!(long.contains("Remote Boo control socket path on the SSH host"));
        assert!(long.contains("Boo-native TCP remote daemon"));
        assert!(long.contains("authless daemons default to 127.0.0.1"));
        assert!(long.contains("bind publicly without --remote-auth-key"));
    }

    #[test]
    fn command_exits_when_server_bootstrap_fails() {
        let cli = Cli::parse_from(["boo", "remote-clients"]);
        let config = crate::config::Config::default();
        let mut ensured = 0;
        let outcome = super::handle_command(&cli, &config, |_, _| {
            ensured += 1;
            false
        });
        assert_eq!(ensured, 1);
        assert!(matches!(outcome, super::Outcome::Exit(1)));
    }
}
