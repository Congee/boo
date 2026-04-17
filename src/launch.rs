use crate::client_gui;
use crate::config;
use crate::control;
use iced_graphics::text as graphics_text;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use unicode_script::Script;

static STARTUP_SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_CONTROL_SOCKET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_REMOTE_HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_REMOTE_WORKDIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_REMOTE_SOCKET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_REMOTE_BINARY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_REMOTE_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
static STARTUP_REMOTE_AUTH_KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn parse_startup_args(cli: &crate::cli::Cli) -> bool {
    if let Some(name) = cli.global.session.as_ref() {
        STARTUP_SESSION.set(name.clone()).ok();
    }
    if let Some(path) = cli.global.socket.as_ref() {
        STARTUP_CONTROL_SOCKET.set(path.clone()).ok();
    }
    if let Some(host) = cli.global.host.as_ref() {
        STARTUP_REMOTE_HOST.set(host.clone()).ok();
    }
    if let Some(port) = cli.global.remote_port {
        STARTUP_REMOTE_PORT.set(port).ok();
    }
    if let Some(key) = cli.global.remote_auth_key.as_ref() {
        STARTUP_REMOTE_AUTH_KEY.set(key.clone()).ok();
    }
    if let Some(path) = cli.global.remote_workdir.as_ref() {
        STARTUP_REMOTE_WORKDIR.set(path.clone()).ok();
    }
    if let Some(path) = cli.global.remote_socket.as_ref() {
        STARTUP_REMOTE_SOCKET.set(path.clone()).ok();
    }
    if let Some(path) = cli.global.remote_binary.as_ref() {
        STARTUP_REMOTE_BINARY.set(path.clone()).ok();
    }
    matches!(cli.command, Some(crate::cli::Command::Server))
}

pub fn startup_session() -> Option<&'static str> {
    STARTUP_SESSION.get().map(String::as_str)
}

pub fn startup_control_socket() -> Option<&'static str> {
    STARTUP_CONTROL_SOCKET.get().map(String::as_str)
}

pub fn load_startup_config() -> config::Config {
    let mut boo_config = config::Config::load();
    let explicit_socket = STARTUP_CONTROL_SOCKET.get().cloned();
    if let Some(socket_path) = explicit_socket.as_ref() {
        boo_config.control_socket = Some(socket_path.clone());
    }
    if let Some(host) = STARTUP_REMOTE_HOST.get() {
        boo_config.remote_host = Some(host.clone());
    }
    if let Some(path) = STARTUP_REMOTE_WORKDIR.get() {
        boo_config.remote_workdir = Some(path.clone());
    }
    if let Some(path) = STARTUP_REMOTE_SOCKET.get() {
        boo_config.remote_socket = Some(path.clone());
    }
    if let Some(path) = STARTUP_REMOTE_BINARY.get() {
        boo_config.remote_binary = Some(path.clone());
    }
    if let Some(port) = STARTUP_REMOTE_PORT.get() {
        boo_config.remote_port = Some(*port);
    }
    if let Some(key) = STARTUP_REMOTE_AUTH_KEY.get() {
        boo_config.remote_auth_key = Some(key.clone());
    }
    if boo_config.remote_host.is_some() && explicit_socket.is_none() {
        let host = boo_config
            .remote_host
            .as_deref()
            .expect("remote host checked above");
        boo_config.control_socket = Some(forwarded_socket_path(host));
    } else if boo_config.control_socket.is_none() {
        boo_config.control_socket = Some(control::default_socket_path());
    }
    if let Some(socket_path) = boo_config.control_socket.as_ref() {
        STARTUP_CONTROL_SOCKET.set(socket_path.clone()).ok();
    }
    boo_config
}

