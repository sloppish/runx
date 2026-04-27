//! Native window visibility and focus helpers.
//!
//! This module keeps the macOS/Tao-facing window mechanics out of the main
//! launcher coordinator: focus tracking, centering, blur handling, and
//! remembering which app was frontmost before Runx appeared.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tao::platform::macos::MonitorHandleExtMacOS;
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    window::Window,
};
use wry::WebView;

use crate::{
    config::{WindowConfig, WindowDisplayTarget},
    debug_log,
    displays::{DisplayProfile, profile_from_monitor},
    macos::{self, FrontmostApp, capture_frontmost_app, cursor_display_location},
    state::AppState,
};

const INITIAL_BLUR_GUARD: Duration = Duration::from_millis(350);
const RUNX_BUNDLE_ID: &str = "io.github.sloppish.runx";

/// Tracks the native window lifecycle state that lives outside `AppState`.
#[derive(Default)]
pub(crate) struct WindowController {
    shown_at: Option<Instant>,
    previous_app: Option<FrontmostApp>,
    preferred_height: Option<f64>,
    layout_version: u64,
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

    /// Resolves the target display profile for the current launcher placement policy.
    pub(crate) fn target_display_profile(
        &self,
        window: &Window,
        config: &WindowConfig,
    ) -> Option<DisplayProfile> {
        monitor_for_window(window, config).map(|monitor| profile_from_monitor(&monitor))
    }

    /// Invalidates cached frontend measurement and returns the next accepted layout version.
    pub(crate) fn invalidate_preferred_height(&mut self) -> u64 {
        self.layout_version = self.layout_version.wrapping_add(1);
        self.preferred_height = None;
        self.layout_version
    }

    /// Accepts a frontend-reported preferred height when it matches the current layout version.
    pub(crate) fn accept_preferred_height(&mut self, layout_version: u64, height: f64) -> bool {
        if layout_version != self.layout_version || !height.is_finite() || height <= 0.0 {
            return false;
        }

        self.preferred_height = Some(height);
        true
    }

    /// Shows the native window.
    pub(crate) fn show_window(&self, window: &Window) {
        #[cfg(target_os = "macos")]
        {
            window.set_visible(true);
            macos::show_launcher_panel(window);
        }

        #[cfg(not(target_os = "macos"))]
        {
            window.set_visible(true);
            window.set_focus();
        }
    }

    /// Hides the native window.
    pub(crate) fn hide_window(&self, window: &Window) {
        window.set_visible(false);
    }

    /// Focuses the launcher window without changing its visibility state.
    pub(crate) fn focus_window(&self, window: &Window) {
        #[cfg(target_os = "macos")]
        {
            macos::focus_launcher_panel(window);
        }

        #[cfg(not(target_os = "macos"))]
        {
            window.set_focus();
        }
    }

    /// Focuses the search input inside the webview.
    pub(crate) fn focus_input(&self, webview: &WebView) -> Result<()> {
        webview
            .focus()
            .context("failed to focus the launcher webview")?;
        webview
            .evaluate_script("window.__RUNX_FOCUS && window.__RUNX_FOCUS();")
            .context("failed to focus the launcher input")
    }

    /// Resolves launcher size and centers the window on the configured monitor.
    pub(crate) fn apply_window_geometry(
        &self,
        window: &Window,
        config: &WindowConfig,
        ui: &crate::config::UiConfig,
    ) {
        let resolved_height = self
            .preferred_height
            .map(|height| config.clamp_height(height))
            .unwrap_or_else(|| config.fallback_size(ui).1);

        let Some(monitor) = monitor_for_window(window, config) else {
            window.set_inner_size(LogicalSize::new(config.min_width, resolved_height));
            return;
        };

        let scale = monitor.scale_factor();
        let monitor_size = monitor.size();
        let monitor_origin = monitor.position();
        let logical_monitor_width = monitor_size.width as f64 / scale;
        let resolved_width = config.resolve_width(logical_monitor_width);
        window.set_inner_size(LogicalSize::new(resolved_width, resolved_height));
        let window_width = resolved_width * scale;
        let window_height = resolved_height * scale;
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

#[cfg(test)]
mod tests {
    use super::WindowController;

    #[test]
    fn ignores_stale_preferred_height_reports() {
        let mut controller = WindowController::default();
        assert!(controller.accept_preferred_height(0, 512.0));
        assert_eq!(controller.preferred_height, Some(512.0));

        let next_version = controller.invalidate_preferred_height();
        assert_eq!(next_version, 1);
        assert_eq!(controller.preferred_height, None);

        assert!(!controller.accept_preferred_height(0, 640.0));
        assert_eq!(controller.preferred_height, None);

        assert!(controller.accept_preferred_height(1, 640.0));
        assert_eq!(controller.preferred_height, Some(640.0));
    }
}
