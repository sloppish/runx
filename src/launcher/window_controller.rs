//! Native window visibility and focus helpers.
//!
//! This module keeps the macOS/Tao-facing window mechanics out of the main
//! launcher coordinator: focus tracking, centering, blur handling, and
//! remembering which app was frontmost before Runx appeared.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tao::platform::macos::MonitorHandleExtMacOS;
use tao::{dpi::LogicalSize, window::Window};
use wry::WebView;

use crate::{
    config::{WindowConfig, WindowDisplayTarget},
    displays::{DisplayProfile, profile_from_monitor},
    macos::{self, FrontmostApp, capture_frontmost_app, cursor_display_location},
    state::AppState,
};
use tracing::{debug, warn};

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
                debug!(
                    name = ?app.name,
                    bundle_id = ?app.bundle_id,
                    path = ?app.path,
                    "capture_previous_app accepted"
                );
                self.previous_app = Some(app);
            }
            Ok(Some(app)) => {
                debug!(
                    name = ?app.name,
                    bundle_id = ?app.bundle_id,
                    path = ?app.path,
                    "capture_previous_app ignored runx-like app"
                );
            }
            Ok(None) => {
                debug!("capture_previous_app found no frontmost app");
            }
            Err(error) => {
                warn!(error = %format!("{error:#}"), "capture_previous_app failed");
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

    /// Returns whether an inactivity event should auto-hide the launcher window.
    pub(crate) fn should_hide_when_inactive(
        &self,
        config: &WindowConfig,
        state: &AppState,
    ) -> bool {
        config.hide_when_inactive
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
        window.set_visible(true);
        macos::show_launcher_panel(window);
    }

    /// Hides the native window.
    pub(crate) fn hide_window(&self, window: &Window) {
        window.set_visible(false);
    }

    /// Focuses the launcher window without changing its visibility state.
    pub(crate) fn focus_window(&self, window: &Window) {
        macos::focus_launcher_panel(window);
    }

    /// Focuses the search input inside the webview.
    pub(crate) fn focus_webview(&self, webview: &WebView) -> Result<()> {
        webview
            .focus()
            .context("failed to focus the launcher webview")
    }

    /// Focuses the search input inside the webview.
    pub(crate) fn focus_input(&self, webview: &WebView) -> Result<()> {
        self.focus_webview(webview)?;
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
        let resolved_height = self.preferred_height.map_or_else(
            || config.fallback_size(ui).1,
            |height| config.clamp_height(height),
        );

        let Some(monitor) = monitor_for_window(window, config) else {
            let fallback_width = config.fallback_size(ui).0;
            window.set_inner_size(LogicalSize::new(fallback_width, resolved_height));
            return;
        };

        let display_id = monitor.native_id();
        if let Some(display_width) = macos::display_logical_size(display_id).map(|(width, _)| width)
        {
            let resolved_width = config.resolve_width(display_width);
            window.set_inner_size(LogicalSize::new(resolved_width, resolved_height));
            if !macos::position_launcher_panel(window, display_id, resolved_width, resolved_height)
            {
                warn!(display_id, "failed to position launcher on AppKit screen");
            }
            return;
        }

        let fallback_width = config.fallback_size(ui).0;
        window.set_inner_size(LogicalSize::new(fallback_width, resolved_height));
        warn!(
            display_id,
            "failed to resolve AppKit screen for launcher placement"
        );
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
