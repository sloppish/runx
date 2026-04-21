//! Runx is a native-feeling macOS launcher implemented as a single binary crate.
//!
//! The crate is intentionally split into focused internal modules so that the
//! generated documentation mirrors the runtime architecture:
//!
//! - [`launcher`] integrates Tao/Wry, the async runtime, providers, and actions.
//! - [`state`] owns pure launcher and search-session transitions.
//! - [`providers`] contains the search sources and worker fan-out layer.
//! - [`plugins`] hosts the Lua plugin runtime and command routing.
//! - [`macos`] isolates platform-specific shell-outs and permissions.
//! - [`types`] defines the shared events and data models passed between modules.
//!
//! `main.rs` itself stays deliberately small so that `cargo doc` starts from a
//! high-level map of the codebase and then points you at the specialized
//! modules.
#![deny(rustdoc::broken_intra_doc_links)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

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

use anyhow::Result;
use global_hotkey::GlobalHotKeyEvent;
#[cfg(target_os = "macos")]
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
};
use types::AppEvent;

use crate::launcher::Launcher;

/// Starts the launcher process and reports any fatal startup error to stderr.
fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

/// Builds the Tao event loop and hands control over to the native event loop.
fn run() -> Result<()> {
    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        event_loop.set_dock_visibility(false);
    }

    let proxy = event_loop.create_proxy();
    let mut launcher = Launcher::bootstrap(&event_loop, proxy)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        while let Ok(global_event) = GlobalHotKeyEvent::receiver().try_recv() {
            if let Err(error) = launcher.handle_global_hotkey_event(global_event) {
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
