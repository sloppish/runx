mod actions;
mod assets;
mod config;
mod icons;
mod macos;
mod plugins;
mod providers;
mod scoring;
mod state;
mod tray;
mod types;
mod ui;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use icons::IconCache;
use providers::ProviderSet;
use state::AppState;
#[cfg(target_os = "macos")]
use tao::platform::macos::{
    ActivationPolicy, EventLoopExtMacOS, WindowBuilderExtMacOS, WindowExtMacOS,
};
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Window, WindowBuilder},
};
use tokio::runtime::{Builder, Runtime};
use types::{AppEvent, FrontendCommand, StatusLine};
use wry::{WebView, WebViewBuilder};

use crate::{
    actions::execute_action, config::LoadedConfig, macos::capture_frontmost_app,
    plugins::PluginExecutionContext,
};

const INITIAL_BLUR_GUARD: Duration = Duration::from_millis(350);
const RENDER_COALESCE: Duration = Duration::from_millis(16);
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(24);

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
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

    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        event_loop.set_dock_visibility(false);
    }
    let proxy = event_loop.create_proxy();
    let mut app = LauncherApp::new(
        loaded,
        runtime,
        providers,
        plugins,
        proxy.clone(),
        build_window(&event_loop, &config)?,
        config,
        proxy,
    )?;

    let hotkey = app.loaded.config.hotkey()?;
    let hotkey_id = hotkey.id();
    let _hotkey_manager =
        GlobalHotKeyManager::new().context("failed to create the hotkey manager")?;
    _hotkey_manager
        .register(hotkey)
        .context("failed to register the global hotkey from config.toml")?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        while let Ok(global_event) = GlobalHotKeyEvent::receiver().try_recv() {
            if global_event.id == hotkey_id && global_event.state == HotKeyState::Pressed {
                if let Err(error) = app.toggle() {
                    app.set_error(error.to_string());
                }
            }
        }

        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Err(error) = app.ensure_tray() {
                    app.set_error(error.to_string());
                }
            }
            Event::WindowEvent { event, .. } => {
                if let Err(error) = app.handle_window_event(event) {
                    app.set_error(error.to_string());
                }
            }
            Event::UserEvent(message) => {
                if let Err(error) = app.handle_user_event(message) {
                    app.set_error(error.to_string());
                }
            }
            _ => {}
        }
    });
}

struct LauncherApp {
    loaded: LoadedConfig,
    runtime: Runtime,
    providers: ProviderSet,
    plugins: Arc<plugins::PluginHost>,
    proxy: EventLoopProxy<AppEvent>,
    window: Window,
    webview: WebView,
    state: AppState,
    shown_at: Option<Instant>,
    previous_app: Option<macos::FrontmostApp>,
    render_scheduled: bool,
    tray: Option<tray::TrayState>,
}

impl LauncherApp {
    fn new(
        loaded: LoadedConfig,
        runtime: Runtime,
        providers: ProviderSet,
        plugins: Arc<plugins::PluginHost>,
        proxy: EventLoopProxy<AppEvent>,
        window: Window,
        config: Arc<config::Config>,
        ipc_proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self> {
        let html = ui::html(&config.ui);
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

    fn toggle(&mut self) -> Result<()> {
        if self.state.is_visible() {
            self.hide()?;
        } else {
            self.show()?;
        }
        Ok(())
    }

    fn show(&mut self) -> Result<()> {
        self.state.show();
        self.shown_at = Some(Instant::now());
        self.previous_app = match capture_frontmost_app() {
            Ok(app) => app,
            Err(error) => {
                eprintln!("Failed to capture the frontmost app: {error:#}");
                None
            }
        };
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

    fn hide(&mut self) -> Result<()> {
        self.state.hide();
        self.shown_at = None;
        self.previous_app = None;
        self.window.set_visible(false);
        self.render_scheduled = false;
        self.render()?;
        Ok(())
    }

    fn handle_window_event(&mut self, event: WindowEvent) -> Result<()> {
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

    fn should_hide_on_blur(&self) -> bool {
        self.loaded.config.window.hide_on_blur
            && self.state.is_visible()
            && self
                .shown_at
                .is_none_or(|shown_at| shown_at.elapsed() > INITIAL_BLUR_GUARD)
            && self.state.focused_since_show()
    }

    fn handle_user_event(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::TrayToggle => self.toggle()?,
            AppEvent::TrayOpen => self.show_or_focus()?,
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

    fn ensure_tray(&mut self) -> Result<()> {
        if self.tray.is_none() {
            self.tray = Some(tray::TrayState::install(self.proxy.clone())?);
        }
        Ok(())
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
            return;
        };
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

    fn set_error(&mut self, message: String) {
        self.state.session_mut().set_status(Some(StatusLine {
            kind: "error",
            message,
        }));
        self.request_render(RENDER_COALESCE);
    }
}

fn build_window<T: 'static>(
    event_loop: &tao::event_loop::EventLoopWindowTarget<T>,
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
