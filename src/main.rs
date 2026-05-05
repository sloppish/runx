//! Runx is a native-feeling macOS launcher implemented as a single binary crate.
//!
//! The crate is intentionally split into focused internal modules so that the
//! generated documentation mirrors the runtime architecture:
//!
//! - [`launcher`] integrates Tao/Wry, the async runtime, providers, and actions.
//! - [`state`] owns pure launcher and search-session transitions.
//! - [`providers`] contains the search sources and worker fan-out layer.
//! - [`plugins`] hosts the Lua plugin runtime and command routing.
//! - [`macos`] isolates AppKit, Accessibility, Quartz, and related system integrations.
//! - [`types`] defines the shared events and data models passed between modules.
//!
//! `main.rs` itself stays deliberately small so that `cargo doc` starts from a
//! high-level map of the codebase and then points you at the specialized
//! modules.
#![deny(rustdoc::broken_intra_doc_links)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod actions;
mod assets;
mod icons;
mod launcher;
mod logging;
mod macos;
mod plugins;
mod providers;
mod scoring;
mod state;
mod tray;
mod types;

use anyhow::Result;
use global_hotkey::GlobalHotKeyEvent;
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
};
use types::AppEvent;

use crate::launcher::Launcher;

pub use runx::{config, displays, ui};

/// Starts the launcher process and reports any fatal startup error to stderr.
fn main() {
    logging::install_panic_hook();
    if let Err(error) = run() {
        logging::fatal_error("fatal startup error", &error);
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

/// Builds the Tao event loop and hands control over to the native event loop.
fn run() -> Result<()> {
    if launcher::is_settings_app_invocation() {
        return launcher::run_settings_app();
    }

    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    event_loop.set_dock_visibility(false);

    let proxy = event_loop.create_proxy();
    let hotkey_proxy = proxy.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        let _ = hotkey_proxy.send_event(AppEvent::GlobalHotKey(event));
    }));
    let mut launcher = Launcher::bootstrap(&event_loop, proxy)?;
    launcher.start_config_reload_listener()?;

    event_loop.run(move |event, _event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Err(error) = launcher.ensure_tray() {
                    launcher.set_error(error.to_string());
                }
            }
            Event::WindowEvent {
                window_id, event, ..
            } => {
                if let Err(error) = launcher.handle_window_event(window_id, event) {
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
