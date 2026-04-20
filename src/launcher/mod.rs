//! High-level launcher orchestration.
//!
//! `Launcher` keeps the event-loop-facing control flow small by delegating the
//! major runtime responsibilities to focused helpers:
//!
//! - [`search_controller`] handles debounced queries, provider callbacks, and rendering
//! - [`window_controller`] handles native window visibility, focus, and previous-app capture
//! - [`action_runner`] executes activated results off the UI thread

mod action_runner;
mod search_controller;
mod window_controller;

use std::sync::Arc;

use crate::{
    config::{self, LoadedConfig},
    debug_log,
    icons::IconCache,
    plugins::{self, PluginExecutionContext},
    providers::ProviderSet,
    state::AppState,
    tray,
    types::{AppEvent, FrontendCommand},
    ui,
};
use action_runner::ActionRunner;
use anyhow::{Context, Result};
#[cfg(target_os = "macos")]
use tao::platform::macos::{WindowBuilderExtMacOS, WindowExtMacOS};
use tao::{
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder},
};
use tokio::runtime::{Builder, Runtime};
use wry::http::Request;
use wry::{WebView, WebViewBuilder};

use self::{search_controller::SearchController, window_controller::WindowController};

/// Owns the live launcher runtime and routes native events into the smaller controllers.
///
/// In practice, this is the top-level coordinator for the running app:
///
/// - it owns the Tao window and Wry webview
/// - it wires together the dedicated search, window, and action controllers
/// - it translates Tao/frontend/provider events into those controller calls
pub struct Launcher {
    loaded: LoadedConfig,
    runtime: Runtime,
    providers: ProviderSet,
    actions: ActionRunner,
    search: SearchController,
    windows: WindowController,
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
        debug_log::append(format!("launcher bootstrap pid={}", std::process::id()));
        let loaded = LoadedConfig::load()?;
        let plugin_config = loaded.config.plugin_config()?;
        let plugin_routes = loaded.config.plugin_routes()?;
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
        let icons = Arc::new(IconCache::new()?);
        let providers = ProviderSet::new(config.clone(), plugins.clone(), icons.clone())?;
        let window = build_window(event_loop, &config)?;
        let html = ui::html(&config.ui);
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
                let event = serde_json::from_str::<FrontendCommand>(payload)
                    .map(AppEvent::Frontend)
                    .unwrap_or_else(|error| AppEvent::ActionOutcome {
                        message: format!("UI IPC error: {error}"),
                        is_error: true,
                    });
                let _ = ipc_proxy.send_event(event);
            })
            .build(&window)
            .context("failed to build the launcher webview")?;

        Ok(Self {
            loaded,
            runtime,
            providers,
            actions: ActionRunner::new(plugins),
            search: SearchController::default(),
            windows: WindowController::default(),
            proxy,
            window,
            webview,
            state: AppState::new(),
            tray: None,
        })
    }

    /// Returns the configured global hotkey used to toggle the launcher.
    pub fn hotkey(&self) -> Result<global_hotkey::hotkey::HotKey> {
        self.loaded.config.hotkey()
    }

    /// Shows the launcher if hidden, or hides it if already visible.
    pub fn toggle(&mut self) -> Result<()> {
        if self.state.is_visible() {
            self.hide()?;
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

    /// Applies Tao `WindowEvent`s to visibility and focus state.
    pub fn handle_window_event(&mut self, event: WindowEvent) -> Result<()> {
        match event {
            WindowEvent::CloseRequested => self.hide()?,
            WindowEvent::Focused(true) => {
                self.windows.note_window_focused(&mut self.state);
            }
            WindowEvent::Focused(false)
                if self
                    .windows
                    .should_hide_on_blur(&self.loaded.config.window, &self.state) =>
            {
                self.hide()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles app-specific events emitted by the frontend, tray, and providers.
    pub fn handle_user_event(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::TrayToggle => self.toggle()?,
            AppEvent::TrayOpen => {
                self.windows.capture_previous_app();
                self.show_or_focus()?;
            }
            AppEvent::Quit => std::process::exit(0),
            AppEvent::Frontend(command) => self.handle_frontend(command)?,
            AppEvent::Render => {
                self.search.flush_render(
                    &mut self.state,
                    &self.loaded.config.ranking,
                    &self.webview,
                )?;
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
                &self.runtime,
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
                &self.runtime,
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

    fn show(&mut self) -> Result<()> {
        self.windows.note_shown(&mut self.state);
        self.search
            .render(&mut self.state, &self.loaded.config.ranking, &self.webview)?;
        self.windows
            .center_window(&self.window, &self.loaded.config.window);
        self.windows.show_window(&self.window);
        let token = self.state.session().search_token();
        self.search.handle_start_search(
            &mut self.state,
            token,
            &self.providers,
            self.proxy.clone(),
        );
        self.windows.focus_input(&self.webview)?;
        Ok(())
    }

    fn show_or_focus(&mut self) -> Result<()> {
        if self.state.is_visible() {
            self.windows.focus_window(&self.window);
            self.windows.focus_input(&self.webview)?;
            return Ok(());
        }

        self.show()
    }

    fn hide(&mut self) -> Result<()> {
        self.windows.note_hidden(&mut self.state);
        self.search.cancel_pending_render();
        self.windows.hide_window(&self.window);
        self.search
            .render(&mut self.state, &self.loaded.config.ranking, &self.webview)?;
        Ok(())
    }

    fn handle_frontend(&mut self, command: FrontendCommand) -> Result<()> {
        match command {
            FrontendCommand::Ready => {
                self.search
                    .render(&mut self.state, &self.loaded.config.ranking, &self.webview)?
            }
            FrontendCommand::QueryChanged { query } => self.search.handle_query_changed(
                &mut self.state,
                query,
                &self.runtime,
                self.proxy.clone(),
            ),
            FrontendCommand::Activate { index } => self.activate(index),
            FrontendCommand::Hide => self.hide()?,
        }
        Ok(())
    }

    fn activate(&mut self, index: usize) {
        let Some(item) = self.state.session().rendered_item(index).cloned() else {
            debug_log::append(format!("activate ignored missing index={index}"));
            return;
        };
        debug_log::append(format!(
            "activate index={index} title={:?} provider={} action={:?} previous_app={:?}",
            item.title,
            item.provider,
            item.action,
            self.windows.previous_app()
        ));

        if let Err(message) = self.actions.preflight(&item.action) {
            debug_log::append("activate blocked: accessibility preflight returned false");
            self.set_error(message);
            return;
        }

        let context = PluginExecutionContext {
            previous_app: self.windows.previous_app(),
        };
        if let Err(error) = self.hide() {
            self.set_error(error.to_string());
            return;
        }

        self.actions
            .spawn(&self.runtime, self.proxy.clone(), item, context);
    }
}

impl Launcher {
    fn log_outcome(&self, message: String, is_error: bool) {
        if is_error {
            eprintln!("{message}");
            debug_log::append(format!("error: {message}"));
        } else {
            debug_log::append(format!("info: {message}"));
        }
    }
}

fn build_window(
    event_loop: &EventLoopWindowTarget<AppEvent>,
    config: &Arc<config::Config>,
) -> Result<Window> {
    let size = LogicalSize::new(config.window.width, config.window.height);
    let builder = WindowBuilder::new()
        .with_title("Runx")
        .with_visible(false)
        .with_transparent(true)
        .with_decorations(false)
        .with_resizable(false)
        .with_inner_size(size)
        .with_always_on_top(config.window.always_on_top);
    #[cfg(target_os = "macos")]
    let builder = builder.with_has_shadow(false);

    let window = builder
        .build(event_loop)
        .context("failed to build the launcher window")?;

    #[cfg(target_os = "macos")]
    window.set_has_shadow(false);

    Ok(window)
}
