//! High-level launcher orchestration.
//!
//! `Launcher` keeps the event-loop-facing control flow small by delegating the
//! major runtime responsibilities to focused helpers:
//!
//! - [`search_controller`] handles debounced queries, provider callbacks, and rendering
//! - [`window_controller`] handles native window visibility, focus, and previous-app capture
//! - [`action_runner`] executes activated results off the UI thread

mod action_runner;
mod config_reload_ipc;
mod search_controller;
mod settings_window;
mod window_controller;

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::SystemTime,
};

use crate::{
    config::{self, LoadedConfig},
    icons::IconCache,
    logging,
    macos::{self, configure_launcher_panel},
    plugins::{self, PluginExecutionContext},
    providers::ProviderSet,
    state::AppState,
    tray,
    types::{AppEvent, FrontendCommand},
    ui,
};
use action_runner::ActionRunner;
use anyhow::{Context, Result};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
#[cfg(target_os = "macos")]
use tao::platform::macos::{WindowBuilderExtMacOS, WindowExtMacOS};
use tao::{
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use tokio::runtime::{Builder, Runtime};
use tracing::{debug, error, info, warn};
use wry::http::Request;
use wry::{WebView, WebViewBuilder};

use self::{search_controller::SearchController, window_controller::WindowController};

const INITIAL_LAYOUT_VERSION: u64 = 0;
const SETTINGS_MODE_ARG: &str = "--settings";
const SETTINGS_EXECUTABLE_NAME: &str = "runx-settings";
const SETTINGS_APP_BUNDLE_NAME: &str = "Runx Settings.app";

/// Returns whether this process should run the standalone Settings app.
pub(crate) fn is_settings_app_invocation() -> bool {
    std::env::args().any(|arg| arg == SETTINGS_MODE_ARG)
        || std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .is_some_and(|name| name == SETTINGS_EXECUTABLE_NAME)
}

/// Starts the standalone Settings app process.
pub(crate) fn run_settings_app() -> Result<()> {
    settings_window::run_standalone_app()
}

/// Owns the live launcher runtime and routes native events into the smaller controllers.
///
/// In practice, this is the top-level coordinator for the running app:
///
/// - it owns the Tao window and Wry webview
/// - it wires together the dedicated search, window, and action controllers
/// - it translates Tao/frontend/provider events into those controller calls
pub struct Launcher {
    loaded: LoadedConfig,
    resolved_window_config: config::WindowConfig,
    resolved_ui_config: config::UiConfig,
    last_config_modified: Option<SystemTime>,
    hotkey_manager: GlobalHotKeyManager,
    hotkey: HotKey,
    runtime: Runtime,
    icons: Arc<IconCache>,
    providers: ProviderSet,
    actions: ActionRunner,
    search: SearchController,
    windows: WindowController,
    config_reload_error: Option<String>,
    proxy: EventLoopProxy<AppEvent>,
    window: Window,
    webview: WebView,
    state: AppState,
    tray: Option<tray::TrayState>,
}

impl Launcher {
    /// Builds the launcher from config, plugins, providers, and webview UI.
    pub fn bootstrap(
        event_loop: &EventLoopWindowTarget<AppEvent>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self> {
        let bootstrap = Self::prepare_runtime_config()?;
        let BootstrapConfig {
            loaded,
            hotkey,
            plugin_config,
            plugin_routes,
            config_error,
        } = bootstrap;
        logging::init_launcher_logging(loaded.config.debug_log);
        info!(pid = std::process::id(), "launcher bootstrap");
        if let Some(error) = config_error.as_deref() {
            warn!(error = %error, "startup config invalid; using defaults");
        }
        let last_config_modified = config::config_modified_at(&loaded.config_path);
        let hotkey_manager =
            GlobalHotKeyManager::new().context("failed to create the hotkey manager")?;
        hotkey_manager
            .register(hotkey)
            .context("failed to register the global hotkey from config.toml")?;
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to build the async runtime")?;

        let config = Arc::new(loaded.config.clone());
        let plugins = Arc::new(plugins::PluginHost::load(
            &loaded.plugin_dirs,
            &loaded.plugin_search_paths,
            plugin_config,
            plugin_routes,
        ));
        let icons = Arc::new(IconCache::new(Some(proxy.clone()))?);
        let providers = ProviderSet::new(config.clone(), plugins.clone(), icons.clone())?;
        let window = build_window(event_loop, &config)?;
        let html = ui::html(
            &config.ui,
            config.window.visible_rows,
            INITIAL_LAYOUT_VERSION,
            config.window.scale,
        );
        let ipc_proxy = proxy.clone();
        let protocol_icons = icons.clone();
        let webview = WebViewBuilder::new()
            .with_transparent(true)
            .with_custom_protocol("runx".into(), move |_id, request: Request<Vec<u8>>| {
                protocol_icons.protocol_response(&request)
            })
            .with_html(&html)
            .with_ipc_handler(move |request| {
                let payload = request.body();
                let event = serde_json::from_str::<FrontendCommand>(payload).map_or_else(
                    |error| AppEvent::ActionOutcome {
                        message: format!("UI IPC error: {error}"),
                        is_error: true,
                    },
                    AppEvent::Frontend,
                );
                let _ = ipc_proxy.send_event(event);
            })
            .build(&window)
            .context("failed to build the launcher webview")?;

        Ok(Self {
            resolved_window_config: loaded.config.window.clone(),
            resolved_ui_config: loaded.config.ui.clone(),
            loaded,
            last_config_modified,
            hotkey_manager,
            hotkey,
            runtime,
            icons,
            providers,
            actions: ActionRunner::new(plugins),
            search: SearchController::new(&config.timing),
            windows: WindowController::default(),
            config_reload_error: config_error,
            proxy,
            window,
            webview,
            state: AppState::new(),
            tray: None,
        })
    }

    /// Shows the launcher if hidden, or hides it if already visible.
    pub fn toggle(&mut self) -> Result<()> {
        if self.state.is_visible() {
            self.hide_without_focus_restore()?;
        } else {
            self.windows.capture_previous_app();
            self.show()?;
        }
        Ok(())
    }

    /// Installs the menu bar tray item on first use.
    pub fn ensure_tray(&mut self) -> Result<()> {
        if self.tray.is_none() {
            self.tray = Some(tray::TrayState::install(self.proxy.clone())?);
        }
        Ok(())
    }

    /// Listens for explicit config-reload notifications from the standalone Settings app.
    pub fn start_config_reload_listener(&self) -> Result<()> {
        config_reload_ipc::start_listener(self.proxy.clone())
    }

    /// Applies Tao `WindowEvent`s to visibility and focus state.
    pub fn handle_window_event(&mut self, window_id: WindowId, event: WindowEvent) -> Result<()> {
        if window_id != self.window.id() {
            return Ok(());
        }

        match event {
            WindowEvent::CloseRequested => self.hide_without_focus_restore()?,
            WindowEvent::Focused(true) => {
                self.windows.note_window_focused(&mut self.state);
            }
            WindowEvent::Focused(false)
                if self
                    .windows
                    .should_hide_when_inactive(&self.loaded.config.window, &self.state) =>
            {
                self.hide_without_focus_restore()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles app-specific events emitted by the frontend, tray, and providers.
    pub fn handle_user_event(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::GlobalHotKey(global_event) => {
                self.handle_global_hotkey_event(global_event)?;
            }
            AppEvent::TrayToggle => self.toggle()?,
            AppEvent::TrayOpen => {
                self.windows.capture_previous_app();
                self.show_or_focus()?;
            }
            AppEvent::TraySettings => self.open_settings_editor()?,
            AppEvent::TrayToggleAutostart => self.toggle_autostart()?,
            AppEvent::TraySponsor => {
                let _ = std::process::Command::new("open")
                    .arg("https://oplachko.nl/sponsor")
                    .spawn();
            }
            AppEvent::Quit => std::process::exit(0),
            AppEvent::Frontend(command) => self.handle_frontend(command)?,
            AppEvent::Settings(_) => {}
            AppEvent::ReloadConfig => self.reload_config_after_settings_save()?,
            AppEvent::Render => {
                self.search.flush_render(
                    &mut self.state,
                    &self.loaded.config.ranking,
                    &self.webview,
                )?;
            }
            AppEvent::IconReady => {
                if self.state.is_visible() {
                    let token = self.state.session().search_token();
                    self.search.handle_start_search(
                        &mut self.state,
                        token,
                        &self.providers,
                        self.proxy.clone(),
                    );
                }
            }
            AppEvent::StartSearch { token } => self.search.handle_start_search(
                &mut self.state,
                token,
                &self.providers,
                self.proxy.clone(),
            ),
            AppEvent::ProviderItems {
                generation,
                provider,
                items,
            } => self.search.handle_provider_items(
                &mut self.state,
                generation,
                provider,
                items,
                self.proxy.clone(),
            ),
            AppEvent::ProviderError {
                generation,
                provider,
                message,
            } => self.search.handle_provider_error(
                &mut self.state,
                generation,
                provider,
                message,
                self.proxy.clone(),
            ),
            AppEvent::ActionOutcome { message, is_error } => {
                self.log_outcome(message, is_error);
            }
        }
        Ok(())
    }

    /// Reports an error outside the UI, since the launcher no longer renders a footer/status row.
    pub fn set_error(&mut self, message: String) {
        self.log_outcome(message, true);
    }

    /// Applies a global hotkey event emitted by the `global-hotkey` crate.
    pub fn handle_global_hotkey_event(&mut self, event: GlobalHotKeyEvent) -> Result<()> {
        if event.id == self.hotkey.id() && event.state == HotKeyState::Pressed {
            self.toggle()?;
        }
        Ok(())
    }

    fn show(&mut self) -> Result<()> {
        self.reload_config_if_needed()?;
        self.providers.begin_session();
        self.refresh_display_config()?;
        let window_config = self.resolved_window_config.clone();
        let ui_config = self.resolved_ui_config.clone();
        self.apply_window_config(&window_config, &ui_config);
        self.windows.note_shown(&mut self.state);
        self.windows.show_window(&self.window);
        if let Some(message) = self.config_reload_error.clone() {
            self.state.session_mut().set_config_error(message);
            self.search
                .render(&mut self.state, &self.loaded.config.ranking, &self.webview)?;
            return Ok(());
        }

        clear_recovered_config_error(&mut self.state, &mut self.config_reload_error);
        self.search
            .render(&mut self.state, &self.loaded.config.ranking, &self.webview)?;
        let token = self.state.session().search_token();
        self.search.handle_start_search(
            &mut self.state,
            token,
            &self.providers,
            self.proxy.clone(),
        );
        self.ensure_launcher_key_focus()?;
        Ok(())
    }

    fn show_or_focus(&mut self) -> Result<()> {
        if self.state.is_visible() {
            self.windows.focus_window(&self.window);
            self.ensure_launcher_key_focus()?;
            return Ok(());
        }

        self.show()
    }

    fn hide_without_focus_restore(&mut self) -> Result<()> {
        self.providers.end_session();
        self.windows.note_hidden(&mut self.state);

        self.windows.hide_window(&self.window);
        self.search
            .render(&mut self.state, &self.loaded.config.ranking, &self.webview)?;
        Ok(())
    }

    fn handle_frontend(&mut self, command: FrontendCommand) -> Result<()> {
        match command {
            FrontendCommand::Ready => {
                self.search
                    .render(&mut self.state, &self.loaded.config.ranking, &self.webview)?;
                if self.state.is_visible() {
                    self.ensure_launcher_key_focus()?;
                }
            }
            FrontendCommand::PreferredHeight {
                height,
                layout_version,
            } => {
                if self.windows.accept_preferred_height(layout_version, height) {
                    let window_config = self.resolved_window_config.clone();
                    let ui_config = self.resolved_ui_config.clone();
                    self.apply_window_config(&window_config, &ui_config);
                }
            }
            FrontendCommand::QueryChanged { query } => self.search.handle_query_changed(
                &mut self.state,
                query,
                &self.runtime,
                self.proxy.clone(),
            ),
            FrontendCommand::Activate { index, all_windows } => self.activate(index, all_windows),
            FrontendCommand::CopyText { text } => {
                let _ = macos::copy_text_to_clipboard(&text)?;
            }
            FrontendCommand::PasteText => {
                if let Ok(text) = macos::read_clipboard_text() {
                    let script = format!(
                        "window.__RUNX_PASTE_TEXT && window.__RUNX_PASTE_TEXT({});",
                        serde_json::to_string(&text)?
                    );
                    self.webview
                        .evaluate_script(&script)
                        .context("failed to paste clipboard text into input")?;
                }
            }
            FrontendCommand::Hide => self.hide_without_focus_restore()?,
        }
        Ok(())
    }

    fn toggle_autostart(&mut self) -> Result<()> {
        let Some(enabled) = macos::toggle_current_app_login_item()? else {
            self.set_error(
                "Launch at Login is only available from the installed app bundle.".to_owned(),
            );
            return Ok(());
        };

        if let Some(tray) = &self.tray {
            tray.set_autostart_enabled(enabled);
        }
        Ok(())
    }

    fn open_settings_editor(&self) -> Result<()> {
        if let Some(path) = packaged_settings_app_path()? {
            let path = path
                .to_str()
                .context("settings app bundle path is not valid UTF-8")?;
            return macos::open_application(path);
        }

        let executable = std::env::current_exe().context("failed to resolve current executable")?;
        Command::new(executable)
            .arg(SETTINGS_MODE_ARG)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to launch the settings process")?;
        Ok(())
    }

    fn activate(&mut self, index: usize, all_windows: bool) {
        let Some(item) = self.state.session().rendered_item(index).cloned() else {
            warn!(index, "activate ignored missing rendered item");
            return;
        };
        debug!(
            index,
            all_windows,
            title = ?item.title,
            provider = %item.provider,
            action = ?item.action,
            previous_app = ?self.windows.previous_app(),
            "activate"
        );

        let context = PluginExecutionContext {
            previous_app: self.windows.previous_app(),
        };
        if let Err(error) = self.hide_without_focus_restore() {
            self.set_error(error.to_string());
            return;
        }

        self.actions.spawn(
            &self.runtime,
            self.proxy.clone(),
            item,
            all_windows,
            context,
        );
    }
}

impl Launcher {
    fn prepare_runtime_config() -> Result<BootstrapConfig> {
        match LoadedConfig::load().and_then(Self::build_bootstrap_config) {
            Ok(bootstrap) => Ok(bootstrap),
            Err(error) => {
                let fallback = Self::build_bootstrap_config(LoadedConfig::load_defaults()?)?;
                Ok(BootstrapConfig {
                    config_error: Some(error.to_string()),
                    ..fallback
                })
            }
        }
    }

    fn build_bootstrap_config(loaded: LoadedConfig) -> Result<BootstrapConfig> {
        let hotkey = loaded.config.hotkey()?;
        let plugin_config = loaded.config.plugin_config()?;
        let plugin_routes = loaded.config.plugin_routes()?;
        Ok(BootstrapConfig {
            loaded,
            hotkey,
            plugin_config,
            plugin_routes,
            config_error: None,
        })
    }

    fn reload_config_if_needed(&mut self) -> Result<()> {
        let current_modified = config::config_modified_at(&self.loaded.config_path);
        if current_modified == self.last_config_modified {
            return Ok(());
        }
        self.try_reload_config("Config reload failed; keeping previous config");
        Ok(())
    }

    fn reload_config_after_settings_save(&mut self) -> Result<()> {
        self.try_reload_config("Config reload failed after settings save");
        Ok(())
    }

    fn try_reload_config(&mut self, error_context: &str) {
        match self.reload_config() {
            Ok(()) => {
                self.last_config_modified = config::config_modified_at(&self.loaded.config_path);
                clear_recovered_config_error(&mut self.state, &mut self.config_reload_error);
            }
            Err(error) => {
                self.config_reload_error = Some(error.to_string());
                self.log_outcome(format!("{error_context}: {error:#}"), true);
            }
        }
    }

    fn reload_config(&mut self) -> Result<()> {
        let was_visible = self.state.is_visible();
        let current_query = self.state.session().query().to_owned();
        let previous_hotkey = self.hotkey;
        let previous_config = self.loaded.config.clone();

        let loaded = LoadedConfig::load()?;
        let reloaded_hotkey = loaded.config.hotkey()?;
        let plugin_config = loaded.config.plugin_config()?;
        let plugin_routes = loaded.config.plugin_routes()?;

        let config = Arc::new(loaded.config.clone());
        let plugins = Arc::new(plugins::PluginHost::load(
            &loaded.plugin_dirs,
            &loaded.plugin_search_paths,
            plugin_config,
            plugin_routes,
        ));
        let providers = ProviderSet::new(config.clone(), plugins.clone(), self.icons.clone())?;

        if reloaded_hotkey != previous_hotkey {
            self.hotkey_manager
                .unregister(previous_hotkey)
                .context("failed to unregister the previous hotkey during config reload")?;
            if let Err(error) = self.hotkey_manager.register(reloaded_hotkey) {
                let _ = self.hotkey_manager.register(previous_hotkey);
                return Err(error).context("failed to register the reloaded hotkey");
            }
            self.hotkey = reloaded_hotkey;
        }

        self.providers.end_session();
        self.providers = providers;
        self.actions = ActionRunner::new(plugins);
        self.search = SearchController::new(&config.timing);
        self.loaded = loaded;
        self.refresh_display_config()?;
        self.apply_window_config(
            &self.resolved_window_config.clone(),
            &self.resolved_ui_config.clone(),
        );

        if was_visible {
            self.providers.begin_session();
            self.state.session_mut().restart_query();
            self.state.session_mut().clear_config_error();
            let token = self.state.session_mut().set_query(current_query);
            self.search
                .render(&mut self.state, &self.loaded.config.ranking, &self.webview)?;
            self.search.handle_start_search(
                &mut self.state,
                token,
                &self.providers,
                self.proxy.clone(),
            );
            self.windows.apply_window_geometry(
                &self.window,
                &self.resolved_window_config,
                &self.resolved_ui_config,
            );
            self.ensure_launcher_key_focus()?;
        }

        macro_rules! diff_field {
            ($updated:ident, $prev:expr, $new:expr, $($field:ident => $label:expr),+ $(,)?) => {
                $(if $prev.$field != $new.$field { $updated.push($label); })+
            };
        }
        let mut updated = Vec::new();
        diff_field!(updated, previous_config, self.loaded.config,
            hotkey => "hotkey",
            window => "window",
            display_overrides => "display_overrides",
            ranking => "ranking",
            timing => "timing",
            ui => "theme",
        );
        if previous_config.plugins != self.loaded.config.plugins
            || previous_config.plugin != self.loaded.config.plugin
        {
            updated.push("plugins");
        }

        if !updated.is_empty() {
            self.log_outcome(format!("Reloaded config: {}", updated.join(", ")), false);
        }

        Ok(())
    }

    fn apply_window_config(&mut self, window: &config::WindowConfig, ui: &config::UiConfig) {
        self.windows.apply_window_geometry(&self.window, window, ui);
        self.window.set_always_on_top(window.always_on_top);
    }

    fn refresh_display_config(&mut self) -> Result<()> {
        let display = self
            .windows
            .target_display_profile(&self.window, &self.loaded.config.window);
        let (window_config, ui_config) =
            self.loaded.config.resolved_window_and_ui(display.as_ref());
        let layout_changed = self.resolved_ui_config != ui_config
            || self.resolved_window_config.visible_rows != window_config.visible_rows;

        if layout_changed {
            let layout_version = self.windows.invalidate_preferred_height();
            self.apply_frontend_config(&window_config, &ui_config, layout_version)?;
        }

        self.resolved_window_config = window_config;
        self.resolved_ui_config = ui_config;
        Ok(())
    }

    fn apply_frontend_config(
        &self,
        window: &config::WindowConfig,
        theme: &config::UiConfig,
        layout_version: u64,
    ) -> Result<()> {
        let script = frontend_config_script(window, theme, layout_version)?;
        self.webview
            .evaluate_script(&script)
            .context("failed to apply the reloaded UI theme")
    }

    fn log_outcome(&self, message: String, is_error: bool) {
        if is_error {
            eprintln!("{message}");
            error!(%message);
        } else {
            info!(%message);
        }
    }

    fn ensure_launcher_key_focus(&self) -> Result<()> {
        self.windows.focus_window(&self.window);
        self.windows.focus_input(&self.webview)?;
        debug!(
            focused = self.window.is_focused(),
            "launcher key focus after panel-native focus"
        );
        Ok(())
    }
}

fn packaged_settings_app_path() -> Result<Option<PathBuf>> {
    let executable = std::env::current_exe().context("failed to resolve current executable")?;
    Ok(settings_app_bundle_path_from_executable(&executable).filter(|path| path.is_dir()))
}

fn settings_app_bundle_path_from_executable(executable: &Path) -> Option<PathBuf> {
    let macos_dir = executable.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }

    let contents_dir = macos_dir.parent()?;
    if contents_dir.file_name()? != "Contents" {
        return None;
    }

    Some(
        contents_dir
            .join("Applications")
            .join(SETTINGS_APP_BUNDLE_NAME),
    )
}

fn frontend_config_script(
    window: &config::WindowConfig,
    theme: &config::UiConfig,
    layout_version: u64,
) -> Result<String> {
    let config = serde_json::json!({
        "css": ui::theme_css(theme, window.scale),
        "shellClass": ui::shell_class(theme),
        "cycleSelection": theme.cycle_selection,
        "focusWindowShortcut": ui::shortcut_value(&theme.shortcuts.focus_window),
        "activateAllWindowsShortcut": ui::shortcut_value(&theme.shortcuts.activate_all_windows),
        "visibleRows": window.visible_rows,
        "layoutVersion": layout_version,
    });
    Ok(format!(
        "(() => {{ const c = {}; \
         const node = document.getElementById('runx-theme'); if (node) node.textContent = c.css; \
         const shell = document.querySelector('main'); if (shell) shell.className = c.shellClass; \
         window.__RUNX_CYCLE_SELECTION__ = c.cycleSelection; \
         window.__RUNX_FOCUS_WINDOW_SHORTCUT__ = c.focusWindowShortcut; \
         window.__RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT__ = c.activateAllWindowsShortcut; \
         window.__RUNX_VISIBLE_ROWS__ = c.visibleRows; \
         window.__RUNX_LAYOUT_VERSION__ = c.layoutVersion; \
         if (window.__RUNX_REQUEST_PREFERRED_HEIGHT) window.__RUNX_REQUEST_PREFERRED_HEIGHT(); \
         }})()",
        serde_json::to_string(&config)?
    ))
}

fn clear_recovered_config_error(state: &mut AppState, config_reload_error: &mut Option<String>) {
    *config_reload_error = None;
    state.session_mut().clear_config_error();
}

struct BootstrapConfig {
    loaded: LoadedConfig,
    hotkey: HotKey,
    plugin_config: std::collections::HashMap<String, serde_json::Value>,
    plugin_routes: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    config_error: Option<String>,
}

fn build_window(
    event_loop: &EventLoopWindowTarget<AppEvent>,
    config: &Arc<config::Config>,
) -> Result<Window> {
    let (width, height) = config.window.fallback_size(&config.ui);
    let size = LogicalSize::new(width, height);
    let builder = WindowBuilder::new()
        .with_title("Runx")
        .with_visible(false)
        .with_transparent(true)
        .with_decorations(false)
        .with_resizable(false)
        .with_inner_size(size)
        .with_always_on_top(config.window.always_on_top);
    #[cfg(target_os = "macos")]
    let builder = builder
        .with_has_shadow(false)
        .with_non_activating_panel(true);

    let window = builder
        .build(event_loop)
        .context("failed to build the launcher window")?;

    #[cfg(target_os = "macos")]
    window.set_has_shadow(false);
    #[cfg(target_os = "macos")]
    configure_launcher_panel(&window);

    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::{
        clear_recovered_config_error, frontend_config_script,
        settings_app_bundle_path_from_executable,
    };
    use crate::{config, state::AppState};

    #[test]
    fn successful_reload_clears_latched_config_error_state() {
        let mut state = AppState::new();
        state.show();
        state
            .session_mut()
            .set_config_error("bad config".to_owned());
        let mut config_reload_error = Some("bad config".to_owned());

        clear_recovered_config_error(&mut state, &mut config_reload_error);

        assert!(config_reload_error.is_none());
        assert!(state.session().view_state().config_error.is_none());
    }

    #[test]
    fn reloaded_frontend_config_updates_canvas_shell_class() {
        let window = config::WindowConfig::default();
        let mut theme = config::UiConfig::default();
        theme.canvas.show = false;

        let script = frontend_config_script(&window, &theme, 3).expect("script should render");

        assert!(script.contains(r#""shellClass":"shell canvas-hidden""#));
        assert!(script.contains(r#""layoutVersion":3"#));
    }

    #[test]
    fn derives_nested_settings_app_path_from_packaged_launcher_executable() {
        let path = settings_app_bundle_path_from_executable(std::path::Path::new(
            "/Applications/Runx.app/Contents/MacOS/runx",
        ));

        assert_eq!(
            path.as_deref(),
            Some(std::path::Path::new(
                "/Applications/Runx.app/Contents/Applications/Runx Settings.app"
            ))
        );
    }
}
