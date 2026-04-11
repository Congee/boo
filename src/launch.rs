use crate::client_gui;
use crate::config;
use crate::control;
use iced_graphics::text as graphics_text;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Mutex;
use unicode_script::Script;

static STARTUP_SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_CONTROL_SOCKET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static STARTUP_REMOTE_PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
static STARTUP_REMOTE_AUTH_KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn parse_startup_args(args: &[String]) -> bool {
    if let Some(pos) = args.iter().position(|arg| arg == "--session") {
        if let Some(name) = args.get(pos + 1) {
            STARTUP_SESSION.set(name.clone()).ok();
        }
    }
    if let Some(pos) = args.iter().position(|arg| arg == "--socket") {
        if let Some(path) = args.get(pos + 1) {
            STARTUP_CONTROL_SOCKET.set(path.clone()).ok();
        }
    }
    if let Some(pos) = args.iter().position(|arg| arg == "--remote-port") {
        if let Some(port) = args
            .get(pos + 1)
            .and_then(|value| value.parse::<u16>().ok())
        {
            STARTUP_REMOTE_PORT.set(port).ok();
        }
    }
    if let Some(pos) = args.iter().position(|arg| arg == "--remote-auth-key") {
        if let Some(key) = args.get(pos + 1) {
            STARTUP_REMOTE_AUTH_KEY.set(key.clone()).ok();
        }
    }
    args.get(1).is_some_and(|arg| arg == "server")
}

pub fn startup_session() -> Option<&'static str> {
    STARTUP_SESSION.get().map(String::as_str)
}

pub fn load_startup_config() -> config::Config {
    let mut boo_config = config::Config::load();
    if let Some(socket_path) = STARTUP_CONTROL_SOCKET.get() {
        boo_config.control_socket = Some(socket_path.clone());
    }
    if let Some(port) = STARTUP_REMOTE_PORT.get() {
        boo_config.remote_port = Some(*port);
    }
    if let Some(key) = STARTUP_REMOTE_AUTH_KEY.get() {
        boo_config.remote_auth_key = Some(key.clone());
    }
    if boo_config.control_socket.is_none() {
        boo_config.control_socket = Some(control::default_socket_path());
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

#[cfg(test)]
mod tests {
    use super::merge_family_lists;

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
}

pub fn ensure_server_running(socket_path: &str, boo_config: &config::Config) {
    let client = control::Client::connect(socket_path.to_string());
    if client.ping().is_ok() {
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
        if client.ping().is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    log::warn!("boo server did not become ready at {socket_path}");
}
