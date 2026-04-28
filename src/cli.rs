use crate::config;
use crate::control;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Generator, Shell};
use serde::Serialize;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "boo",
    version = env!("CARGO_PKG_VERSION"),
    about = "Terminal multiplexer and GUI client for the Boo runtime",
    disable_help_flag = true,
    long_about = "Terminal multiplexer and GUI client for the Boo runtime.\n\nRunning `boo` with no subcommand opens the GUI client.",
    after_long_help = "Remote flags apply both before and after subcommands, for example `boo --host macbook ls` and `boo ls --host macbook`."
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RemoteUpgradeTargetSummary {
    ssh_host: String,
    upgrade_ready: bool,
    reason: Option<String>,
    selected_transport: Option<crate::remote::DirectTransportKind>,
    direct_host: Option<String>,
    port: Option<u16>,
    server_instance_id: Option<String>,
    build_id: Option<String>,
    capabilities: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RemoteUpgradeProbeCommandSummary {
    target: RemoteUpgradeTargetSummary,
    probe: crate::remote::RemoteUpgradeProbeSummary,
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
        help = "Enable built-in Boo profiling logs; equivalent to setting BOO_PROFILE=1"
    )]
    pub profiling: bool,

    #[arg(
        long = "trace-filter",
        global = true,
        value_name = "FILTER",
        help = "Set tracing filter directives using RUST_LOG syntax, for example boo::latency=info"
    )]
    pub trace_filter: Option<String>,

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
        long = "remote-prefer-nix-profile-binary",
        global = true,
        help = "Prefer ~/.nix-profile/bin/boo on the SSH host when --remote-binary is not set"
    )]
    pub remote_prefer_nix_profile_binary: bool,

    #[arg(
        long = "remote-port",
        global = true,
        help = "Start the Boo-native QUIC remote daemon on this port"
    )]
    pub remote_port: Option<u16>,

    #[arg(
        long = "remote-bind-address",
        global = true,
        help = "Bind address for the Boo-native QUIC remote daemon; defaults to 127.0.0.1"
    )]
    pub remote_bind_address: Option<String>,

    #[arg(long, global = true, help = "Saved layout to load at startup")]
    pub layout: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Connect the GUI client to a Boo runtime server
    Gui,
    /// Print shell completion scripts
    Completions {
        #[arg(value_enum, default_value_t = CompletionShell::Bash)]
        shell: CompletionShell,
    },
    /// Stop the Boo server
    KillServer,
    /// List live tabs on the Boo server
    Ls,
    /// Probe a Boo-native QUIC remote daemon directly
    ProbeRemoteDaemon {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long)]
        port: u16,
    },
    /// List tabs from a Boo-native QUIC remote daemon directly
    RemoteDaemonTabs {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long)]
        port: u16,
    },
    /// Create a tab on a Boo-native QUIC remote daemon directly
    RemoteDaemonCreate {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long)]
        port: u16,
        #[arg(long, default_value_t = 120)]
        cols: u16,
        #[arg(long, default_value_t = 40)]
        rows: u16,
    },
    /// Bootstrap a remote Boo host over SSH and report its canonical native remote endpoint
    RemoteUpgradeTarget,
    /// Bootstrap a remote Boo host over SSH, resolve its canonical native endpoint, and probe the direct transport
    RemoteUpgradeProbe,
    /// Show connected remote and local-stream client diagnostics
    RemoteClients,
    /// Create a new live tab
    NewTab,
    #[command(hide = true)]
    Ping,
    /// Stop the Boo server
    QuitServer,
    /// Run the Boo runtime server without a GUI
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

/// Transport choice resolved from daemon capabilities.
fn probe_remote_daemon_dispatch(
    host: &str,
    port: u16,
) -> Result<crate::remote::RemoteProbeSummary, String> {
    crate::remote::probe_remote_endpoint(host, port)
}

fn list_remote_daemon_tabs_dispatch(
    host: &str,
    port: u16,
) -> Result<crate::remote::RemoteTabListSummary, String> {
    crate::remote::list_remote_daemon_tabs(host, port)
}

