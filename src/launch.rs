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
static STARTUP_REMOTE_PREFER_NIX_PROFILE_BINARY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static STARTUP_REMOTE_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
static STARTUP_REMOTE_BIND_ADDRESS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_REMOTE_AUTH_KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_REMOTE_ALLOW_INSECURE_NO_AUTH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static STARTUP_REMOTE_CERT_PATH: std::sync::OnceLock<std::path::PathBuf> =
    std::sync::OnceLock::new();
static STARTUP_REMOTE_KEY_PATH: std::sync::OnceLock<std::path::PathBuf> =
    std::sync::OnceLock::new();

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StartupOverrides {
    session: Option<String>,
    control_socket: Option<String>,
    remote_host: Option<String>,
    remote_workdir: Option<String>,
    remote_socket: Option<String>,
    remote_binary: Option<String>,
    remote_prefer_nix_profile_binary: bool,
    remote_port: Option<u16>,
    remote_bind_address: Option<String>,
    remote_auth_key: Option<String>,
    remote_allow_insecure_no_auth: bool,
    remote_cert_path: Option<std::path::PathBuf>,
    remote_key_path: Option<std::path::PathBuf>,
}

struct ResolvedRemotePaths {
    socket_path: String,
    binary_path: String,
    workdir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemotePathPart {
    Literal(String),
    Home,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemotePathSpec {
    raw: String,
    parts: Vec<RemotePathPart>,
}

impl RemotePathSpec {
    fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Self {
            parts: parse_remote_path_parts(&raw),
            raw,
        }
    }

    fn needs_remote_home(&self) -> bool {
        self.parts.iter().any(|part| matches!(part, RemotePathPart::Home))
    }

    fn resolve(&self, remote_home: Option<&str>) -> Result<String, String> {
        if self.needs_remote_home() && remote_home.is_none() {
            return Err(format!(
                "remote path {:?} requires HOME resolution",
                self.raw
            ));
        }

        let mut resolved = String::new();
        for part in &self.parts {
            match part {
                RemotePathPart::Literal(value) => resolved.push_str(value),
                RemotePathPart::Home => resolved.push_str(
                    remote_home.expect("validated remote HOME presence before resolving"),
                ),
            }
        }
        Ok(resolved)
    }
}

fn parse_remote_path_parts(raw: &str) -> Vec<RemotePathPart> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut index = 0;

    while index < raw.len() {
        let tail = &raw[index..];
        let home_match = if index == 0 && tail == "~" {
            Some(1)
        } else if index == 0 && tail.starts_with("~/") {
            Some(1)
        } else if tail.starts_with("${HOME}") {
            Some("${HOME}".len())
        } else if tail.starts_with("$HOME")
            && tail
                .chars()
                .nth("$HOME".len())
                .is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_'))
        {
            Some("$HOME".len())
        } else {
            None
        };

        if let Some(length) = home_match {
            if !literal.is_empty() {
                parts.push(RemotePathPart::Literal(std::mem::take(&mut literal)));
            }
            parts.push(RemotePathPart::Home);
            index += length;
            continue;
        }

        let mut chars = tail.chars();
        let ch = chars.next().expect("tail is non-empty while parsing path");
        literal.push(ch);
        index += ch.len_utf8();
    }

    if !literal.is_empty() {
        parts.push(RemotePathPart::Literal(literal));
    }

    if parts.is_empty() {
        parts.push(RemotePathPart::Literal(String::new()));
    }

    parts
}

const LOCAL_BOO_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long a locally-spawned `boo server` has to come up and answer a control
/// RPC before we give up and report the spawn as failed.
const LOCAL_SERVER_READY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);

/// Poll interval for the local-server readiness check.
const LOCAL_SERVER_READY_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(50);

/// How long the SSH-bootstrapped remote boo server has to become reachable
/// over the forwarded control socket before we declare the bootstrap broken.
/// Longer than the local deadline because SSH connection reuse + remote
/// process spawn adds non-trivial latency.
const REMOTE_SERVER_READY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(8);

/// Poll interval for the remote-server readiness check. Slightly longer than
/// the local one so a slow remote doesn't get hit too aggressively.
const REMOTE_SERVER_READY_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);

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
    if let Some(bind_address) = cli.global.remote_bind_address.as_ref() {
        STARTUP_REMOTE_BIND_ADDRESS
            .set(bind_address.clone())
            .ok();
    }
    if let Some(key) = cli.global.remote_auth_key.as_ref() {
        STARTUP_REMOTE_AUTH_KEY.set(key.clone()).ok();
    }
    if cli.global.remote_allow_insecure_no_auth {
        STARTUP_REMOTE_ALLOW_INSECURE_NO_AUTH.set(true).ok();
    }
    if let Some(path) = cli.global.remote_cert_path.as_ref() {
        STARTUP_REMOTE_CERT_PATH.set(path.clone()).ok();
    }
    if let Some(path) = cli.global.remote_key_path.as_ref() {
        STARTUP_REMOTE_KEY_PATH.set(path.clone()).ok();
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
    if cli.global.remote_prefer_nix_profile_binary {
        STARTUP_REMOTE_PREFER_NIX_PROFILE_BINARY.set(true).ok();
    }
    matches!(cli.command, Some(crate::cli::Command::Server))
}

