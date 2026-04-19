use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    actions::execute_action,
    config::{self, LoadedConfig},
    debug_log,
    icons::IconCache,
    macos::{capture_frontmost_app, ensure_accessibility_trusted},
    plugins::{self, PluginExecutionContext},
    providers::ProviderSet,
    state::AppState,
    tray,
    types::{AppEvent, FrontendCommand, StatusLine},
    ui,
};
use anyhow::{Context, Result};
#[cfg(target_os = "macos")]
use tao::platform::macos::{WindowBuilderExtMacOS, WindowExtMacOS};
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event::WindowEvent,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder},
};
use tokio::runtime::{Builder, Runtime};
use wry::{WebView, WebViewBuilder};

const INITIAL_BLUR_GUARD: Duration = Duration::from_millis(350);
const RENDER_COALESCE: Duration = Duration::from_millis(16);
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(24);
const RUNX_BUNDLE_ID: &str = "dev.runx.launcher";

pub struct Launcher {
    loaded: LoadedConfig,
    runtime: Runtime,
    providers: ProviderSet,
    plugins: Arc<plugins::PluginHost>,
    proxy: EventLoopProxy<AppEvent>,
    window: Window,
    webview: WebView,
    state: AppState,
    shown_at: Option<Instant>,
    previous_app: Option<crate::macos::FrontmostApp>,
    render_scheduled: bool,
    tray: Option<tray::TrayState>,
}

fn looks_like_runx(app: &crate::macos::FrontmostApp) -> bool {
    app.bundle_id.as_deref() == Some(RUNX_BUNDLE_ID)
        || app.name.as_deref() == Some("Runx")
        || app
            .path
            .as_deref()
            .map(|path: &str| path.ends_with("/Runx.app"))
            == Some(true)
}