fn create_remote_daemon_tab_dispatch(
    host: &str,
    port: u16,
    cols: u16,
    rows: u16,
) -> Result<crate::remote::RemoteCreateSummary, String> {
    crate::remote::create_remote_daemon_tab(host, port, cols, rows)
}

fn print_completions<G: Generator>(generator: G) -> Result<(), String> {
    let mut command = Cli::command();
    let mut stdout = std::io::stdout().lock();
    generate(generator, &mut command, "boo", &mut stdout);
    Ok(())
}

pub fn handle_command<F>(
    cli: &Cli,
    boo_config: &config::Config,
    mut ensure_server_running: F,
) -> Outcome
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
        } => match probe_remote_daemon_dispatch(host, *port) {
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
        Command::RemoteDaemonTabs {
            host,
            port,
        } => match list_remote_daemon_tabs_dispatch(host, *port) {
            Ok(summary) => {
                let mut stdout = std::io::stdout().lock();
                use std::io::Write;
                if serde_json::to_writer_pretty(&mut stdout, &summary).is_err() {
                    eprintln!("failed to serialize remote daemon tab summary");
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
        Command::RemoteDaemonCreate {
            host,
            port,
            cols,
            rows,
        } => match create_remote_daemon_tab_dispatch(host, *port, *cols, *rows) {
            Ok(summary) => {
                let mut stdout = std::io::stdout().lock();
                use std::io::Write;
                if serde_json::to_writer_pretty(&mut stdout, &summary).is_err() {
                    eprintln!("failed to serialize remote daemon create summary");
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
        Command::RemoteUpgradeTarget => {
            let Some(ssh_host) = cli
                .global
                .host
                .as_deref()
                .or(boo_config.remote_host.as_deref())
            else {
                eprintln!(
                    "remote upgrade target discovery requires --host or remote-host in config"
                );
                return Outcome::Exit(1);
            };
            if !require_server() {
                return Outcome::Exit(1);
            }
            let client = control::Client::connect(socket_path);
            match client.get_remote_clients() {
                Ok(snapshot) => {
                    let summary = resolve_remote_upgrade_target(ssh_host, &snapshot);
                    let mut stdout = std::io::stdout().lock();
                    use std::io::Write;
                    if serde_json::to_writer_pretty(&mut stdout, &summary).is_err() {
                        eprintln!("failed to serialize remote upgrade target summary");
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
        Command::RemoteUpgradeProbe => {
            let Some(ssh_host) = cli
                .global
                .host
                .as_deref()
                .or(boo_config.remote_host.as_deref())
            else {
                eprintln!("remote upgrade probe requires --host or remote-host in config");
                return Outcome::Exit(1);
            };
            if !require_server() {
                return Outcome::Exit(1);
            }
            let client = control::Client::connect(socket_path);
            match client.get_remote_clients() {
                Ok(snapshot) => {
                    let target = resolve_remote_upgrade_target(ssh_host, &snapshot);
                    let Some(selected_transport) = target.selected_transport else {
                        eprintln!(
                            "{}",
                            target
                                .reason
                                .clone()
                                .unwrap_or_else(|| "remote upgrade target is not ready".to_string())
                        );
                        return Outcome::Exit(1);
                    };
                    let Some(direct_host) = target.direct_host.as_deref() else {
                        eprintln!(
                            "{}",
                            target.reason.clone().unwrap_or_else(|| {
                                "remote upgrade target has no direct host".to_string()
                            })
                        );
                        return Outcome::Exit(1);
                    };
                    let Some(port) = target.port else {
                        eprintln!("remote upgrade target has no direct port");
                        return Outcome::Exit(1);
                    };
                    let probe_result = crate::remote::probe_selected_direct_transport(
                        selected_transport,
                        direct_host,
                        port,
                    );
                    match probe_result {
                        Ok(probe) => {
                            let summary = RemoteUpgradeProbeCommandSummary { target, probe };
                            let mut stdout = std::io::stdout().lock();
                            use std::io::Write;
                            if serde_json::to_writer_pretty(&mut stdout, &summary).is_err() {
                                eprintln!("failed to serialize remote upgrade probe summary");
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
        Command::NewTab => {
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
        Command::Gui => Outcome::Continue,
        Command::Server => Outcome::Continue,
    }
}

fn resolve_remote_upgrade_target(
    ssh_host: &str,
    snapshot: &crate::remote::RemoteClientsSnapshot,
) -> RemoteUpgradeTargetSummary {
    let Some(server) = snapshot.servers.first() else {
        return RemoteUpgradeTargetSummary {
            ssh_host: ssh_host.to_string(),
            upgrade_ready: false,
            reason: Some(
                "no native remote daemon is running on the forwarded Boo server".to_string(),
            ),
            selected_transport: None,
            direct_host: None,
            port: None,
            server_instance_id: None,
            build_id: None,
            capabilities: None,
        };
    };
    let Some(port) = server.port else {
        return RemoteUpgradeTargetSummary {
            ssh_host: ssh_host.to_string(),
            upgrade_ready: false,
            reason: Some(
                "forwarded Boo server does not advertise a native remote daemon port".to_string(),
            ),
            selected_transport: None,
            direct_host: None,
            port: None,
            server_instance_id: Some(server.server_instance_id.clone()),
            build_id: Some(server.build_id.clone()),
            capabilities: Some(server.capabilities),
        };
    };
    let bind_address = server.bind_address.as_deref().unwrap_or_default();
    let direct_host = match bind_address {
        "" => None,
        "0.0.0.0" | "::" => Some(ssh_host.to_string()),
        "127.0.0.1" | "localhost" | "::1" => None,
        other => Some(other.to_string()),
    };
    let transport_selection =
        crate::remote::select_direct_transport(server.capabilities, false).ok();
    let (upgrade_ready, reason) = if direct_host.is_none() {
        (
            false,
            Some(format!(
                "native remote daemon is bound to loopback ({bind_address}); it is not reachable for direct transport upgrade"
            )),
        )
    } else if transport_selection.is_none() {
        (
            false,
            Some("native remote daemon does not advertise a usable direct transport".to_string()),
        )
    } else {
        (true, None)
    };
    RemoteUpgradeTargetSummary {
        ssh_host: ssh_host.to_string(),
        upgrade_ready,
        reason,
        selected_transport: transport_selection,
        direct_host,
        port: Some(port),
        server_instance_id: Some(server.server_instance_id.clone()),
        build_id: Some(server.build_id.clone()),
        capabilities: Some(server.capabilities),
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, CompletionShell};
    use clap::{error::ErrorKind, CommandFactory, Parser};

    const DEFAULT_REMOTE_PORT_STR: &str = "7337";

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
            "--remote-prefer-nix-profile-binary",
            "ls",
        ]);
        assert_eq!(cli.global.host.as_deref(), Some("example-mbp.local"));
        assert_eq!(
            cli.global.remote_binary.as_deref(),
            Some("/Users/example/dev/boo/target/debug/boo")
        );
        assert!(cli.global.remote_prefer_nix_profile_binary);
        assert!(matches!(cli.command, Some(super::Command::Ls)));
    }

    #[test]
    fn parse_trace_filter_global_flag() {
        let cli = Cli::parse_from(["boo", "server", "--trace-filter", "boo::latency=info"]);
        assert_eq!(
            cli.global.trace_filter.as_deref(),
            Some("boo::latency=info")
        );
        assert!(matches!(cli.command, Some(super::Command::Server)));
    }

    #[test]
    fn parse_profiling_global_flag() {
        let cli = Cli::parse_from(["boo", "--profiling", "server"]);
        assert!(cli.global.profiling);
        assert!(matches!(cli.command, Some(super::Command::Server)));

        let cli = Cli::parse_from(["boo", "server", "--profiling"]);
        assert!(cli.global.profiling);
        assert!(matches!(cli.command, Some(super::Command::Server)));
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
            DEFAULT_REMOTE_PORT_STR,
        ]);
        match cli.command {
            Some(super::Command::ProbeRemoteDaemon { host, port }) => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, crate::config::DEFAULT_REMOTE_PORT);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_remote_daemon_tabs_subcommand() {
        let cli = Cli::parse_from([
            "boo",
            "remote-daemon-tabs",
            "--host",
            "127.0.0.1",
            "--port",
            DEFAULT_REMOTE_PORT_STR,
        ]);
        match cli.command {
            Some(super::Command::RemoteDaemonTabs { host, port }) => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, crate::config::DEFAULT_REMOTE_PORT);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_remote_upgrade_target_subcommand() {
        let cli = Cli::parse_from(["boo", "remote-upgrade-target"]);
        assert!(matches!(
            cli.command,
            Some(super::Command::RemoteUpgradeTarget)
        ));
    }

    #[test]
    fn parse_remote_upgrade_probe_subcommand() {
        let cli = Cli::parse_from(["boo", "remote-upgrade-probe"]);
        match cli.command {
            Some(super::Command::RemoteUpgradeProbe) => {}
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_remote_daemon_create_subcommand() {
        let cli = Cli::parse_from([
            "boo",
            "remote-daemon-create",
            "--host",
            "127.0.0.1",
            "--port",
            DEFAULT_REMOTE_PORT_STR,
            "--cols",
            "132",
            "--rows",
            "48",
        ]);
        match cli.command {
            Some(super::Command::RemoteDaemonCreate {
                host,
                port,
                cols,
                rows,
            }) => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, crate::config::DEFAULT_REMOTE_PORT);
                assert_eq!(cols, 132);
                assert_eq!(rows, 48);
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
        assert!(long.contains("Boo-native QUIC remote daemon"));
        assert!(long.contains("~/.nix-profile/bin/boo"));
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

    #[test]
    fn resolve_remote_upgrade_target_maps_public_bind_to_ssh_host() {
        let summary = super::resolve_remote_upgrade_target(
            "macbook.local",
            &crate::remote::RemoteClientsSnapshot {
                servers: vec![crate::remote::RemoteServerInfo {
                    local_socket_path: Some("/tmp/boo.sock".to_string()),
                    bind_address: Some("0.0.0.0".to_string()),
                    port: Some(crate::config::DEFAULT_REMOTE_PORT),
                    protocol_version: crate::remote::REMOTE_PROTOCOL_VERSION,
                    capabilities: crate::remote::REMOTE_CAPABILITIES,
                    build_id: env!("CARGO_PKG_VERSION").to_string(),
                    server_instance_id: "test-instance".to_string(),
                    auth_challenge_window_ms: 10_000,
                    heartbeat_window_ms: 20_000,
                    connected_clients: 0,
                    viewing_clients: 0,
                    pending_auth_clients: 0,
                }],
                clients: Vec::new(),
            },
        );
        assert!(summary.upgrade_ready);
        assert_eq!(
            summary.selected_transport,
            Some(crate::remote::DirectTransportKind::QuicDirect)
        );
        assert_eq!(summary.direct_host.as_deref(), Some("macbook.local"));
        assert_eq!(summary.port, Some(crate::config::DEFAULT_REMOTE_PORT));
    }

    #[test]
    fn resolve_remote_upgrade_target_rejects_loopback_bind() {
        let summary = super::resolve_remote_upgrade_target(
            "macbook.local",
            &crate::remote::RemoteClientsSnapshot {
                servers: vec![crate::remote::RemoteServerInfo {
                    local_socket_path: Some("/tmp/boo.sock".to_string()),
                    bind_address: Some("127.0.0.1".to_string()),
                    port: Some(crate::config::DEFAULT_REMOTE_PORT),
                    protocol_version: crate::remote::REMOTE_PROTOCOL_VERSION,
                    capabilities: crate::remote::REMOTE_CAPABILITIES,
                    build_id: env!("CARGO_PKG_VERSION").to_string(),
                    server_instance_id: "test-instance".to_string(),
                    auth_challenge_window_ms: 10_000,
                    heartbeat_window_ms: 20_000,
                    connected_clients: 0,
                    viewing_clients: 0,
                    pending_auth_clients: 0,
                }],
                clients: Vec::new(),
            },
        );
        assert!(!summary.upgrade_ready);
        assert_eq!(
            summary.selected_transport,
            Some(crate::remote::DirectTransportKind::QuicDirect)
        );
        assert_eq!(summary.direct_host, None);
        assert!(
            summary
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("loopback")),
            "unexpected reason: {:?}",
            summary.reason
        );
    }
}