fn startup_overrides() -> StartupOverrides {
    StartupOverrides {
        session: STARTUP_SESSION.get().cloned(),
        control_socket: STARTUP_CONTROL_SOCKET.get().cloned(),
        remote_host: STARTUP_REMOTE_HOST.get().cloned(),
        remote_workdir: STARTUP_REMOTE_WORKDIR.get().cloned(),
        remote_socket: STARTUP_REMOTE_SOCKET.get().cloned(),
        remote_binary: STARTUP_REMOTE_BINARY.get().cloned(),
        remote_prefer_nix_profile_binary: STARTUP_REMOTE_PREFER_NIX_PROFILE_BINARY
            .get()
            .copied()
            .unwrap_or(false),
        remote_port: STARTUP_REMOTE_PORT.get().copied(),
        remote_bind_address: STARTUP_REMOTE_BIND_ADDRESS.get().cloned(),
        remote_auth_key: STARTUP_REMOTE_AUTH_KEY.get().cloned(),
        remote_allow_insecure_no_auth: STARTUP_REMOTE_ALLOW_INSECURE_NO_AUTH
            .get()
            .copied()
            .unwrap_or(false),
        remote_cert_path: STARTUP_REMOTE_CERT_PATH.get().cloned(),
        remote_key_path: STARTUP_REMOTE_KEY_PATH.get().cloned(),
    }
}

fn apply_startup_overrides(
    mut boo_config: config::Config,
    overrides: &StartupOverrides,
) -> config::Config {
    let explicit_socket = overrides.control_socket.clone();
    if let Some(socket_path) = explicit_socket.as_ref() {
        boo_config.control_socket = Some(socket_path.clone());
    }
    if let Some(host) = overrides.remote_host.as_ref() {
        boo_config.remote_host = Some(host.clone());
    }
    if let Some(path) = overrides.remote_workdir.as_ref() {
        boo_config.remote_workdir = Some(path.clone());
    }
    if let Some(path) = overrides.remote_socket.as_ref() {
        boo_config.remote_socket = Some(path.clone());
    }
    if let Some(path) = overrides.remote_binary.as_ref() {
        boo_config.remote_binary = Some(path.clone());
    }
    if overrides.remote_prefer_nix_profile_binary {
        boo_config.remote_prefer_nix_profile_binary = true;
    }
    if let Some(port) = overrides.remote_port {
        boo_config.remote_port = Some(port);
    }
    if let Some(bind_address) = overrides.remote_bind_address.as_ref() {
        boo_config.remote_bind_address = Some(bind_address.clone());
    }
    if let Some(key) = overrides.remote_auth_key.as_ref() {
        boo_config.remote_auth_key = Some(key.clone());
    }
    if overrides.remote_allow_insecure_no_auth {
        boo_config.remote_allow_insecure_no_auth = true;
    }
    if let Some(path) = overrides.remote_cert_path.as_ref() {
        boo_config.remote_cert_path = Some(path.clone());
    }
    if let Some(path) = overrides.remote_key_path.as_ref() {
        boo_config.remote_key_path = Some(path.clone());
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
    boo_config
}

pub fn startup_session() -> Option<&'static str> {
    STARTUP_SESSION.get().map(String::as_str)
}

pub fn startup_control_socket() -> Option<&'static str> {
    STARTUP_CONTROL_SOCKET.get().map(String::as_str)
}

