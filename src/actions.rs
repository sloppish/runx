//! Thin adapter from selected result actions to concrete side effects.
//!
//! This module stays intentionally small: it pattern-matches on [`Action`] and
//! delegates the actual work to [`crate::macos`] or the plugin runtime.

use anyhow::Result;

use crate::config::WindowFocusBehavior;
use crate::macos;
use crate::plugins::{PluginExecutionContext, PluginHost};
use crate::types::Action;

/// Executes the action associated with the currently selected search result.
pub fn execute_action(
    action: &Action,
    window_focus_behavior: WindowFocusBehavior,
    all_windows: bool,
    plugins: &PluginHost,
    context: &PluginExecutionContext,
) -> Result<Option<String>> {
    match action {
        Action::Noop => Ok(None),
        Action::OpenApplication { path } => {
            macos::open_application(path)?;
            Ok(Some(format!("Opened {}", display_name(path))))
        }
        Action::OpenPath { path } => {
            macos::open_path(path)?;
            Ok(Some(format!("Opened {}", display_name(path))))
        }
        Action::OpenSettings { url, title } => {
            macos::open_settings(url)?;
            Ok(Some(format!("Opened {}", title)))
        }
        Action::FocusWindow {
            app_name,
            window_title,
            window_id,
        } => {
            if all_windows {
                macos::focus_window_and_activate_all_windows(app_name, window_title, *window_id)
            } else {
                macos::focus_window(app_name, window_title, *window_id, window_focus_behavior)
            }
        }
        Action::Plugin { plugin_id, payload } => plugins.run(plugin_id, payload, context),
    }
}

fn display_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_owned()
}