pub fn run_gui_client() {
    let boo_config = load_startup_config();
    let socket_path = boo_config
        .control_socket
        .clone()
        .unwrap_or_else(control::default_socket_path);

    ensure_server_running(&socket_path, &boo_config);
    crate::platform::install_command_drag_monitor();
    install_ordered_font_fallbacks(&boo_config);

    iced::application(
        move || client_gui::ClientApp::new(socket_path.clone()),
        client_gui::ClientApp::update,
        client_gui::ClientApp::view,
    )
    .settings(iced::Settings {
        fonts: system_text_fallback_fonts(),
        vsync: boo_config.sync_to_monitor,
        ..iced::Settings::default()
    })
    .title("boo")
    .decorations(boo_config.window_decoration.shows_system_decorations())
    .transparent(true)
    .style(|state, _theme| state.window_style())
    .theme(client_gui::ClientApp::theme)
    .subscription(client_gui::ClientApp::subscription)
    .run()
    .unwrap();
    crate::profiling::flush();
}

fn install_ordered_font_fallbacks(boo_config: &config::Config) {
    let user_fallbacks = boo_config
        .font_families
        .iter()
        .skip(1)
        .map(|family| crate::leak_font_family(family))
        .collect::<Vec<_>>();

    if user_fallbacks.is_empty() {
        return;
    }

    let mut font_system = graphics_text::font_system()
        .write()
        .expect("Write font system");
    let raw = std::mem::replace(
        font_system.raw(),
        graphics_text::cosmic_text::FontSystem::new_with_locale_and_db_and_fallback(
            "en-US".to_string(),
            graphics_text::cosmic_text::fontdb::Database::new(),
            graphics_text::cosmic_text::PlatformFallback,
        ),
    );
    let (locale, db) = raw.into_locale_and_db();
    *font_system.raw() = graphics_text::cosmic_text::FontSystem::new_with_locale_and_db_and_fallback(
        locale,
        db,
        BooFontFallback::new(user_fallbacks),
    );
}

