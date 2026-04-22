//! Native window visibility and focus helpers.
//!
//! This module keeps the macOS/Tao-facing window mechanics out of the main
//! launcher coordinator: focus tracking, centering, blur handling, and
//! remembering which app was frontmost before Runx appeared.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tao::platform::macos::MonitorHandleExtMacOS;
use tao::{dpi::PhysicalPosition, window::Window};
use wry::WebView;

use crate::{
    config::{WindowConfig, WindowDisplayTarget},
    debug_log,
    macos::{FrontmostApp, capture_frontmost_app, cursor_display_location},
    state::AppState,
};

const INITIAL_BLUR_GUARD: Duration = Duration::from_millis(350);
const RUNX_BUNDLE_ID: &str = "dev.runx.launcher";

/// Tracks the native window lifecycle state that lives outside `AppState`.
#[derive(Default)]
pub(crate) struct WindowController {
    shown_at: Option<Instant>,
    previous_app: Option<FrontmostApp>,
}

impl WindowController {
    /// Captures the frontmost non-Runx application for later focus/typing actions.
    pub(crate) fn capture_previous_app(&mut self) {
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

    /// Records that the launcher has just become visible.
    pub(crate) fn note_shown(&mut self, state: &mut AppState) {
        state.show();
        self.shown_at = Some(Instant::now());
    }

    /// Records that the launcher has been hidden and clears the previous app target.
    pub(crate) fn note_hidden(&mut self, state: &mut AppState) {
        state.hide();
        self.shown_at = None;
        self.previous_app = None;
    }

    /// Forwards a native focus event into the app state.
    pub(crate) fn note_window_focused(&self, state: &mut AppState) {
        state.note_window_focused();
    }

    /// Returns whether a blur event should auto-hide the launcher window.
    pub(crate) fn should_hide_on_blur(&self, config: &WindowConfig, state: &AppState) -> bool {
        config.hide_on_blur
            && state.is_visible()
            && self
                .shown_at
                .is_none_or(|shown_at| shown_at.elapsed() > INITIAL_BLUR_GUARD)
            && state.focused_since_show()
    }

    /// Returns the app that was frontmost before Runx appeared.
    pub(crate) fn previous_app(&self) -> Option<FrontmostApp> {
        self.previous_app.clone()
    }

    /// Shows the native window.
    pub(crate) fn show_window(&self, window: &Window) {
        window.set_visible(true);
        window.set_focus();
    }

    /// Hides the native window.
    pub(crate) fn hide_window(&self, window: &Window) {
        window.set_visible(false);
    }

    /// Focuses the launcher window without changing its visibility state.
    pub(crate) fn focus_window(&self, window: &Window) {
        window.set_focus();
    }

    /// Focuses the search input inside the webview.
    pub(crate) fn focus_input(&self, webview: &WebView) -> Result<()> {
        webview
            .evaluate_script("window.__RUNX_FOCUS && window.__RUNX_FOCUS();")
            .context("failed to focus the launcher input")
    }

    /// Centers the launcher on the configured monitor.
    pub(crate) fn center_window(&self, window: &Window, config: &WindowConfig) {
        let Some(monitor) = monitor_for_window(window, config) else {
            return;
        };

        let scale = monitor.scale_factor();
        let monitor_size = monitor.size();
        let monitor_origin = monitor.position();
        let window_width = config.width * scale;
        let window_height = config.height * scale;
        let x = monitor_origin.x as f64 + (monitor_size.width as f64 - window_width) / 2.0;
        let y = monitor_origin.y as f64 + (monitor_size.height as f64 - window_height) / 3.2;

        window.set_outer_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }
}

fn monitor_for_window(
    window: &Window,
    config: &WindowConfig,
) -> Option<tao::monitor::MonitorHandle> {
    match config.show_on {
        WindowDisplayTarget::Primary => window.primary_monitor(),
        WindowDisplayTarget::Cursor => {
            let cursor_display = cursor_display_location();
            cursor_display
                .as_ref()
                .and_then(|cursor| {
                    window
                        .available_monitors()
                        .find(|monitor| monitor.native_id() == cursor.display_id)
                })
                .or_else(|| window.current_monitor())
                .or_else(|| window.primary_monitor())
        }
    }
}

fn looks_like_runx(app: &FrontmostApp) -> bool {
    app.bundle_id.as_deref() == Some(RUNX_BUNDLE_ID)
        || app.name.as_deref() == Some("Runx")
        || app
            .path
            .as_deref()
            .map(|path: &str| path.ends_with("/Runx.app"))
            == Some(true)
}
