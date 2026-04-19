mod actions;
mod assets;
mod config;
mod debug_log;
mod icons;
mod launcher;
mod macos;
mod plugins;
mod providers;
mod scoring;
mod state;
mod tray;
mod types;
mod ui;

use anyhow::{Context, Result};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
};
use types::AppEvent;

use crate::launcher::Launcher;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        event_loop.set_dock_visibility(false);
    }

    let proxy = event_loop.create_proxy();
    let mut launcher = Launcher::bootstrap(&event_loop, proxy)?;
    let hotkey = launcher.hotkey()?;
    let hotkey_id = hotkey.id();
    let hotkey_manager =
        GlobalHotKeyManager::new().context("failed to create the hotkey manager")?;
    hotkey_manager
        .register(hotkey)
        .context("failed to register the global hotkey from config.toml")?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        while let Ok(global_event) = GlobalHotKeyEvent::receiver().try_recv() {
            if global_event.id == hotkey_id
                && global_event.state == HotKeyState::Pressed
                && let Err(error) = launcher.toggle()
            {
                launcher.set_error(error.to_string());
            }
        }

        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Err(error) = launcher.ensure_tray() {
                    launcher.set_error(error.to_string());
                }
            }
            Event::WindowEvent { event, .. } => {
                if let Err(error) = launcher.handle_window_event(event) {
                    launcher.set_error(error.to_string());
                }
            }
            Event::UserEvent(message) => {
                if let Err(error) = launcher.handle_user_event(message) {
                    launcher.set_error(error.to_string());
                }
            }
            _ => {}
        }
    });
}