fn merge_family_lists(
    preferred: &[&'static str],
    fallback: &[&'static str],
) -> Box<[&'static str]> {
    let mut merged = Vec::with_capacity(preferred.len() + fallback.len());
    for family in preferred.iter().chain(fallback.iter()) {
        if !merged.iter().any(|existing| existing == family) {
            merged.push(*family);
        }
    }
    merged.into_boxed_slice()
}

struct BooFontFallback {
    user_fallbacks: Box<[&'static str]>,
    common_fallbacks: Box<[&'static str]>,
    platform: graphics_text::cosmic_text::PlatformFallback,
    script_fallbacks: Mutex<HashMap<Script, &'static [&'static str]>>,
}

impl BooFontFallback {
    fn new(user_fallbacks: Vec<&'static str>) -> Self {
        let platform = graphics_text::cosmic_text::PlatformFallback;
        let user_fallbacks = user_fallbacks.into_boxed_slice();
        let common_fallbacks = merge_family_lists(
            &user_fallbacks,
            graphics_text::cosmic_text::Fallback::common_fallback(&platform),
        );
        Self {
            user_fallbacks,
            common_fallbacks,
            platform,
            script_fallbacks: Mutex::new(HashMap::new()),
        }
    }
}

impl graphics_text::cosmic_text::Fallback for BooFontFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common_fallbacks
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        graphics_text::cosmic_text::Fallback::forbidden_fallback(&self.platform)
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        let mut cache = self
            .script_fallbacks
            .lock()
            .expect("lock script fallback cache");
        *cache.entry(script).or_insert_with(|| {
            let merged = merge_family_lists(
                &self.user_fallbacks,
                graphics_text::cosmic_text::Fallback::script_fallback(
                    &self.platform,
                    script,
                    locale,
                ),
            );
            Box::leak(merged)
        })
    }
}

fn system_text_fallback_fonts() -> Vec<Cow<'static, [u8]>> {
    #[cfg(target_os = "macos")]
    {
        const CANDIDATES: &[&str] = &[
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/Supplemental/Songti.ttc",
            "/System/Library/Fonts/Apple Color Emoji.ttc",
        ];

        CANDIDATES
            .iter()
            .filter_map(|path| match std::fs::read(path) {
                Ok(bytes) => {
                    if std::env::var_os("BOO_RENDER_DEBUG").is_some() {
                        eprintln!(
                            "boo_render loaded_text_fallback_font path={} bytes={}",
                            path,
                            bytes.len()
                        );
                    }
                    Some(Cow::Owned(bytes))
                }
                Err(error) => {
                    if std::env::var_os("BOO_RENDER_DEBUG").is_some() {
                        eprintln!(
                            "boo_render skipped_text_fallback_font path={} error={}",
                            path, error
                        );
                    }
                    None
                }
            })
            .collect()
    }

    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

pub fn ensure_server_running(socket_path: &str, boo_config: &config::Config) {
    if let Some(host) = boo_config.remote_host.as_deref() {
        ensure_remote_server_running(host, socket_path, boo_config);
        return;
    }

    let client = control::Client::connect(socket_path.to_string());
    if server_ui_ready(&client) {
        return;
    }

    let Ok(exe) = std::env::current_exe() else {
        log::error!("failed to locate current executable for server autostart");
        return;
    };
    let mut command = std::process::Command::new(exe);
    command.arg("server").arg("--socket").arg(socket_path);
    if let Some(port) = boo_config.remote_port {
        command.arg("--remote-port").arg(port.to_string());
    }
    if let Some(key) = boo_config.remote_auth_key.as_deref() {
        command.arg("--remote-auth-key").arg(key);
    }
    if let Some(name) = startup_session() {
        command.arg("--session").arg(name);
    }
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Err(error) = command.spawn() {
        log::error!("failed to spawn boo server: {error}");
        return;
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if server_ui_ready(&client) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    log::warn!("boo server did not become ready at {socket_path}");
}

fn ensure_remote_server_running(host: &str, local_socket_path: &str, boo_config: &config::Config) {
    let client = control::Client::connect(local_socket_path.to_string());
    if server_ui_ready(&client) {
        return;
    }

    let remote_socket_path = boo_config
        .remote_socket
        .clone()
        .unwrap_or_else(control::default_socket_path);
    if let Err(error) = bootstrap_remote_server(host, &remote_socket_path, boo_config) {
        log::error!("failed to bootstrap remote boo on {host}: {error}");
        return;
    }
    if let Err(error) = ensure_remote_tunnel(host, local_socket_path, &remote_socket_path) {
        log::error!("failed to establish SSH tunnel to {host}: {error}");
        return;
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        if server_ui_ready(&client) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    log::warn!("remote boo server did not become ready through {local_socket_path}");
}

fn bootstrap_remote_server(
    host: &str,
    remote_socket_path: &str,
    boo_config: &config::Config,
) -> Result<(), String> {
    let remote_binary = boo_config
        .remote_binary
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("boo");
    let remote_log = format!("/tmp/boo-{}.log", sanitize_for_path(host));
    let mut script_parts = vec!["set -e".to_string()];
    if let Some(workdir) = boo_config
        .remote_workdir
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        script_parts.push(format!("cd {}", shell_single_quote(workdir)));
    }
    script_parts.push(format!(
        "if [ ! -S {socket} ]; then nohup {binary} server --socket {socket} >{log} 2>&1 </dev/null & fi",
        socket = shell_single_quote(remote_socket_path),
        binary = shell_single_quote(remote_binary),
        log = shell_single_quote(&remote_log),
    ));
    let script = script_parts.join("; ");
    let remote_command = format!("sh -c {}", shell_single_quote(&script));
    let status = std::process::Command::new("ssh")
        .arg(host)
        .arg(remote_command)
        .status()
        .map_err(|error| format!("ssh bootstrap: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("ssh bootstrap exited with status {status}"))
    }
}

fn ensure_remote_tunnel(
    host: &str,
    local_socket_path: &str,
    remote_socket_path: &str,
) -> Result<(), String> {
    let control_path = forwarded_control_path(host);
    let local_stream_path = format!("{local_socket_path}.stream");
    let remote_stream_path = format!("{remote_socket_path}.stream");

    if ssh_master_healthy(host, &control_path) && Path::new(local_socket_path).exists() {
        return Ok(());
    }

    if ssh_master_healthy(host, &control_path) {
        let _ = std::process::Command::new("ssh")
            .arg("-S")
            .arg(&control_path)
            .arg("-O")
            .arg("exit")
            .arg(host)
            .status();
    }

    let _ = std::fs::remove_file(local_socket_path);
    let _ = std::fs::remove_file(&local_stream_path);
    let status = std::process::Command::new("ssh")
        .arg("-M")
        .arg("-S")
        .arg(&control_path)
        .arg("-fnNT")
        .arg("-o")
        .arg("ControlPersist=yes")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("StreamLocalBindUnlink=yes")
        .arg("-L")
        .arg(format!("{local_socket_path}:{remote_socket_path}"))
        .arg("-L")
        .arg(format!("{local_stream_path}:{remote_stream_path}"))
        .arg(host)
        .status()
        .map_err(|error| format!("ssh forward: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("ssh forward exited with status {status}"))
    }
}

fn ssh_master_healthy(host: &str, control_path: &str) -> bool {
    std::process::Command::new("ssh")
        .arg("-S")
        .arg(control_path)
        .arg("-O")
        .arg("check")
        .arg(host)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn sanitize_for_path(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "remote".to_string()
    } else {
        sanitized
    }
}

fn forwarded_socket_path(host: &str) -> String {
    format!("/tmp/boo-{}.sock", sanitize_for_path(host))
}

fn forwarded_control_path(host: &str) -> String {
    format!("/tmp/boo-{}.ssh-ctl", sanitize_for_path(host))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn server_ui_ready(client: &control::Client) -> bool {
    let Ok(snapshot) = client.get_ui_snapshot() else {
        return false;
    };
    !snapshot.tabs.is_empty()
        && !snapshot.visible_panes.is_empty()
        && snapshot.focused_pane != 0
        && (!snapshot.pane_terminals.is_empty() || snapshot.terminal.is_some())
}

#[cfg(test)]
mod tests {
    use super::{forwarded_control_path, forwarded_socket_path, merge_family_lists, sanitize_for_path};
    use crate::launch::{load_startup_config, parse_startup_args};
    use clap::Parser;

    #[test]
    fn merge_family_lists_preserves_preferred_order_and_dedups() {
        let merged = merge_family_lists(
            &["User One", "User Two"],
            &["User Two", "Platform One", "Platform Two"],
        );

        assert_eq!(
            merged.as_ref(),
            &["User One", "User Two", "Platform One", "Platform Two"]
        );
    }

    #[test]
    fn sanitize_for_path_preserves_safe_chars() {
        assert_eq!(
            sanitize_for_path("example-mbp.local"),
            "example-mbp.local"
        );
    }

    #[test]
    fn sanitize_for_path_rewrites_unsafe_chars() {
        assert_eq!(sanitize_for_path("user@host:/tmp"), "user_host__tmp");
    }

    #[test]
    fn forwarded_paths_are_host_specific() {
        assert_eq!(
            forwarded_socket_path("example-mbp.local"),
            "/tmp/boo-example-mbp.local.sock"
        );
        assert_eq!(
            forwarded_control_path("example-mbp.local"),
            "/tmp/boo-example-mbp.local.ssh-ctl"
        );
    }

    #[test]
    fn startup_config_uses_host_specific_socket_when_remote_host_is_present() {
        let cli = crate::cli::Cli::parse_from([
            "boo",
            "ls",
            "--host",
            "example-mbp.local",
        ]);
        parse_startup_args(&cli);
        let config = load_startup_config();
        assert_eq!(
            config.control_socket.as_deref(),
            Some("/tmp/boo-example-mbp.local.sock")
        );
        assert_eq!(config.remote_host.as_deref(), Some("example-mbp.local"));
    }
}