impl Launcher {
    pub fn bootstrap(
        event_loop: &EventLoopWindowTarget<AppEvent>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self> {
        debug_log::append(format!("launcher bootstrap pid={}", std::process::id()));
        let loaded = LoadedConfig::load()?;
        let plugin_config = loaded.config.plugin_config()?;
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to build the async runtime")?;

        let config = Arc::new(loaded.config.clone());
        let plugins = Arc::new(plugins::PluginHost::load(
            &loaded.plugin_dirs,
            plugin_config,
        ));
        let icons = Arc::new(IconCache::new()?);
        let providers = ProviderSet::new(config.clone(), plugins.clone(), icons)?;
        let window = build_window(event_loop, &config)?;
        let html = ui::html(&config.ui);
        let ipc_proxy = proxy.clone();
        let webview = WebViewBuilder::new()
            .with_transparent(true)
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
            plugins,
            proxy,
            window,
            webview,
            state: AppState::new(),
            shown_at: None,
            previous_app: None,
            render_scheduled: false,
            tray: None,
        })
    }

    pub fn hotkey(&self) -> Result<global_hotkey::hotkey::HotKey> {
        self.loaded.config.hotkey()
    }

    pub fn toggle(&mut self) -> Result<()> {
        if self.state.is_visible() {
            self.hide()?;
        } else {
            self.capture_previous_app();
            self.show()?;
        }
        Ok(())
    }

    pub fn ensure_tray(&mut self) -> Result<()> {
        if self.tray.is_none() {
            self.tray = Some(tray::TrayState::install(self.proxy.clone())?);
        }
        Ok(())
    }

    pub fn handle_window_event(&mut self, event: WindowEvent) -> Result<()> {
        match event {
            WindowEvent::CloseRequested => self.hide()?,
            WindowEvent::Focused(true) => {
                self.state.note_window_focused();
            }
            WindowEvent::Focused(false) if self.should_hide_on_blur() => {
                self.hide()?;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn handle_user_event(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::TrayToggle => self.toggle()?,
            AppEvent::TrayOpen => {
                self.capture_previous_app();
                self.show_or_focus()?;
            }
            AppEvent::Quit => std::process::exit(0),
            AppEvent::Frontend(command) => self.handle_frontend(command)?,
            AppEvent::Render => {
                self.render_scheduled = false;
                self.render()?;
            }
            AppEvent::StartSearch { token } => {
                if token == self.state.session().search_token() {
                    self.start_search_now();
                }
            }
            AppEvent::ProviderItems {
                generation,
                provider,
                items,
            } => {
                if self
                    .state
                    .session_mut()
                    .apply_provider_items(generation, provider, items)
                {
                    self.request_render(RENDER_COALESCE);
                }
            }
            AppEvent::ProviderError {
                generation,
                provider,
                message,
            } => {
                if self
                    .state
                    .session_mut()
                    .apply_provider_error(generation, &provider, message)
                {
                    self.request_render(RENDER_COALESCE);
                }
            }
            AppEvent::ActionOutcome { message, is_error } => {
                self.state.session_mut().set_status(Some(StatusLine {
                    kind: if is_error { "error" } else { "info" },
                    message,
                }));
                self.request_render(RENDER_COALESCE);
            }
        }
        Ok(())
    }

    pub fn set_error(&mut self, message: String) {
        self.state.session_mut().set_status(Some(StatusLine {
            kind: "error",
            message,
        }));
        self.request_render(RENDER_COALESCE);
    }

    fn show(&mut self) -> Result<()> {
        self.state.show();
        self.shown_at = Some(Instant::now());
        self.render()?;
        self.center_window();
        self.window.set_visible(true);
        self.window.set_focus();
        self.start_search_now();
        self.focus_input()?;
        Ok(())
    }

    fn show_or_focus(&mut self) -> Result<()> {
        if self.state.is_visible() {
            self.window.set_focus();
            self.focus_input()?;
            return Ok(());
        }

        self.show()
    }

    fn capture_previous_app(&mut self) {
        match capture_frontmost_app() {
            Ok(Some(app)) if !looks_like_runx(&app) => {
                debug_log::append(format!(
                    "capture_previous_app accepted name={:?} bundle_id={:?} path={:?}",
                    app.name, app.bundle_id, app.path
                ));
                self.previous_app = Some(app);
            }
            Ok(Some(app)) => {
                debug_log::append(format!(
                    "capture_previous_app ignored runx-like app name={:?} bundle_id={:?} path={:?}",
                    app.name, app.bundle_id, app.path
                ));
            }
            Ok(None) => {
                debug_log::append("capture_previous_app found no frontmost app");
            }
            Err(error) => {
                debug_log::append(format!("capture_previous_app error: {error:#}"));
                eprintln!("Failed to capture the frontmost app: {error:#}");
            }
        }
    }

    fn hide(&mut self) -> Result<()> {
        self.state.hide();
        self.shown_at = None;
        self.previous_app = None;
        self.window.set_visible(false);
        self.render_scheduled = false;
        self.render()?;
        Ok(())
    }

    fn should_hide_on_blur(&self) -> bool {
        self.loaded.config.window.hide_on_blur
            && self.state.is_visible()
            && self
                .shown_at
                .is_none_or(|shown_at| shown_at.elapsed() > INITIAL_BLUR_GUARD)
            && self.state.focused_since_show()
    }

    fn handle_frontend(&mut self, command: FrontendCommand) -> Result<()> {
        match command {
            FrontendCommand::Ready => self.render()?,
            FrontendCommand::QueryChanged { query } => self.schedule_search(query),
            FrontendCommand::Activate { index } => self.activate(index),
            FrontendCommand::Hide => self.hide()?,
        }
        Ok(())
    }

    fn schedule_search(&mut self, query: String) {
        let token = self.state.session_mut().set_query(query);
        let proxy = self.proxy.clone();
        self.runtime.handle().spawn(async move {
            tokio::time::sleep(SEARCH_DEBOUNCE).await;
            let _ = proxy.send_event(AppEvent::StartSearch { token });
        });
    }

    fn start_search_now(&mut self) {
        let generation = self
            .state
            .session_mut()
            .begin_search(self.providers.provider_count());
        self.providers.spawn_search(
            self.proxy.clone(),
            generation,
            self.state.session().query().to_owned(),
        );
    }

    fn activate(&mut self, index: usize) {
        let Some(item) = self.state.session().rendered_item(index).cloned() else {
            debug_log::append(format!("activate ignored missing index={index}"));
            return;
        };
        debug_log::append(format!(
            "activate index={index} title={:?} provider={} action={:?} previous_app={:?}",
            item.title, item.provider, item.action, self.previous_app
        ));
        if item.action.likely_needs_accessibility() && !ensure_accessibility_trusted(true) {
            debug_log::append("activate blocked: accessibility preflight returned false");
            self.set_error("Runx needs Accessibility permission to control other apps. Enable Runx in System Settings > Privacy & Security > Accessibility, then retry.".to_owned());
            return;
        }
        let proxy = self.proxy.clone();
        let plugins = self.plugins.clone();
        let context = PluginExecutionContext {
            previous_app: self.previous_app.clone(),
        };
        if let Err(error) = self.hide() {
            self.set_error(error.to_string());
            return;
        }
        self.runtime.handle().spawn_blocking(move || {
            let result = execute_action(&item.action, &plugins, &context);
            let (message, is_error) = match result {
                Ok(Some(message)) => (message, false),
                Ok(None) => ("Action completed".to_owned(), false),
                Err(error) => (error.to_string(), true),
            };
            debug_log::append(format!(
                "activate outcome is_error={} message={:?}",
                is_error, message
            ));
            let _ = proxy.send_event(AppEvent::ActionOutcome { message, is_error });
        });
    }

    fn render(&mut self) -> Result<()> {
        self.state
            .session_mut()
            .refresh_rendered_items(&self.loaded.config.ranking);
        let script = format!(
            "window.__RUNX_RENDER({});",
            serde_json::to_string(&self.state.session().view_state())?
        );
        self.webview
            .evaluate_script(&script)
            .context("failed to render the launcher UI")?;
        Ok(())
    }

    fn request_render(&mut self, delay: Duration) {
        if self.render_scheduled {
            return;
        }

        self.render_scheduled = true;
        let proxy = self.proxy.clone();
        self.runtime.handle().spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = proxy.send_event(AppEvent::Render);
        });
    }

    fn focus_input(&self) -> Result<()> {
        self.webview
            .evaluate_script("window.__RUNX_FOCUS && window.__RUNX_FOCUS();")
            .context("failed to focus the launcher input")
    }

    fn center_window(&self) {
        let Some(monitor) = self
            .window
            .current_monitor()
            .or_else(|| self.window.primary_monitor())
        else {
            return;
        };

        let scale = monitor.scale_factor();
        let monitor_size = monitor.size();
        let monitor_origin = monitor.position();
        let window_width = self.loaded.config.window.width * scale;
        let window_height = self.loaded.config.window.height * scale;
        let x = monitor_origin.x as f64 + (monitor_size.width as f64 - window_width) / 2.0;
        let y = monitor_origin.y as f64 + (monitor_size.height as f64 - window_height) / 3.2;

        self.window
            .set_outer_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
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