pub fn load_startup_config() -> config::Config {
    let boo_config = apply_startup_overrides(config::Config::load(), &startup_overrides());
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

    if !ensure_server_running(&socket_path, &boo_config) {
        return;
    }
    crate::platform::install_command_drag_monitor();
    install_ordered_font_fallbacks(&boo_config);
    let remote_host = boo_config.remote_host.clone();

    iced::application(
        move || {
            client_gui::ClientApp::new_with_remote_host(
                socket_path.clone(),
                remote_host.clone(),
            )
        },
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

pub fn ensure_server_running(socket_path: &str, boo_config: &config::Config) -> bool {
    if let Some(host) = boo_config.remote_host.as_deref() {
        return ensure_remote_server_running(host, socket_path, boo_config);
    }

    let client = control::Client::connect(socket_path.to_string());
    if server_ui_ready(&client) {
        return true;
    }

    let Ok(exe) = std::env::current_exe() else {
        log::error!("failed to locate current executable for server autostart");
        return false;
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
        return false;
    }

    let deadline = std::time::Instant::now() + LOCAL_SERVER_READY_DEADLINE;
    while std::time::Instant::now() < deadline {
        if server_ui_ready(&client) {
            return true;
        }
        std::thread::sleep(LOCAL_SERVER_READY_POLL_INTERVAL);
    }
    log::warn!("boo server did not become ready at {socket_path}");
    false
}

fn ensure_remote_server_running(
    host: &str,
    local_socket_path: &str,
    boo_config: &config::Config,
) -> bool {
    let client = control::Client::connect(local_socket_path.to_string());
    if server_ui_ready(&client) {
        return true;
    }

    let resolved_paths = match resolve_remote_paths(host, boo_config) {
        Ok(paths) => paths,
        Err(error) => {
            log::error!("failed to resolve remote paths for {host}: {error}");
            return false;
        }
    };
    log::info!(
        "remote boo target host={} local_socket={} remote_socket={} binary={} workdir={}",
        host,
        local_socket_path,
        resolved_paths.socket_path,
        resolved_paths.binary_path,
        resolved_paths.workdir.as_deref().unwrap_or("<default>")
    );
    if let Err(error) = ensure_remote_bootstrap_ready(host, &resolved_paths) {
        log::error!("failed remote bootstrap preflight on {host}: {error}");
        return false;
    }
    if let Err(error) = ensure_remote_version_compatible(host, &resolved_paths) {
        log::error!("failed to verify remote boo version on {host}: {error}");
        return false;
    }
    if let Err(error) = bootstrap_remote_server(host, &resolved_paths) {
        log::error!("failed to bootstrap remote boo on {host}: {error}");
        return false;
    }
    if let Err(error) = ensure_remote_tunnel(host, local_socket_path, &resolved_paths.socket_path) {
        log::error!("failed to establish SSH tunnel to {host}: {error}");
        return false;
    }
    if let Err(error) =
        ensure_remote_control_version_compatible(host, local_socket_path, &resolved_paths)
    {
        if remote_control_version_negotiation_unsupported(&error) {
            log::warn!(
                "remote control version negotiation failed on {host}; restarting remote server on {}",
                resolved_paths.socket_path
            );
            if let Err(restart_error) = restart_remote_server(host, &resolved_paths) {
                log::error!(
                    "failed to restart remote boo on {host} after version negotiation error: {restart_error}"
                );
                return false;
            }
            if let Err(retry_error) =
                ensure_remote_control_version_compatible(host, local_socket_path, &resolved_paths)
            {
                log::error!(
                    "failed remote control version check on {host} after restart: {retry_error}"
                );
                return false;
            }
        } else {
            log::error!("failed remote control version check on {host}: {error}");
            return false;
        }
    }

    let deadline = std::time::Instant::now() + REMOTE_SERVER_READY_DEADLINE;
    while std::time::Instant::now() < deadline {
        if server_ui_ready(&client) {
            return true;
        }
        std::thread::sleep(REMOTE_SERVER_READY_POLL_INTERVAL);
    }
    log::warn!("remote boo server did not become ready through {local_socket_path}");
    false
}

fn bootstrap_remote_server(
    host: &str,
    resolved_paths: &ResolvedRemotePaths,
) -> Result<(), String> {
    let remote_log = format!("/tmp/boo-{}.log", sanitize_for_path(host));
    let mut script_parts = vec!["set -e".to_string()];
    if let Some(workdir) = resolved_paths.workdir.as_deref() {
        script_parts.push(format!("cd {}", shell_single_quote(workdir)));
    }
    script_parts.push(format!(
        "if [ -S {socket} ]; then \
            if {binary} ping --socket {socket} >/dev/null 2>&1; then \
                exit 0; \
            fi; \
            rm -f {socket}; \
        fi; \
        if [ ! -S {socket} ]; then \
            nohup {binary} server --socket {socket} >{log} 2>&1 </dev/null & \
            pid=$!; \
            i=0; \
            while [ \"$i\" -lt 40 ]; do \
                if [ -S {socket} ]; then break; fi; \
                if ! kill -0 \"$pid\" 2>/dev/null; then break; fi; \
                i=$((i + 1)); \
                sleep 0.1; \
            done; \
            if [ ! -S {socket} ]; then \
                tail_line=$(tail -n 20 {log} 2>/dev/null | tr '\\n' ' ' | sed 's/[[:space:]]\\+/ /g' | sed 's/^ //; s/ $//'); \
                if [ -n \"$tail_line\" ]; then \
                    printf '%s\\n' {failed_prefix}\"$tail_line\"; \
                else \
                    printf '%s\\n' {failed_log}; \
                fi; \
                exit 51; \
            fi; \
        fi",
        socket = shell_single_quote(&resolved_paths.socket_path),
        binary = shell_single_quote(&resolved_paths.binary_path),
        log = shell_single_quote(&remote_log),
        failed_prefix = shell_single_quote(&format!(
            "{}start-failed:",
            REMOTE_BOOTSTRAP_MARKER
        )),
        failed_log = shell_single_quote(&format!(
            "{}start-failed:{}",
            REMOTE_BOOTSTRAP_MARKER, remote_log
        )),
    ));
    let script = script_parts.join("; ");
    let output = run_remote_shell_output(host, &script, false)
        .map_err(|error| format!("ssh bootstrap: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(classify_remote_bootstrap_failure(&output))
    }
}

fn resolve_remote_paths(
    host: &str,
    boo_config: &config::Config,
) -> Result<ResolvedRemotePaths, String> {
    let socket_spec = RemotePathSpec::new(
        boo_config
        .remote_socket
        .clone()
        .unwrap_or_else(control::default_socket_path),
    );
    let binary_spec = RemotePathSpec::new(select_remote_binary_candidate(boo_config));
    let workdir_spec = boo_config
        .remote_workdir
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(RemotePathSpec::new);

    let need_remote_home = socket_spec.needs_remote_home()
        || binary_spec.needs_remote_home()
        || workdir_spec
            .as_ref()
            .is_some_and(RemotePathSpec::needs_remote_home);
    let remote_home = if need_remote_home {
        Some(fetch_remote_home(host)?)
    } else {
        None
    };

    Ok(ResolvedRemotePaths {
        socket_path: socket_spec.resolve(remote_home.as_deref())?,
        binary_path: binary_spec.resolve(remote_home.as_deref())?,
        workdir: workdir_spec
            .as_ref()
            .map(|path| path.resolve(remote_home.as_deref()))
            .transpose()?,
    })
}

fn select_remote_binary_candidate(boo_config: &config::Config) -> String {
    if let Some(binary) = boo_config
        .remote_binary
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return binary.to_string();
    }
    if boo_config.remote_prefer_nix_profile_binary {
        return "~/.nix-profile/bin/boo".to_string();
    }
    "boo".to_string()
}

fn fetch_remote_home(host: &str) -> Result<String, String> {
    let output = run_remote_shell_output(host, "printf %s \"$HOME\"", true)
        .map_err(|error| format!("ssh remote home lookup: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ssh remote home lookup exited with status {}",
            output.status
        ));
    }
    let remote_home = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if remote_home.is_empty() {
        return Err("ssh remote home lookup returned empty HOME".to_string());
    }
    Ok(remote_home)
}

fn ensure_remote_bootstrap_ready(
    host: &str,
    resolved_paths: &ResolvedRemotePaths,
) -> Result<(), String> {
    let mut script_parts = vec!["set -e".to_string()];
    if let Some(workdir) = resolved_paths.workdir.as_deref() {
        script_parts.push(format!(
            "if [ ! -d {workdir} ]; then printf '%s\\n' {marker}; exit 41; fi",
            workdir = shell_single_quote(workdir),
            marker = shell_single_quote(&format!(
                "{}missing-workdir:{}",
                REMOTE_PREFLIGHT_MARKER, workdir
            )),
        ));
        script_parts.push(format!("cd {}", shell_single_quote(workdir)));
    }
    let socket_dir = Path::new(&resolved_paths.socket_path)
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    script_parts.push(format!(
        "if [ ! -d {socket_dir} ]; then printf '%s\\n' {missing_dir}; exit 45; fi",
        socket_dir = shell_single_quote(&socket_dir),
        missing_dir = shell_single_quote(&format!(
            "{}missing-socket-dir:{}",
            REMOTE_PREFLIGHT_MARKER, socket_dir
        )),
    ));
    script_parts.push(format!(
        "if [ ! -w {socket_dir} ]; then printf '%s\\n' {non_writable_dir}; exit 46; fi",
        socket_dir = shell_single_quote(&socket_dir),
        non_writable_dir = shell_single_quote(&format!(
            "{}non-writable-socket-dir:{}",
            REMOTE_PREFLIGHT_MARKER, socket_dir
        )),
    ));
    script_parts.push(format!(
        "if [ -e {socket} ] && [ ! -S {socket} ]; then printf '%s\\n' {conflict}; exit 47; fi",
        socket = shell_single_quote(&resolved_paths.socket_path),
        conflict = shell_single_quote(&format!(
            "{}conflicting-socket-path:{}",
            REMOTE_PREFLIGHT_MARKER, resolved_paths.socket_path
        )),
    ));
    if remote_binary_looks_like_path(&resolved_paths.binary_path) {
        script_parts.push(format!(
            "if [ ! -e {binary} ]; then printf '%s\\n' {missing}; exit 42; fi",
            binary = shell_single_quote(&resolved_paths.binary_path),
            missing = shell_single_quote(&format!(
                "{}missing-binary:{}",
                REMOTE_PREFLIGHT_MARKER, resolved_paths.binary_path
            )),
        ));
        script_parts.push(format!(
            "if [ ! -x {binary} ]; then printf '%s\\n' {not_exec}; exit 43; fi",
            binary = shell_single_quote(&resolved_paths.binary_path),
            not_exec = shell_single_quote(&format!(
                "{}non-executable-binary:{}",
                REMOTE_PREFLIGHT_MARKER, resolved_paths.binary_path
            )),
        ));
    } else {
        script_parts.push(format!(
            "if ! command -v {binary} >/dev/null 2>&1; then printf '%s\\n' {missing}; exit 44; fi",
            binary = shell_single_quote(&resolved_paths.binary_path),
            missing = shell_single_quote(&format!(
                "{}missing-binary:{}",
                REMOTE_PREFLIGHT_MARKER, resolved_paths.binary_path
            )),
        ));
    }
    script_parts.push(format!(
        "printf '%s\\n' {}",
        shell_single_quote(REMOTE_PREFLIGHT_OK)
    ));
    let output = run_remote_shell_output(host, &script_parts.join("; "), true)
        .map_err(|error| format!("ssh bootstrap preflight: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(classify_remote_preflight_failure(&output))
}

fn ensure_remote_version_compatible(
    host: &str,
    resolved_paths: &ResolvedRemotePaths,
) -> Result<(), String> {
    let remote_version = fetch_remote_boo_version(host, resolved_paths)?;
    if remote_version == LOCAL_BOO_VERSION {
        return Ok(());
    }
    Err(format_remote_version_mismatch(
        host,
        &resolved_paths.binary_path,
        &remote_version,
    ))
}

fn ensure_remote_control_version_compatible(
    host: &str,
    local_socket_path: &str,
    resolved_paths: &ResolvedRemotePaths,
) -> Result<(), String> {
    let client = control::Client::connect(local_socket_path);
    let remote_version = match client.get_version() {
        Ok(version) => version,
        Err(error)
            if error.contains("unknown variant")
                || error.contains("parse error")
                || error.contains("unexpected response") =>
        {
            return Err(format!(
                "remote boo server on {host} does not support control version negotiation; rebuild the remote binary at {} so both sides match",
                resolved_paths.binary_path
            ));
        }
        Err(error) => return Err(format!("control get-version failed: {error}")),
    };
    if remote_version == LOCAL_BOO_VERSION {
        return Ok(());
    }
    Err(format_remote_version_mismatch(
        host,
        &resolved_paths.binary_path,
        &remote_version,
    ))
}

fn remote_control_version_negotiation_unsupported(error: &str) -> bool {
    error.contains("does not support control version negotiation")
}

fn restart_remote_server(host: &str, resolved_paths: &ResolvedRemotePaths) -> Result<(), String> {
    let remote_log = format!("/tmp/boo-{}-restart.log", sanitize_for_path(host));
    let stream_socket = format!("{}.stream", resolved_paths.socket_path);
    let mut script_parts = vec!["set -e".to_string()];
    if let Some(workdir) = resolved_paths.workdir.as_deref() {
        script_parts.push(format!("cd {}", shell_single_quote(workdir)));
    }
    script_parts.push(format!(
        "if [ -S {socket} ]; then \
            {binary} quit-server --socket {socket} >/dev/null 2>&1 || true; \
            i=0; \
            while [ \"$i\" -lt 20 ]; do \
                if [ ! -S {socket} ]; then break; fi; \
                i=$((i + 1)); \
                sleep 0.1; \
            done; \
        fi; \
        rm -f {socket} {stream_socket}; \
        nohup {binary} server --socket {socket} >{log} 2>&1 </dev/null & \
        pid=$!; \
        i=0; \
        while [ \"$i\" -lt 40 ]; do \
            if [ -S {socket} ]; then break; fi; \
            if ! kill -0 \"$pid\" 2>/dev/null; then break; fi; \
            i=$((i + 1)); \
            sleep 0.1; \
        done; \
        if [ ! -S {socket} ]; then \
            tail_line=$(tail -n 20 {log} 2>/dev/null | tr '\\n' ' ' | sed 's/[[:space:]]\\+/ /g' | sed 's/^ //; s/ $//'); \
            if [ -n \"$tail_line\" ]; then \
                printf '%s\\n' {failed_prefix}\"$tail_line\"; \
            else \
                printf '%s\\n' {failed_log}; \
            fi; \
            exit 51; \
        fi",
        socket = shell_single_quote(&resolved_paths.socket_path),
        stream_socket = shell_single_quote(&stream_socket),
        binary = shell_single_quote(&resolved_paths.binary_path),
        log = shell_single_quote(&remote_log),
        failed_prefix = shell_single_quote(&format!(
            "{}start-failed:",
            REMOTE_BOOTSTRAP_MARKER
        )),
        failed_log = shell_single_quote(&format!(
            "{}start-failed:{}",
            REMOTE_BOOTSTRAP_MARKER, remote_log
        )),
    ));
    let output = run_remote_shell_output(host, &script_parts.join("; "), false)
        .map_err(|error| format!("ssh restart: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(classify_remote_bootstrap_failure(&output))
    }
}

fn fetch_remote_boo_version(host: &str, resolved_paths: &ResolvedRemotePaths) -> Result<String, String> {
    let mut script_parts = vec!["set -e".to_string()];
    if let Some(workdir) = resolved_paths.workdir.as_deref() {
        script_parts.push(format!("cd {}", shell_single_quote(workdir)));
    }
    script_parts.push(format!(
        "{} --version",
        shell_single_quote(&resolved_paths.binary_path)
    ));
    let output = run_remote_shell_output(host, &script_parts.join("; "), true)
        .map_err(|error| format!("ssh remote version lookup: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("--version") {
            return Err(
                "remote boo binary does not support --version; rebuild the remote checkout so it matches the local Boo binary"
                    .to_string(),
            );
        }
        return Err(format!(
            "ssh remote version lookup exited with status {}: {}",
            output.status,
            stderr.trim()
        ));
    }
    parse_boo_version_output(&String::from_utf8_lossy(&output.stdout))
}

fn format_remote_version_mismatch(host: &str, remote_binary_path: &str, remote_version: &str) -> String {
    format!(
        "version mismatch: local boo {LOCAL_BOO_VERSION} vs remote boo {remote_version} on {host}; rebuild the remote binary at {remote_binary_path} so both sides match"
    )
}

const REMOTE_PREFLIGHT_MARKER: &str = "__boo_remote_preflight__:";
const REMOTE_PREFLIGHT_OK: &str = "__boo_remote_preflight__:ok";
const REMOTE_BOOTSTRAP_MARKER: &str = "__boo_remote_bootstrap__:";

fn remote_binary_looks_like_path(binary: &str) -> bool {
    binary.contains('/')
}

fn classify_remote_preflight_failure(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix(REMOTE_PREFLIGHT_MARKER) {
            if rest == "ok" {
                continue;
            }
            if let Some((kind, value)) = rest.split_once(':') {
                return match kind {
                    "missing-workdir" => format!("remote workdir does not exist: {value}"),
                    "missing-socket-dir" => {
                        format!("remote socket directory does not exist: {value}")
                    }
                    "non-writable-socket-dir" => {
                        format!("remote socket directory is not writable: {value}")
                    }
                    "conflicting-socket-path" => {
                        format!("remote socket path already exists and is not a socket: {value}")
                    }
                    "missing-binary" => format!("remote boo binary was not found: {value}"),
                    "non-executable-binary" => {
                        format!("remote boo binary is not executable: {value}")
                    }
                    _ => format!("remote bootstrap preflight failed: {rest}"),
                };
            }
            return format!("remote bootstrap preflight failed: {rest}");
        }
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!(
            "remote bootstrap preflight exited with status {}",
            output.status
        )
    } else {
        format!(
            "remote bootstrap preflight exited with status {}: {}",
            output.status, stderr
        )
    }
}

fn classify_remote_bootstrap_failure(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix(REMOTE_BOOTSTRAP_MARKER) {
            if let Some((kind, value)) = rest.split_once(':') {
                return match kind {
                    "start-failed" => format!("remote boo server failed to start: {value}"),
                    _ => format!("remote bootstrap failed: {rest}"),
                };
            }
            return format!("remote bootstrap failed: {rest}");
        }
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("ssh bootstrap exited with status {}", output.status)
    } else {
        format!(
            "ssh bootstrap exited with status {}: {}",
            output.status, stderr
        )
    }
}

fn run_remote_shell_output(
    host: &str,
    script: &str,
    login_shell: bool,
) -> Result<std::process::Output, std::io::Error> {
    let shell_flag = if login_shell { "-lc" } else { "-c" };
    let remote_command = format!("sh {shell_flag} {}", shell_single_quote(script));
    std::process::Command::new("ssh")
        .arg(host)
        .arg(remote_command)
        .output()
}

fn parse_boo_version_output(output: &str) -> Result<String, String> {
    let trimmed = output.trim();
    let version = trimmed
        .split_whitespace()
        .last()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "empty version output".to_string())?;
    Ok(version.to_string())
}

fn ensure_remote_tunnel(
    host: &str,
    local_socket_path: &str,
    remote_socket_path: &str,
) -> Result<(), String> {
    let control_path = forwarded_control_path(host);
    let local_stream_path = format!("{local_socket_path}.stream");
    let remote_stream_path = format!("{remote_socket_path}.stream");

    if remote_tunnel_healthy(
        ssh_master_healthy(host, &control_path),
        forwarded_control_socket_healthy(local_socket_path),
        forwarded_stream_socket_healthy(&local_stream_path),
    ) {
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

    cleanup_local_forwarded_sockets(local_socket_path, &local_stream_path);
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

fn remote_tunnel_healthy(
    ssh_master_healthy: bool,
    control_socket_healthy: bool,
    stream_socket_healthy: bool,
) -> bool {
    ssh_master_healthy && control_socket_healthy && stream_socket_healthy
}

fn cleanup_local_forwarded_sockets(local_socket_path: &str, local_stream_path: &str) {
    let _ = std::fs::remove_file(local_socket_path);
    let _ = std::fs::remove_file(local_stream_path);
}

fn forwarded_control_socket_healthy(local_socket_path: &str) -> bool {
    if !Path::new(local_socket_path).exists() {
        return false;
    }
    control::Client::connect(local_socket_path).ping().is_ok()
}

fn forwarded_stream_socket_healthy(local_stream_path: &str) -> bool {
    if !Path::new(local_stream_path).exists() {
        return false;
    }
    std::os::unix::net::UnixStream::connect(local_stream_path).is_ok()
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
    use super::{
        apply_startup_overrides, cleanup_local_forwarded_sockets,
        classify_remote_bootstrap_failure, classify_remote_preflight_failure,
        forwarded_control_path, forwarded_control_socket_healthy, forwarded_socket_path,
        forwarded_stream_socket_healthy, format_remote_version_mismatch, merge_family_lists,
        parse_boo_version_output, parse_remote_path_parts,
        remote_binary_looks_like_path, remote_control_version_negotiation_unsupported,
        select_remote_binary_candidate,
        remote_tunnel_healthy, sanitize_for_path, RemotePathPart, RemotePathSpec,
        StartupOverrides,
        REMOTE_BOOTSTRAP_MARKER, REMOTE_PREFLIGHT_MARKER,
    };
    use crate::launch::{load_startup_config, parse_startup_args};
    use clap::Parser;
    use std::os::unix::process::ExitStatusExt;
    use std::path::Path;

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
    fn remote_path_parts_parse_home_shorthand() {
        assert_eq!(parse_remote_path_parts("~"), vec![RemotePathPart::Home]);
        assert_eq!(
            parse_remote_path_parts("~/bin/boo"),
            vec![
                RemotePathPart::Home,
                RemotePathPart::Literal("/bin/boo".to_string()),
            ]
        );
        assert_eq!(
            parse_remote_path_parts("$HOME/bin/boo"),
            vec![
                RemotePathPart::Home,
                RemotePathPart::Literal("/bin/boo".to_string()),
            ]
        );
        assert_eq!(
            parse_remote_path_parts("${HOME}/run/boo.sock"),
            vec![
                RemotePathPart::Home,
                RemotePathPart::Literal("/run/boo.sock".to_string()),
            ]
        );
    }

    #[test]
    fn remote_path_spec_resolves_home_shorthand() {
        assert_eq!(
            RemotePathSpec::new("~").resolve(Some("/Users/example")).unwrap(),
            "/Users/example"
        );
        assert_eq!(
            RemotePathSpec::new("~/bin/boo")
                .resolve(Some("/Users/example"))
                .unwrap(),
            "/Users/example/bin/boo"
        );
        assert_eq!(
            RemotePathSpec::new("$HOME/bin/boo")
                .resolve(Some("/Users/example"))
                .unwrap(),
            "/Users/example/bin/boo"
        );
        assert_eq!(
            RemotePathSpec::new("${HOME}/run/boo.sock")
                .resolve(Some("/Users/example"))
                .unwrap(),
            "/Users/example/run/boo.sock"
        );
    }

    #[test]
    fn remote_path_spec_leaves_non_home_literals_unchanged() {
        let spec = RemotePathSpec::new("/tmp/~boo/$HOMER/run.sock");
        assert!(!spec.needs_remote_home());
        assert_eq!(
            spec.resolve(None).unwrap(),
            "/tmp/~boo/$HOMER/run.sock"
        );
    }

    #[test]
    fn remote_binary_path_detection_requires_slash() {
        assert!(!remote_binary_looks_like_path("boo"));
        assert!(!remote_binary_looks_like_path("boo-debug"));
        assert!(remote_binary_looks_like_path("./target/debug/boo"));
        assert!(remote_binary_looks_like_path("/Users/example/dev/boo/target/debug/boo"));
    }

    #[test]
    fn parse_boo_version_output_extracts_version() {
        assert_eq!(
            parse_boo_version_output("boo 0.1.0\n").unwrap(),
            "0.1.0"
        );
    }

    #[test]
    fn remote_version_mismatch_message_is_actionable() {
        let message = format_remote_version_mismatch(
            "example-mbp.local",
            "/Users/example/dev/boo/target/debug/boo",
            "0.2.0",
        );
        assert!(message.contains("version mismatch: local boo"));
        assert!(message.contains("example-mbp.local"));
        assert!(message.contains("/Users/example/dev/boo/target/debug/boo"));
        assert!(message.contains("rebuild the remote binary"));
    }

    #[test]
    fn remote_control_version_negotiation_error_is_actionable() {
        let message = format!(
            "remote boo server on {} does not support control version negotiation; rebuild the remote binary at {} so both sides match",
            "example-mbp.local",
            "/Users/example/dev/boo/target/debug/boo"
        );
        assert!(message.contains("does not support control version negotiation"));
        assert!(message.contains("example-mbp.local"));
        assert!(message.contains("/Users/example/dev/boo/target/debug/boo"));
        assert!(message.contains("rebuild the remote binary"));
    }

    #[test]
    fn remote_control_version_negotiation_detection_matches_actionable_errors() {
        assert!(remote_control_version_negotiation_unsupported(
            "remote boo server on example-mbp.local does not support control version negotiation; rebuild the remote binary at /Users/example/dev/boo/target/debug/boo so both sides match"
        ));
        assert!(!remote_control_version_negotiation_unsupported(
            "control get-version failed: connection refused"
        ));
    }

    #[test]
    fn classify_remote_preflight_failure_reports_missing_workdir() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(41 << 8),
            stdout: format!("{REMOTE_PREFLIGHT_MARKER}missing-workdir:/Users/example/dev/boo\n")
                .into_bytes(),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify_remote_preflight_failure(&output),
            "remote workdir does not exist: /Users/example/dev/boo"
        );
    }

    #[test]
    fn classify_remote_preflight_failure_reports_missing_binary() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(44 << 8),
            stdout: format!("{REMOTE_PREFLIGHT_MARKER}missing-binary:boo\n").into_bytes(),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify_remote_preflight_failure(&output),
            "remote boo binary was not found: boo"
        );
    }

    #[test]
    fn classify_remote_preflight_failure_reports_non_executable_binary() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(43 << 8),
            stdout: format!(
                "{REMOTE_PREFLIGHT_MARKER}non-executable-binary:/Users/example/dev/boo/target/debug/boo\n"
            )
            .into_bytes(),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify_remote_preflight_failure(&output),
            "remote boo binary is not executable: /Users/example/dev/boo/target/debug/boo"
        );
    }

    #[test]
    fn classify_remote_preflight_failure_reports_missing_socket_dir() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(45 << 8),
            stdout: format!("{REMOTE_PREFLIGHT_MARKER}missing-socket-dir:/missing/run\n")
                .into_bytes(),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify_remote_preflight_failure(&output),
            "remote socket directory does not exist: /missing/run"
        );
    }

    #[test]
    fn classify_remote_preflight_failure_reports_non_writable_socket_dir() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(46 << 8),
            stdout: format!(
                "{REMOTE_PREFLIGHT_MARKER}non-writable-socket-dir:/private/tmp/boo\n"
            )
            .into_bytes(),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify_remote_preflight_failure(&output),
            "remote socket directory is not writable: /private/tmp/boo"
        );
    }

    #[test]
    fn classify_remote_preflight_failure_reports_conflicting_socket_path() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(47 << 8),
            stdout: format!(
                "{REMOTE_PREFLIGHT_MARKER}conflicting-socket-path:/tmp/boo.sock\n"
            )
            .into_bytes(),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify_remote_preflight_failure(&output),
            "remote socket path already exists and is not a socket: /tmp/boo.sock"
        );
    }

    #[test]
    fn classify_remote_preflight_failure_reports_ssh_error_text() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(255 << 8),
            stdout: Vec::new(),
            stderr: b"ssh: Could not resolve hostname missing-host: Name or service not known\n"
                .to_vec(),
        };
        assert_eq!(
            classify_remote_preflight_failure(&output),
            "remote bootstrap preflight exited with status exit status: 255: ssh: Could not resolve hostname missing-host: Name or service not known"
        );
    }

    #[test]
    fn classify_remote_bootstrap_failure_reports_start_failure() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(51 << 8),
            stdout: format!(
                "{REMOTE_BOOTSTRAP_MARKER}start-failed:Permission denied while binding /tmp/boo.sock\n"
            )
            .into_bytes(),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify_remote_bootstrap_failure(&output),
            "remote boo server failed to start: Permission denied while binding /tmp/boo.sock"
        );
    }

    #[test]
    fn classify_remote_bootstrap_failure_reports_ssh_error_text() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(255 << 8),
            stdout: Vec::new(),
            stderr: b"ssh: connect to host missing-host port 22: Connection refused\n".to_vec(),
        };
        assert_eq!(
            classify_remote_bootstrap_failure(&output),
            "ssh bootstrap exited with status exit status: 255: ssh: connect to host missing-host port 22: Connection refused"
        );
    }

    #[test]
    fn forwarded_control_socket_health_is_false_for_missing_socket() {
        assert!(!forwarded_control_socket_healthy(
            "/tmp/boo-missing-control-health.sock"
        ));
    }

    #[test]
    fn forwarded_stream_socket_health_is_false_for_missing_socket() {
        assert!(!forwarded_stream_socket_healthy(
            "/tmp/boo-missing-stream-health.sock"
        ));
    }

    #[test]
    fn remote_tunnel_is_healthy_only_when_all_parts_are_healthy() {
        assert!(remote_tunnel_healthy(true, true, true));
        assert!(!remote_tunnel_healthy(false, true, true));
        assert!(!remote_tunnel_healthy(true, false, true));
        assert!(!remote_tunnel_healthy(true, true, false));
        assert!(!remote_tunnel_healthy(false, false, false));
    }

    #[test]
    fn cleanup_local_forwarded_sockets_removes_stale_files() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        );
        let control_path = format!("/tmp/boo-stale-control-{unique}.sock");
        let stream_path = format!("/tmp/boo-stale-control-{unique}.sock.stream");

        std::fs::write(&control_path, b"stale control").expect("write stale control file");
        std::fs::write(&stream_path, b"stale stream").expect("write stale stream file");
        assert!(Path::new(&control_path).exists());
        assert!(Path::new(&stream_path).exists());

        cleanup_local_forwarded_sockets(&control_path, &stream_path);

        assert!(!Path::new(&control_path).exists());
        assert!(!Path::new(&stream_path).exists());
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

    #[test]
    fn startup_overrides_cli_host_beats_config_host() {
        let mut config = crate::config::Config::default();
        config.remote_host = Some("config-host.local".to_string());

        let config = apply_startup_overrides(
            config,
            &StartupOverrides {
                remote_host: Some("cli-host.local".to_string()),
                ..StartupOverrides::default()
            },
        );

        assert_eq!(config.remote_host.as_deref(), Some("cli-host.local"));
        assert_eq!(
            config.control_socket.as_deref(),
            Some("/tmp/boo-cli-host.local.sock")
        );
    }

    #[test]
    fn startup_overrides_explicit_socket_beats_generated_remote_socket() {
        let mut config = crate::config::Config::default();
        config.remote_host = Some("config-host.local".to_string());

        let config = apply_startup_overrides(
            config,
            &StartupOverrides {
                control_socket: Some("/tmp/boo-explicit.sock".to_string()),
                remote_host: Some("cli-host.local".to_string()),
                ..StartupOverrides::default()
            },
        );

        assert_eq!(config.remote_host.as_deref(), Some("cli-host.local"));
        assert_eq!(
            config.control_socket.as_deref(),
            Some("/tmp/boo-explicit.sock")
        );
    }

    #[test]
    fn select_remote_binary_candidate_prefers_explicit_binary() {
        let mut config = crate::config::Config::default();
        config.remote_binary = Some("/opt/boo/bin/boo".to_string());
        config.remote_prefer_nix_profile_binary = true;
        assert_eq!(select_remote_binary_candidate(&config), "/opt/boo/bin/boo");
    }

    #[test]
    fn select_remote_binary_candidate_uses_nix_profile_when_enabled() {
        let mut config = crate::config::Config::default();
        config.remote_prefer_nix_profile_binary = true;
        assert_eq!(
            select_remote_binary_candidate(&config),
            "~/.nix-profile/bin/boo"
        );
    }

    #[test]
    fn startup_overrides_enable_nix_profile_remote_binary_preference() {
        let config = apply_startup_overrides(
            crate::config::Config::default(),
            &StartupOverrides {
                remote_prefer_nix_profile_binary: true,
                ..StartupOverrides::default()
            },
        );
        assert!(config.remote_prefer_nix_profile_binary);
    }
}
